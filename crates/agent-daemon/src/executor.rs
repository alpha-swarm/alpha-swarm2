use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn, error};

use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::{AgentRun, KnowledgeStore, RunStatus};
use swarm_config::SwarmConfig;
use swarm_events::{EventPublisher, SwarmEvent};

use crate::repo;

/// Handle a single task: claim → clone repo → execute → update status → emit events.
pub async fn handle_task(
    config: &SwarmConfig,
    router: &InferenceRouter,
    ollama: &OllamaBackend,
    store: &KnowledgeStore,
    publisher: Option<&EventPublisher>,
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
    if let Some(pub_) = publisher {
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
            fail_task(store, publisher, task_id, project, goal, "No repo URL configured for project").await;
            return;
        }
        Err(e) => {
            fail_task(store, publisher, task_id, project, goal, &format!("Failed to query project: {e}")).await;
            return;
        }
    };

    // 3. Clone/update repo
    let repo_path = match repo::ensure_repo(project, &repo_url) {
        Ok(p) => p,
        Err(e) => {
            fail_task(store, publisher, task_id, project, goal, &format!("Git clone failed: {e}")).await;
            return;
        }
    };

    info!(task_id, repo = %repo_path.display(), "Repo ready, executing swarm");

    // 4. Run the swarm orchestrator
    let start = std::time::Instant::now();

    let mut runner = swarm_orchestrator::SwarmRunner::new(router, ollama, &repo_path, project);
    runner = runner.with_store(store);

    match runner.run(goal).await {
        Ok(result) => {
            let duration = start.elapsed().as_millis() as u64;
            let status = if result.quality_passed { RunStatus::Passed } else { RunStatus::Failed };
            let tasks_passed = result.results.iter().filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied)).count();
            let tasks_failed = result.results.iter().filter(|r| r.error.is_some()).count();

            info!(
                task_id, project,
                quality_passed = result.quality_passed,
                tasks = result.tasks.len(),
                tasks_passed,
                tasks_failed,
                duration_ms = duration,
                "Swarm completed"
            );

            // Update run record
            let mut final_run = AgentRun::new(project, goal, "daemon", "swarm");
            final_run.status = status;
            final_run.duration_ms = duration;
            final_run.quality_gate_passed = Some(result.quality_passed);
            final_run.diff = result.merged_diff;

            // Aggregate token counts from sub-agents
            let (total_in, total_out) = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .fold((0u32, 0u32), |(i, o), a| (i + a.inference_response.tokens_input, o + a.inference_response.tokens_output));
            final_run.tokens_input = total_in;
            final_run.tokens_output = total_out;

            if !result.quality_passed {
                let errors: Vec<String> = result.results.iter()
                    .filter_map(|r| r.error.as_ref())
                    .cloned()
                    .collect();
                if !errors.is_empty() {
                    final_run.error_message = Some(errors.join("\n"));
                }
            }

            let _ = store.update_run(task_id, &final_run).await;

            // Emit completion event
            if let Some(pub_) = publisher {
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
        Err(e) => {
            let duration = start.elapsed().as_millis() as u64;
            fail_task_with_duration(store, publisher, task_id, project, goal, &format!("Swarm failed: {e}"), duration).await;
        }
    }
}

async fn fail_task(
    store: &KnowledgeStore,
    publisher: Option<&EventPublisher>,
    task_id: &str, project: &str, goal: &str, error: &str,
) {
    fail_task_with_duration(store, publisher, task_id, project, goal, error, 0).await;
}

async fn fail_task_with_duration(
    store: &KnowledgeStore,
    publisher: Option<&EventPublisher>,
    task_id: &str, project: &str, goal: &str, error_msg: &str, duration_ms: u64,
) {
    error!(task_id, project, error = error_msg, "Task failed");

    let mut run = AgentRun::new(project, goal, "daemon", "error");
    run.status = RunStatus::Failed;
    run.error_message = Some(error_msg.to_string());
    run.duration_ms = duration_ms;
    let _ = store.update_run(task_id, &run).await;

    if let Some(pub_) = publisher {
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
