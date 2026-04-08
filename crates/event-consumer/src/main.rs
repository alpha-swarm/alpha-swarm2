/// Event consumer daemon: subscribes to NATS events and persists them to SurrealDB.
/// Single writer to SurrealDB — all other components publish to NATS only.
///
/// Configuration: alpha-swarm.toml or env vars.
use anyhow::Result;
use tracing::{info, warn, error};

use knowledge_base::{AgentRun, KnowledgeStore, RunStatus};
use swarm_config::SwarmConfig;
use swarm_events::{EventSubscriber, SwarmEvent};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let config = SwarmConfig::load();

    info!(nats = %config.nats.url, "Connecting to NATS");
    let subscriber = EventSubscriber::connect(&config.nats.url).await?;

    info!(surrealdb = %config.surrealdb.url, "Connecting to SurrealDB");
    let store = KnowledgeStore::connect(&config.surrealdb.url, &config.surrealdb.namespace, &config.surrealdb.database).await?;

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
                let _ = store.store_run(&run).await;
            }
            SwarmEvent::AgentFailed { project, agent_id, error, model, duration_ms, .. } => {
                warn!(project, agent_id, error, "Agent failed");
                let mut run = AgentRun::new(project, "", agent_id, model);
                run.status = RunStatus::Failed;
                run.error_message = Some(error.clone());
                run.duration_ms = *duration_ms;
                let _ = store.store_run(&run).await;
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
        }
    }
    Ok(())
}
