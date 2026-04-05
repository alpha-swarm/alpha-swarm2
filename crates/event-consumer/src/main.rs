/// Event consumer daemon: subscribes to NATS events and persists them to SurrealDB.
/// This is the single writer to SurrealDB — all other components publish to NATS only.
///
/// Usage: event-consumer
///   ALPHA_SWARM_NATS_URL=nats://127.0.0.1:4222
///   ALPHA_SWARM_SURREALDB_URL=127.0.0.1:8001
use anyhow::Result;
use tracing::{info, warn, error};

use knowledge_base::{AgentRun, KnowledgeStore, RunStatus};
use swarm_events::{EventSubscriber, SwarmEvent};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let nats_url = std::env::var("ALPHA_SWARM_NATS_URL")
        .unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let surreal_url = std::env::var("ALPHA_SWARM_SURREALDB_URL")
        .unwrap_or_else(|_| "127.0.0.1:8001".into());
    let surreal_ns = std::env::var("ALPHA_SWARM_SURREALDB_NS")
        .unwrap_or_else(|_| "alpha_swarm".into());
    let surreal_db = std::env::var("ALPHA_SWARM_SURREALDB_DB")
        .unwrap_or_else(|_| "swarm".into());

    info!("Connecting to NATS at {nats_url}");
    let subscriber = EventSubscriber::connect(&nats_url).await?;

    info!("Connecting to SurrealDB at {surreal_url}");
    let store = KnowledgeStore::connect(&surreal_url, &surreal_ns, &surreal_db).await?;

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
                match store.store_run(&run).await {
                    Ok(id) => info!(id, "Stored agent start"),
                    Err(e) => error!("Failed to store: {e}"),
                }
            }

            SwarmEvent::AgentFinished {
                project, agent_id, status, edits,
                tokens_input, tokens_output, duration_ms, model, ..
            } => {
                info!(project, agent_id, status, edits, "Agent finished");
                let mut run = AgentRun::new(project, "", agent_id, model);
                run.status = match status.as_str() {
                    "passed" => RunStatus::Passed,
                    "skipped" => RunStatus::Skipped,
                    _ => RunStatus::Failed,
                };
                run.tokens_input = *tokens_input;
                run.tokens_output = *tokens_output;
                run.duration_ms = *duration_ms;
                match store.store_run(&run).await {
                    Ok(id) => info!(id, "Stored agent finish"),
                    Err(e) => error!("Failed to store: {e}"),
                }
            }

            SwarmEvent::AgentFailed { project, agent_id, error, model, duration_ms, .. } => {
                warn!(project, agent_id, error, "Agent failed");
                let mut run = AgentRun::new(project, "", agent_id, model);
                run.status = RunStatus::Failed;
                run.error_message = Some(error.clone());
                run.duration_ms = *duration_ms;
                match store.store_run(&run).await {
                    Ok(id) => info!(id, "Stored agent failure"),
                    Err(e) => error!("Failed to store: {e}"),
                }
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
        }
    }

    Ok(())
}
