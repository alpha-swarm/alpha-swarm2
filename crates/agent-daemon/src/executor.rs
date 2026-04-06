use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn, error};

use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::{AgentRun, AttemptRecord, KnowledgeStore, RunStatus};
use swarm_config::SwarmConfig;

/// Max chars for attempt preview fields
const ATTEMPT_PREVIEW_CHARS: usize = 500;
use swarm_events::{EventPublisher, SwarmEvent};

use crate::repo;
use crate::git_pr;

/// Handle a single task: claim → clone repo → execute → update status → emit events.
pub async fn handle_task(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<KnowledgeStore>,
    publisher: Option<Arc<EventPublisher>>,
    task_id: &str,
    project: &str,
    goal: &str,
) {
    info!(task_id, project, goal, "Starting task execution");

    // 1. Claim the task (set to running)
    let mut run = AgentRun::new(project, goal, "daemon", "pending");
    run.status = RunStatus::Running;
    if let Err(e) = store.update_run(task_id, &run).await {
        warn!(task_id, "Failed to claim task: {e}");
        return;
    }

    // Emit agent started event
    if let Some(pub_) = &publisher {
        let _ = pub_.publish(&SwarmEvent::AgentStarted {
            project: project.into(),
            agent_id: format!("daemon-{}", &task_id[..task_id.len().min(8)]),
            task: goal.into(),
            model: "auto".into(),
            files: vec![],
            timestamp: SwarmEvent::timestamp(),
        }).await;
    }

    // 2. Look up repo URL
    let repo_url = match store.get_project_repo(project).await {
        Ok(Some(url)) => url,
        Ok(None) => {
            fail_task(&store, &publisher, task_id, project, goal, "No repo URL configured for project").await;
            return;
        }
        Err(e) => {
            fail_task(&store, &publisher, task_id, project, goal, &format!("Failed to query project: {e}")).await;
            return;
        }
    };

    // 3. Clone/update repo
    let repo_path = match repo::ensure_repo(project, &repo_url) {
        Ok(p) => p,
        Err(e) => {
            fail_task(&store, &publisher, task_id, project, goal, &format!("Git clone failed: {e}")).await;
            return;
        }
    };

    info!(task_id, repo = %repo_path.display(), "Repo ready, executing swarm");

    // Helper: update progress on the running task
    async fn update_progress(store: &KnowledgeStore, task_id: &str, msg: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        let safe_msg = msg.replace('\'', "");
        let query = if task_id.contains(':') {
            format!("UPDATE {} SET last_activity_at = '{}', progress_message = '{}'", task_id, now, safe_msg)
        } else {
            format!("UPDATE type::thing('agent_run', '{}') SET last_activity_at = '{}', progress_message = '{}'", task_id, now, safe_msg)
        };
        let _ = store.db_query_raw(&query).await;
    }

    update_progress(&store, task_id, "Planning goal decomposition...").await;

    // 4. Run the swarm orchestrator with retry loop (orchestrator tier fuel)
    let tier = &config.tiers.orchestrator;
    let start = std::time::Instant::now();
    let max_iterations = tier.max_iterations;
    let time_limit_ms: u64 = tier.time_limit_secs * 1000;
    let token_limit: u32 = tier.token_limit;
    let max_backoff = tier.max_backoff_secs;
    info!(task_id, model = %tier.model, time_limit = tier.time_limit_secs, token_limit, max_iterations, "Using orchestrator tier");
    let mut total_tokens_used: u32 = 0;
    let mut iteration = 0;
    let mut last_errors = String::new();
    let mut final_result = None;

    loop {
        iteration += 1;
        let elapsed = start.elapsed().as_millis() as u64;

        if elapsed > time_limit_ms {
            warn!(task_id, iteration, "Time fuel exhausted ({elapsed}ms > {time_limit_ms}ms)");
            break;
        }
        if total_tokens_used > token_limit {
            warn!(task_id, iteration, total_tokens_used, "Token fuel exhausted");
            break;
        }
        if iteration > max_iterations {
            warn!(task_id, iteration, "Max iterations reached");
            break;
        }

        // Exponential backoff between retries
        if iteration > 1 {
            let backoff = std::cmp::min(2u64.pow(iteration as u32 - 1), max_backoff);
            info!(task_id, iteration, backoff_secs = backoff, errors = %last_errors.chars().take(100).collect::<String>(), "Retrying after backoff");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        }

        // Build goal with error context from previous iteration
        let augmented_goal = if last_errors.is_empty() {
            goal.to_string()
        } else {
            format!("{}\n\nPREVIOUS ATTEMPT FAILED:\n{}\n\nFix the issues from the previous attempt.", goal, last_errors)
        };

        let progress_msg = if iteration > 1 {
            format!("Retry {}/{} — replanning...", iteration, max_iterations)
        } else {
            "Running agents...".to_string()
        };
        update_progress(&store, task_id, &progress_msg).await;

        let mut runner = swarm_orchestrator::SwarmRunner::new(Arc::clone(&router), Arc::clone(&ollama), &repo_path, project);
        runner = runner
            .with_store(Arc::clone(&store))
            .with_parent_run_id(task_id)
            .with_max_concurrent(config.resources.max_concurrent_agents);

        match runner.run(&augmented_goal).await {
            Ok(result) => {
                // Track token usage
                let iter_tokens: u32 = result.results.iter()
                    .filter_map(|r| r.agent_result.as_ref())
                    .map(|a| a.inference_response.tokens_input + a.inference_response.tokens_output)
                    .sum();
                total_tokens_used += iter_tokens;

                if result.quality_passed {
                    let tasks_done = result.results.iter().filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied)).count();
                    update_progress(&store, task_id, &format!("Quality passed — {} tasks done, creating PR...", tasks_done)).await;
                    info!(task_id, iteration, total_tokens_used, "Quality gate passed!");
                    final_result = Some(result);
                    break;
                } else {
                    // Collect errors for next iteration
                    last_errors = result.results.iter()
                        .filter_map(|r| r.error.as_ref())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    info!(task_id, iteration, total_tokens_used, "Quality gate failed, will retry");
                    final_result = Some(result);
                    // Reset repo to clean state for next attempt
                    let _ = std::process::Command::new("git")
                        .args(["checkout", "."])
                        .current_dir(&repo_path)
                        .output();
                }
            }
            Err(e) => {
                last_errors = e.to_string();
                warn!(task_id, iteration, error = %e, "Swarm execution failed");
                final_result = None;
            }
        }
    }

    let duration = start.elapsed().as_millis() as u64;

    let start_time_rfc3339 = chrono::Utc::now().checked_sub_signed(chrono::Duration::milliseconds(duration as i64))
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    match final_result {
        Some(result) => {
            let tasks_passed = result.results.iter().filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied)).count();
            let tasks_failed = result.results.iter().filter(|r| r.error.is_some()).count();
            let any_work_done = tasks_passed > 0;

            // A run with zero successful sub-agents is always a failure
            let status = if !any_work_done {
                RunStatus::Failed
            } else if result.quality_passed {
                RunStatus::Passed
            } else {
                RunStatus::Failed
            };

            info!(
                task_id, project,
                quality_passed = result.quality_passed,
                any_work_done,
                tasks = result.tasks.len(),
                tasks_passed,
                tasks_failed,
                duration_ms = duration,
                "Swarm completed"
            );

            // Collect actual models used by sub-agents
            let models_used: Vec<String> = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .map(|a| a.inference_response.model.clone())
                .filter(|m| !m.is_empty())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let model_str = if models_used.is_empty() { "unknown".to_string() } else { models_used.join(", ") };

            // Aggregate token counts from sub-agents
            let (total_in, total_out) = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .fold((0u32, 0u32), |(i, o), a| (i + a.inference_response.tokens_input, o + a.inference_response.tokens_output));

            // Build run record with full tracking data
            let mut final_run = AgentRun::new(project, goal, "daemon", &model_str);
            final_run.status = status;
            final_run.duration_ms = duration;
            final_run.quality_gate_passed = Some(result.quality_passed && any_work_done);
            final_run.diff = result.merged_diff;
            final_run.tokens_input = total_in;
            final_run.tokens_output = total_out;
            final_run.started_at = Some(start_time_rfc3339);
            final_run.last_activity_at = Some(chrono::Utc::now().to_rfc3339());

            // Build attempts from sub-agent results (one per sub-task)
            final_run.attempts = result.results.iter().enumerate().map(|(i, r)| {
                AttemptRecord {
                    attempt: (i + 1) as u32,
                    model: r.agent_result.as_ref().map(|a| a.inference_response.model.clone()).unwrap_or_default(),
                    prompt_preview: r.task.description.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
                    response_preview: r.agent_result.as_ref()
                        .map(|a| a.inference_response.content.chars().take(ATTEMPT_PREVIEW_CHARS).collect())
                        .unwrap_or_default(),
                    tokens_input: r.agent_result.as_ref().map(|a| a.inference_response.tokens_input).unwrap_or(0),
                    tokens_output: r.agent_result.as_ref().map(|a| a.inference_response.tokens_output).unwrap_or(0),
                    duration_ms: r.agent_result.as_ref().map(|a| a.inference_response.duration_ms).unwrap_or(0),
                    quality_passed: r.agent_result.as_ref().map(|a| a.applied),
                    error: r.error.clone(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                }
            }).collect();

            // Aggregate response text from all sub-agents
            let responses: Vec<String> = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .map(|a| a.inference_response.content.clone())
                .collect();
            if !responses.is_empty() {
                final_run.response_text = Some(responses.join("\n---\n"));
            }

            // Collect files modified from sub-agents
            final_run.files_modified = result.results.iter()
                .flat_map(|r| r.task.files.iter().cloned())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            // Collect errors
            let agent_errors: Vec<String> = result.results.iter()
                .filter_map(|r| r.error.as_ref())
                .cloned()
                .collect();

            if !any_work_done {
                final_run.error_message = Some(if agent_errors.is_empty() {
                    "All agents completed without making changes".into()
                } else {
                    format!("All agents failed:\n{}", agent_errors.join("\n"))
                });
            } else if !result.quality_passed && !agent_errors.is_empty() {
                final_run.error_message = Some(agent_errors.join("\n"));
            }

            // 5. Create PR if there are actual changes
            if any_work_done {
                let sub_tasks_info: Vec<(String, String, String)> = result.results.iter()
                    .map(|r| {
                        let model = r.agent_result.as_ref().map(|a| a.inference_response.model.clone()).unwrap_or_default();
                        let st = if r.error.is_some() { "failed" } else if r.agent_result.as_ref().is_some_and(|a| a.applied) { "passed" } else { "skipped" };
                        (r.task.description.clone(), model, st.to_string())
                    })
                    .collect();

                match git_pr::create_pr(&repo_path, goal, &sub_tasks_info, result.quality_passed, duration, total_in, total_out) {
                    Ok(pr_url) => {
                        info!(pr_url = %pr_url, "PR created");
                        final_run.diff = Some(format!("PR: {}", pr_url));
                    }
                    Err(e) => {
                        warn!("PR creation failed (non-fatal): {e}");
                    }
                }
            }

            let _ = store.update_run(task_id, &final_run).await;

            // Emit completion event
            if let Some(pub_) = &publisher {
                let _ = pub_.publish(&SwarmEvent::SwarmCompleted {
                    project: project.into(),
                    goal: goal.into(),
                    quality_passed: result.quality_passed,
                    tasks_passed: tasks_passed as u32,
                    tasks_failed: tasks_failed as u32,
                    total_duration_ms: duration,
                    timestamp: SwarmEvent::timestamp(),
                }).await;
            }
        }
        None => {
            fail_task_with_duration(&store, &publisher, task_id, project, goal,
                &format!("All {} iterations failed. Last error: {}", iteration, last_errors), duration).await;
        }
    }
}

async fn fail_task(
    store: &KnowledgeStore,
    publisher: &Option<Arc<EventPublisher>>,
    task_id: &str, project: &str, goal: &str, error: &str,
) {
    fail_task_with_duration(&store, &publisher, task_id, project, goal, error, 0).await;
}

async fn fail_task_with_duration(
    store: &KnowledgeStore,
    publisher: &Option<Arc<EventPublisher>>,
    task_id: &str, project: &str, goal: &str, error_msg: &str, duration_ms: u64,
) {
    error!(task_id, project, error = error_msg, "Task failed");

    let mut run = AgentRun::new(project, goal, "daemon", "error");
    run.status = RunStatus::Failed;
    run.error_message = Some(error_msg.to_string());
    run.duration_ms = duration_ms;
    let _ = store.update_run(task_id, &run).await;

    if let Some(pub_) = &publisher {
        let _ = pub_.publish(&SwarmEvent::AgentFailed {
            project: project.into(),
            agent_id: format!("daemon-{}", &task_id[..task_id.len().min(8)]),
            error: error_msg.into(),
            model: String::new(),
            duration_ms,
            timestamp: SwarmEvent::timestamp(),
        }).await;
    }
}
