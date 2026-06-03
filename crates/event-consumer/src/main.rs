/// Event consumer daemon: subscribes to NATS events and persists them via the
/// agent-daemon's DB bridge. The daemon is the sole DB owner (embedded
/// surrealkv); this process never opens a SurrealDB connection.
///
/// Configuration: alpha-swarm.toml or env vars.
use anyhow::Result;
use tracing::{info, warn, error};

use knowledge_base::{AgentRun, RunStatus};
use knowledge_base::bridge_client::NatsDbClient;
use swarm_config::SwarmConfig;
use swarm_events::{EventSubscriber, SwarmEvent};

/// Store an agent run through the DB bridge.
async fn store_run(bridge: &NatsDbClient, run: &AgentRun) -> Result<()> {
    let mut json = serde_json::to_value(run)?;
    if let serde_json::Value::Object(ref mut map) = json {
        map.remove("id");
    }
    bridge.query("CREATE agent_run CONTENT $data", serde_json::json!({ "data": json })).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let config = SwarmConfig::load();

    info!(nats = %config.nats.url, "Connecting to NATS");
    let subscriber = EventSubscriber::connect(&config.nats.url).await?;

    info!(nats = %config.nats.url, "Connecting to DB bridge (swarm.db.>)");
    let store = NatsDbClient::connect(&config.nats.url).await?;

    info!("Subscribing to all alpha-swarm events...");
    let mut stream = subscriber.subscribe_all().await?;

    info!("Event consumer running. Press Ctrl+C to stop.");

    loop {
        let Some(event) = stream.next().await else {
            warn!("Event stream closed");
            break;
        };

        match &event {
            SwarmEvent::AgentStarted { project, agent_id, task, model, files, .. } => {
                info!(project, agent_id, task, "Agent started");
                let mut run = AgentRun::new(project, task, agent_id, model);
                run.status = RunStatus::Running;
                run.files_modified = files.clone();
                match store_run(&store, &run).await {
                    Ok(()) => info!("Stored agent start"),
                    Err(e) => error!("Failed to store: {e}"),
                }
            }
            SwarmEvent::AgentFinished { project, agent_id, status, tokens_input, tokens_output, duration_ms, model, .. } => {
                info!(project, agent_id, status, "Agent finished");
                let mut run = AgentRun::new(project, "", agent_id, model);
                run.status = match status.as_str() {
                    "passed" => RunStatus::Passed,
                    "skipped" => RunStatus::Skipped,
                    _ => RunStatus::Failed,
                };
                run.tokens_input = *tokens_input;
                run.tokens_output = *tokens_output;
                run.duration_ms = *duration_ms;
                let _ = store_run(&store, &run).await;
            }
            SwarmEvent::AgentFailed { project, agent_id, error, model, duration_ms, .. } => {
                warn!(project, agent_id, error, "Agent failed");
                let mut run = AgentRun::new(project, "", agent_id, model);
                run.status = RunStatus::Failed;
                run.error_message = Some(error.clone());
                run.duration_ms = *duration_ms;
                let _ = store_run(&store, &run).await;
            }
            SwarmEvent::SwarmPlanned { project, goal, task_count, .. } => {
                info!(project, goal, task_count, "Swarm planned");
            }
            SwarmEvent::SwarmCompleted { project, goal, quality_passed, .. } => {
                info!(project, goal, quality_passed, "Swarm completed");
            }
            SwarmEvent::QualityChecked { project, check_name, passed, .. } => {
                info!(project, check_name, passed, "Quality checked");
            }
            SwarmEvent::TaskSubmitted { project, task_id, goal, .. } => {
                info!(project, task_id, goal, "Task submitted");
            }
            SwarmEvent::WorkflowStateChanged { project, run_id, state, .. } => {
                info!(project, run_id, state, "Workflow state changed");
            }
            SwarmEvent::WorkflowStepDone { project, run_id, step_id, passed, .. } => {
                info!(project, run_id, step_id, passed, "Workflow step done");
            }
            SwarmEvent::WorkflowReplanned { project, run_id, failed_step_id, new_step_count, .. } => {
                info!(project, run_id, failed_step_id, new_step_count, "Workflow replanned");
            }
            // High-frequency progress events: observed live by the dashboard,
            // not persisted here.
            SwarmEvent::ToolCallExecuted { .. } | SwarmEvent::AgentProgress { .. } => {}
        }
    }
    Ok(())
}
