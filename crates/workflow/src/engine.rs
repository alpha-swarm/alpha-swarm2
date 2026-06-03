//! Workflow execution engine.
//!
//! Drives a persisted `WorkflowRun` through the existing `SwarmRunner` machinery
//! ("wrap, don't replace"): each round executes the remaining DAG via
//! `SwarmRunner::run_planned`, checkpoints the document after the round, and on
//! a step failure performs adaptive replanning (validated through the same
//! `parse_plan` as initial plans) instead of failing the whole run.
//!
//! Pause/cancel are cooperative via `RunControl` — checked between waves inside
//! the runner and between rounds here; in-flight agents are never preempted.
//!
//! Crash-recovery semantics: workspace merges happen only when a round returns,
//! so a daemon crash mid-round leaves the repo untouched. On resume, steps left
//! `Running` are reset to `Pending` and re-executed from a clean repo — safe by
//! construction (memtree workspaces merge only on success).

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use inference_client::InferenceRouter;
use swarm_events::{EventPublisher, SwarmEvent};
use swarm_orchestrator::{replan_goal, ControlState, RunControl, SwarmResult, SwarmRunner};

use crate::model::{
    Condition, StepState, WorkflowRun, WorkflowState, MAX_REPLAN_ATTEMPTS,
};
use crate::repo::WorkflowRepo;

/// Per-call execution context (everything the engine borrows from the daemon).
pub struct EngineContext<'a> {
    pub runner: &'a SwarmRunner,
    pub router: &'a InferenceRouter,
    pub planner_tier: &'a swarm_config::TierConfig,
    /// Known repo files — replan validation rejects paths outside this list.
    pub repo_files: Vec<String>,
    /// Repo root for precondition checks.
    pub repo_path: std::path::PathBuf,
}

/// Terminal outcome of one engine execution.
pub enum EngineOutcome {
    Completed(SwarmResult),
    Failed { result: Option<SwarmResult>, error: String },
    Paused,
    Cancelled,
}

/// Drives workflow runs. One engine per daemon; controls registry allows
/// pause/cancel requests from the NATS bridge while a run executes.
pub struct WorkflowEngine {
    repo: WorkflowRepo,
    publisher: Option<Arc<EventPublisher>>,
    controls: Mutex<HashMap<String, Arc<RunControl>>>,
}

impl WorkflowEngine {
    pub fn new(repo: WorkflowRepo, publisher: Option<Arc<EventPublisher>>) -> Self {
        Self {
            repo,
            publisher,
            controls: Mutex::new(HashMap::new()),
        }
    }

    pub fn repo(&self) -> &WorkflowRepo {
        &self.repo
    }

    /// Control handle for a run (created on demand). The bridge uses this to
    /// request pause/cancel; the runner observes it between waves.
    pub async fn control_for(&self, run_id: &str) -> Arc<RunControl> {
        let mut map = self.controls.lock().await;
        Arc::clone(map.entry(run_id.to_string()).or_insert_with(|| Arc::new(RunControl::new())))
    }

    async fn drop_control(&self, run_id: &str) {
        self.controls.lock().await.remove(run_id);
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    async fn publish_state(&self, wf: &WorkflowRun) {
        if let Some(p) = &self.publisher {
            let _ = p.publish(&SwarmEvent::WorkflowStateChanged {
                project: wf.project.clone(),
                workflow_id: wf.id.clone().unwrap_or_default(),
                run_id: wf.run_id.clone(),
                state: serde_json::to_value(wf.state)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default(),
                timestamp: SwarmEvent::timestamp(),
            }).await;
        }
    }

    async fn publish_step(&self, wf: &WorkflowRun, step_id: &str, passed: bool) {
        if let Some(p) = &self.publisher {
            let _ = p.publish(&SwarmEvent::WorkflowStepDone {
                project: wf.project.clone(),
                workflow_id: wf.id.clone().unwrap_or_default(),
                run_id: wf.run_id.clone(),
                step_id: step_id.to_string(),
                passed,
                timestamp: SwarmEvent::timestamp(),
            }).await;
        }
    }

    /// Check locally-verifiable preconditions for all pending steps; an unmet
    /// precondition fails the step (which routes into replanning).
    fn check_preconditions(wf: &mut WorkflowRun, repo_path: &std::path::Path) {
        for step in &mut wf.steps {
            if step.state != StepState::Pending || step.preconditions.is_empty() {
                continue;
            }
            for cond in &step.preconditions {
                let ok = match cond {
                    Condition::FileExists { path } => repo_path.join(path).exists(),
                    // FilesChanged is an effect-only condition pre-execution.
                    Condition::FilesChanged { .. } => true,
                };
                if !ok {
                    step.state = StepState::Failed;
                    step.error = Some(format!("Precondition unmet: {cond:?}"));
                    break;
                }
            }
        }
    }

    /// Verify effects of steps that just passed; an unmet effect demotes the
    /// step to Failed (routes into replanning).
    fn check_effects(wf: &mut WorkflowRun, round: &SwarmResult) {
        let changed: std::collections::HashSet<&str> =
            round.modified_files.iter().map(|(p, _)| p.as_str()).collect();
        for step in &mut wf.steps {
            if step.state != StepState::Passed || step.effects.is_empty() {
                continue;
            }
            for eff in &step.effects {
                let ok = match eff {
                    Condition::FileExists { path } => changed.contains(path.as_str()),
                    Condition::FilesChanged { paths } => {
                        paths.iter().any(|p| changed.contains(p.as_str()))
                    }
                };
                if !ok {
                    step.state = StepState::Failed;
                    step.error = Some(format!("Effect unmet: {eff:?}"));
                    break;
                }
            }
        }
    }

    /// Fold one round's task results back into step states.
    fn absorb_round(wf: &mut WorkflowRun, round: &SwarmResult) {
        for r in &round.results {
            let Some(step) = wf.steps.iter_mut().find(|s| s.id == r.task.id) else { continue };
            step.attempts += 1;
            match (&r.agent_result, &r.error) {
                (Some(ar), _) if ar.applied => {
                    step.state = StepState::Passed;
                    step.error = None;
                    step.agent_run_id = ar.run_id.clone();
                }
                (Some(_), None) => {
                    // Agent finished without applying changes — not a failure.
                    step.state = StepState::Skipped;
                }
                (_, Some(err)) => {
                    step.state = StepState::Failed;
                    step.error = Some(err.clone());
                }
                _ => {}
            }
        }
    }

    /// Merge a round's results into the accumulated aggregate.
    fn merge_results(total: &mut Option<SwarmResult>, round: SwarmResult) {
        match total {
            None => *total = Some(round),
            Some(acc) => {
                acc.results.extend(round.results);
                acc.tasks.extend(round.tasks);
                acc.modified_files.extend(round.modified_files);
                if let Some(d) = round.merged_diff {
                    match &mut acc.merged_diff {
                        Some(existing) => {
                            existing.push('\n');
                            existing.push_str(&d);
                        }
                        None => acc.merged_diff = Some(d),
                    }
                }
                acc.quality_passed = round.quality_passed;
                acc.total_duration_ms += round.total_duration_ms;
                acc.halted = round.halted;
                acc.retrieved_pattern_ids.extend(round.retrieved_pattern_ids);
            }
        }
    }

    /// Execute (or resume) a workflow run to a terminal/paused state.
    ///
    /// Pre-existing `Passed` steps are never re-run within this engine session;
    /// steps found `Running` (crash residue) are reset to `Pending` first.
    /// On resume, earlier rounds' file outputs are reproduced from the
    /// checkpoint; an incomplete checkpoint resets Passed steps instead
    /// (correctness over speed — never trust partial outputs).
    pub async fn execute(&self, wf: &mut WorkflowRun, ctx: &EngineContext<'_>) -> Result<EngineOutcome> {
        // Reset crash residue.
        for s in &mut wf.steps {
            if s.state == StepState::Running {
                s.state = StepState::Pending;
            }
        }

        // Resume seeding: reproduce checkpointed outputs of already-Passed
        // steps, or re-run them when the checkpoint can't be trusted.
        let has_passed = wf.steps.iter().any(|s| s.state == StepState::Passed);
        let mut resume_seed: Option<SwarmResult> = None;
        if has_passed {
            if wf.checkpoint_complete {
                info!(run_id = %wf.run_id, files = wf.captured_files.len(), "Resume: seeding outputs from checkpoint");
                resume_seed = Some(SwarmResult {
                    goal: wf.goal.clone(),
                    tasks: vec![],
                    results: vec![],
                    merged_diff: None,
                    quality_passed: true,
                    total_duration_ms: 0,
                    modified_files: wf.captured_files.iter()
                        .map(|f| (f.path.clone(), f.content.clone().into_bytes()))
                        .collect(),
                    phase_timings: Default::default(),
                    halted: false,
                    retrieved_pattern_ids: vec![],
                });
            } else {
                let reset = wf.reset_passed_steps();
                warn!(run_id = %wf.run_id, reset, "Resume: checkpoint incomplete — re-running Passed steps");
            }
        }

        if wf.state == WorkflowState::Created || wf.state == WorkflowState::Paused {
            wf.transition(WorkflowState::Running, Self::now());
        }
        self.repo.update_run(wf).await?;
        self.publish_state(wf).await;

        let control = self.control_for(&wf.run_id).await;
        control.resume();
        let mut aggregate: Option<SwarmResult> = resume_seed;

        let outcome = loop {
            // Cooperative pause/cancel between rounds.
            match control.state() {
                ControlState::Pause => {
                    wf.transition(WorkflowState::Paused, Self::now());
                    self.repo.update_run(wf).await?;
                    self.publish_state(wf).await;
                    break EngineOutcome::Paused;
                }
                ControlState::Cancel => {
                    wf.transition(WorkflowState::Cancelled, Self::now());
                    self.repo.update_run(wf).await?;
                    self.publish_state(wf).await;
                    break EngineOutcome::Cancelled;
                }
                ControlState::Continue => {}
            }

            Self::check_preconditions(wf, &ctx.repo_path);

            // Failed step (from precondition or a previous round) → replan or fail.
            if let Some(failed) = wf.failed_step().cloned() {
                if wf.replan_attempts >= MAX_REPLAN_ATTEMPTS {
                    let err = format!(
                        "Step '{}' failed and replan budget ({MAX_REPLAN_ATTEMPTS}) exhausted: {}",
                        failed.id,
                        failed.error.as_deref().unwrap_or("unknown error"),
                    );
                    wf.transition(WorkflowState::Failed, Self::now());
                    self.repo.update_run(wf).await?;
                    self.publish_state(wf).await;
                    break EngineOutcome::Failed { result: aggregate.take(), error: err };
                }

                let completed_summary: String = wf.steps.iter()
                    .filter(|s| s.state == StepState::Passed)
                    .map(|s| format!("- {}: {}", s.id, s.task.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                let files_modified: Vec<String> = aggregate.as_ref()
                    .map(|a| a.modified_files.iter().map(|(p, _)| p.clone()).collect())
                    .unwrap_or_default();

                info!(run_id = %wf.run_id, failed_step = %failed.id, attempt = wf.replan_attempts + 1, "Adaptive replanning");
                match replan_goal(
                    ctx.router,
                    &wf.goal,
                    &completed_summary,
                    &failed.task.description,
                    failed.error.as_deref().unwrap_or(""),
                    &files_modified,
                    &ctx.repo_files,
                    ctx.planner_tier,
                ).await {
                    Ok(new_tasks) if !new_tasks.is_empty() => {
                        let new_count = new_tasks.len() as u32;
                        wf.splice_replan(&failed.id, new_tasks, Self::now());
                        self.repo.update_run(wf).await?;
                        if let Some(p) = &self.publisher {
                            let _ = p.publish(&SwarmEvent::WorkflowReplanned {
                                project: wf.project.clone(),
                                workflow_id: wf.id.clone().unwrap_or_default(),
                                run_id: wf.run_id.clone(),
                                failed_step_id: failed.id.clone(),
                                new_step_count: new_count,
                                replan_attempt: wf.replan_attempts,
                                timestamp: SwarmEvent::timestamp(),
                            }).await;
                        }
                        continue;
                    }
                    Ok(_) => {
                        // Empty replan = model says nothing left to do, but a
                        // step failed — treat as unrecoverable.
                        let err = format!("Replan returned no tasks after step '{}' failed", failed.id);
                        wf.transition(WorkflowState::Failed, Self::now());
                        self.repo.update_run(wf).await?;
                        self.publish_state(wf).await;
                        break EngineOutcome::Failed { result: aggregate.take(), error: err };
                    }
                    Err(e) => {
                        // Hard rule: invalid replan output is never executed.
                        warn!(run_id = %wf.run_id, error = %e, "Replan aborted (validation/inference failure)");
                        let err = format!("Replan aborted: {e}");
                        wf.transition(WorkflowState::Failed, Self::now());
                        self.repo.update_run(wf).await?;
                        self.publish_state(wf).await;
                        break EngineOutcome::Failed { result: aggregate.take(), error: err };
                    }
                }
            }

            // Nothing pending and nothing failed → done.
            let remaining = wf.pending_tasks();
            if remaining.is_empty() {
                wf.transition(WorkflowState::Completed, Self::now());
                self.repo.update_run(wf).await?;
                self.publish_state(wf).await;
                let result = aggregate.take().unwrap_or_else(|| SwarmResult {
                    goal: wf.goal.clone(),
                    tasks: vec![],
                    results: vec![],
                    merged_diff: None,
                    quality_passed: true,
                    total_duration_ms: 0,
                    modified_files: vec![],
                    phase_timings: Default::default(),
                    halted: false,
                    retrieved_pattern_ids: vec![],
                });
                break EngineOutcome::Completed(result);
            }

            // Execute one round of the remaining DAG.
            info!(run_id = %wf.run_id, remaining = remaining.len(), "Workflow round dispatch");
            let round = ctx.runner.run_planned(&wf.goal, remaining).await?;
            Self::absorb_round(wf, &round);
            Self::check_effects(wf, &round);
            // Durable output checkpoint: a crash after this update cannot
            // lose this round's edits.
            wf.absorb_captured(&round.modified_files);

            // Checkpoint + per-step events.
            self.repo.update_run(wf).await?;
            for r in &round.results {
                let passed = r.error.is_none();
                self.publish_step(wf, &r.task.id, passed).await;
            }
            let halted = round.halted;
            Self::merge_results(&mut aggregate, round);
            if halted {
                // Control fired mid-round; loop head handles the transition.
                continue;
            }
        };

        if !matches!(outcome, EngineOutcome::Paused) {
            self.drop_control(&wf.run_id).await;
        }
        Ok(outcome)
    }
}
