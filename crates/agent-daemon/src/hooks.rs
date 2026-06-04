//! Daemon-side built-in execution hooks: trajectory recording, SONA pattern
//! distillation, and pattern-effectiveness tracking.
//!
//! The `ExecutionHook` trait itself lives in `swarm_orchestrator::hooks` (the
//! runner fires task-level events); these implementations carry the heavier
//! knowledge-base dependencies.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use inference_client::{ChatMessage, Complexity, InferenceOptions, InferenceRouter};
use knowledge_base::{    KnowledgeBackend, MemoryEntry, MemoryStore, MEM_NS_ERRORS, MEM_NS_PATTERNS, MEM_NS_TRAJECTORIES,
};
use swarm_config::LearningConfig;
use swarm_orchestrator::hooks::{
    ExecutionHook, RunCompleteCtx, RunStartCtx, TaskCompleteCtx, TaskFailCtx,
};
use swarm_orchestrator::planner_types::DISTILL_SYSTEM;

/// Max chars of a distilled pattern stored to memory.
const MAX_PATTERN_CHARS: usize = 900;
/// Max chars of an error signature stored to memory.
const ERR_SIG_MAX_CHARS: usize = 600;
/// Max tokens requested from the distillation LLM call.
const DISTILL_MAX_TOKENS: u32 = 512;
/// Max chars of the verified diff fed to the distillation LLM (keeps the prompt
/// bounded on large changes).
const DISTILL_DIFF_MAX_CHARS: usize = 2000;

/// Stable key for a goal shape: sha256 of the lowercased, whitespace-collapsed goal.
fn goal_shape_key(goal: &str) -> String {
    let normalized = goal
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

struct TrajectoryState {
    goal: String,
    steps: Vec<serde_json::Value>,
}

/// Records run trajectories into memory and distills successful runs into
/// reusable patterns (the SONA learning loop). Distillation runs in a spawned
/// task with a per-project guard so it never blocks run completion and never
/// floods the single Ollama queue.
pub struct TrajectoryRecorder {
    memory: Arc<MemoryStore>,
    store: Arc<dyn KnowledgeBackend>,
    router: Arc<InferenceRouter>,
    distill_tier: swarm_config::TierConfig,
    learning: LearningConfig,
    runs: Mutex<HashMap<String, TrajectoryState>>,
    distilling_projects: Arc<Mutex<HashSet<String>>>,
}

impl TrajectoryRecorder {
    pub fn new(
        memory: Arc<MemoryStore>,
        store: Arc<dyn KnowledgeBackend>,
        router: Arc<InferenceRouter>,
        distill_tier: swarm_config::TierConfig,
        learning: LearningConfig,
    ) -> Self {
        Self {
            memory,
            store,
            router,
            distill_tier,
            learning,
            runs: Mutex::new(HashMap::new()),
            distilling_projects: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[async_trait]
impl ExecutionHook for TrajectoryRecorder {
    fn name(&self) -> &str {
        "trajectory-recorder"
    }

    async fn on_run_start(&self, ctx: &RunStartCtx<'_>) {
        self.runs.lock().await.insert(
            ctx.run_id.to_string(),
            TrajectoryState { goal: ctx.goal.to_string(), steps: Vec::new() },
        );
    }

    async fn on_task_complete(&self, ctx: &TaskCompleteCtx<'_>) {
        if let Some(state) = self.runs.lock().await.get_mut(ctx.run_id) {
            state.steps.push(serde_json::json!({
                "task_id": ctx.task_id,
                "description": ctx.description,
                "files": ctx.files,
                "outcome": if ctx.applied { "applied" } else { "noop" },
                "duration_ms": ctx.duration_ms,
                "model": ctx.model,
            }));
        }
    }

    async fn on_task_fail(&self, ctx: &TaskFailCtx<'_>) {
        if let Some(state) = self.runs.lock().await.get_mut(ctx.run_id) {
            state.steps.push(serde_json::json!({
                "task_id": ctx.task_id,
                "description": ctx.description,
                "outcome": "failed",
                "error": ctx.error,
            }));
        }
    }

    async fn on_run_complete(&self, ctx: &RunCompleteCtx<'_>) {
        let Some(state) = self.runs.lock().await.remove(ctx.run_id) else { return };
        let succeeded = ctx.quality_passed && ctx.tasks_passed > 0;
        let now = chrono::Utc::now().to_rfc3339();

        // 1. Trajectory entry (always, when learning is on).
        let trajectory_content = serde_json::json!({
            "goal": state.goal,
            "steps": state.steps,
            "outcome": if succeeded { "passed" } else { "failed" },
        }).to_string();
        let goal_embedding = self.memory.embed(&state.goal).await.unwrap_or_default();
        let entry = MemoryEntry {
            id: None,
            namespace: MEM_NS_TRAJECTORIES.into(),
            key: ctx.run_id.to_string(),
            content: trajectory_content,
            embedding: goal_embedding.clone(),
            metadata: serde_json::json!({
                "quality_passed": ctx.quality_passed,
                "tasks_passed": ctx.tasks_passed,
                "tasks_failed": ctx.tasks_failed,
                "retrieved_pattern_ids": ctx.retrieved_pattern_ids,
            }),
            project: ctx.project.to_string(),
            created_at: now.clone(),
            last_used_at: now.clone(),
            use_count: 0,
            ttl_secs: None,
        };
        if let Err(e) = self.memory.store(entry).await {
            warn!(run_id = ctx.run_id, error = %e, "trajectory store failed");
        }

        // 2. Pattern effectiveness: which injected patterns led to success?
        for pattern_id in ctx.retrieved_pattern_ids {
            let q = format!(
                "CREATE pattern_effectiveness SET pattern_id = '{}', run_id = '{}', project = '{}', run_succeeded = {}, created_at = time::now()",
                pattern_id.replace('\'', ""), ctx.run_id, ctx.project.replace('\'', ""), succeeded,
            );
            if let Err(e) = self.store.db_query_raw(&q).await {
                warn!(pattern_id, error = %e, "pattern_effectiveness write failed");
            }
        }

        // 3. Distillation (async, guarded, never blocks completion).
        if succeeded && self.learning.distill_on_success {
            let project = ctx.project.to_string();
            {
                let mut guard = self.distilling_projects.lock().await;
                if !guard.insert(project.clone()) {
                    info!(project, "Distillation already running for project — skipping");
                    return;
                }
            }
            let memory = Arc::clone(&self.memory);
            let router = Arc::clone(&self.router);
            let tier = self.distill_tier.clone();
            let guard_set = Arc::clone(&self.distilling_projects);
            let goal = state.goal.clone();
            let plan_summary = ctx.plan_summary.to_string();
            let diff: String = ctx.diff.chars().take(DISTILL_DIFF_MAX_CHARS).collect();
            tokio::spawn(async move {
                distill_pattern(&memory, &router, &tier, &project, &goal, &plan_summary, &diff, goal_embedding).await;
                guard_set.lock().await.remove(&project);
            });
        } else if !succeeded {
            // Failure → cheap error-signature entry, no LLM call.
            let signature: String = ctx.plan_summary.chars().take(ERR_SIG_MAX_CHARS).collect();
            let entry = MemoryEntry {
                id: None,
                namespace: MEM_NS_ERRORS.into(),
                // Key per-run (not per goal-shape): distinct failures of the same
                // goal must all be retained, else the UPSERT collapses history to
                // the latest one and the planner only ever sees one pitfall.
                key: ctx.run_id.to_string(),
                content: format!("GOAL: {}\nFAILED PLAN:\n{}", state.goal, signature),
                embedding: goal_embedding,
                metadata: serde_json::json!({ "run_id": ctx.run_id }),
                project: ctx.project.to_string(),
                created_at: now.clone(),
                last_used_at: now,
                use_count: 0,
                ttl_secs: None,
            };
            if let Err(e) = self.memory.store(entry).await {
                warn!(error = %e, "error-signature store failed");
            }
        }
    }
}

/// LLM-distill a successful run into a `patterns` entry keyed and embedded by
/// goal shape. Best-effort: failures only log.
#[allow(clippy::too_many_arguments)]
async fn distill_pattern(
    memory: &MemoryStore,
    router: &InferenceRouter,
    tier: &swarm_config::TierConfig,
    project: &str,
    goal: &str,
    plan_summary: &str,
    diff: &str,
    goal_embedding: Vec<f32>,
) {
    // Distill from the VERIFIED diff (the change the gate passed), with the plan
    // summary as secondary context — not the plan alone, which can list noop or
    // failed sub-tasks.
    let diff_block = if diff.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nVERIFIED DIFF:\n{diff}")
    };
    let user_msg = format!("GOAL: {goal}\n\nPLAN THAT WORKED:\n{plan_summary}{diff_block}\n\nOUTCOME: passed");
    let messages = vec![
        ChatMessage::system(DISTILL_SYSTEM),
        ChatMessage::user(user_msg),
    ];
    let options = InferenceOptions {
        max_tokens: Some(DISTILL_MAX_TOKENS),
        preferred_model: Some(tier.model.clone()),
        preferred_backend: Some(inference_client::BackendKind::Ollama),
        ..Default::default()
    };
    let pattern_text = match router.chat(&messages, Complexity::Simple, &options).await {
        Ok(resp) => resp.content.trim().chars().take(MAX_PATTERN_CHARS).collect::<String>(),
        Err(e) => {
            warn!(project, error = %e, "distillation LLM call failed");
            return;
        }
    };
    if pattern_text.is_empty() {
        return;
    }

    let now = chrono::Utc::now().to_rfc3339();
    let entry = MemoryEntry {
        id: None,
        namespace: MEM_NS_PATTERNS.into(),
        key: goal_shape_key(goal),
        content: pattern_text,
        embedding: goal_embedding,
        metadata: serde_json::json!({ "goal": goal }),
        project: project.to_string(),
        created_at: now.clone(),
        last_used_at: now,
        use_count: 0,
        ttl_secs: None,
    };
    match memory.store(entry).await {
        Ok(id) => info!(project, id, "SONA: pattern distilled"),
        Err(e) => warn!(project, error = %e, "pattern store failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_shape_key_normalizes() {
        let a = goal_shape_key("Fix  the   auth bug");
        let b = goal_shape_key("fix the auth bug");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
