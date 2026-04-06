/// Agent daemon: watches SurrealDB for pending tasks and executes them.
///
/// Event-driven: polls pending on startup, then subscribes to NATS for
/// real-time task notifications. Each task runs in a tokio::spawn.
///
/// Usage: cargo run -p agent-daemon
mod repo;
mod executor;
mod git_pr;
pub mod resources;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn, error};

use inference_client::{ClaudeBackend, InferenceRouter, OllamaBackend};
use knowledge_base::{KnowledgeStore, RunStatus};
use swarm_config::SwarmConfig;
use swarm_events::{EventPublisher, EventSubscriber, SwarmEvent};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,agent_core=info,swarm_orchestrator=info")
        .init();

    let config = SwarmConfig::load();
    info!("Agent daemon starting");
    info!(ollama = %config.ollama.url, surrealdb = %config.surrealdb.url, nats = %config.nats.url);

    // Setup inference router
    let mut router = InferenceRouter::new();
    if !config.claude.api_key.is_empty() {
        info!(model = %config.claude.model, "Claude backend enabled");
        router = router.add_backend(ClaudeBackend::new(&config.claude.api_key).with_model(&config.claude.model));
    }
    info!(url = %config.ollama.url, "Ollama backend enabled");
    router = router.add_backend(OllamaBackend::new(&config.ollama.url));
    let router = Arc::new(router);

    let ollama = Arc::new(OllamaBackend::new(&config.ollama.url));

    // Connect to SurrealDB
    let store = Arc::new(
        KnowledgeStore::connect(&config.surrealdb.url, &config.surrealdb.namespace, &config.surrealdb.database)
            .await?
    );

    // Connect to NATS for events
    let publisher = match EventPublisher::connect(&config.nats.url).await {
        Ok(p) => { info!("NATS publisher connected"); Some(Arc::new(p)) }
        Err(e) => { warn!("NATS unavailable: {e}"); None }
    };

    let subscriber = match EventSubscriber::connect(&config.nats.url).await {
        Ok(s) => { info!("NATS subscriber connected"); Some(s) }
        Err(e) => { warn!("NATS unavailable for subscription: {e}"); None }
    };

    // 0. Start resource heartbeat (writes per-host snapshots to SurrealDB)
    {
        let store = Arc::clone(&store);
        let res_config = config.resources.clone();
        tokio::spawn(async move {
            loop {
                let snapshots = resources::check_all_hosts(&res_config).await;
                // Clear old snapshots and write new ones
                let _ = store.db_query_raw("DELETE FROM resource_snapshot").await;
                for snap in &snapshots {
                    let models_json = serde_json::to_string(&snap.ollama_models).unwrap_or_else(|_| "[]".into());
                    let query = format!(
                        "CREATE resource_snapshot SET host='{}', host_type='{}', cpu_percent={:.1}, ram_total_mb={}, ram_used_mb={}, ram_percent={:.1}, disk_total_gb={:.1}, disk_free_gb={:.1}, disk_percent={:.1}, ollama_models={}, timestamp=time::now()",
                        snap.host, snap.host_type,
                        snap.cpu_percent, snap.ram_total_mb, snap.ram_used_mb, snap.ram_percent,
                        snap.disk_total_gb, snap.disk_free_gb, snap.disk_percent,
                        models_json,
                    );
                    let _ = store.db_query_raw(&query).await;
                }
                tokio::time::sleep(Duration::from_secs(res_config.check_interval_secs)).await;
            }
        });
        info!(hosts = config.resources.hosts.len(), "Resource heartbeat started");
    }

    // 1. Reset zombie tasks (running but daemon died)
    info!("Recovering zombie tasks (status=running from previous daemon)...");
    let reset_result = store.db_query_raw(
        "UPDATE agent_run SET status = 'pending', agent_id = 'recovered' WHERE status = 'running'"
    ).await;
    match reset_result {
        Ok(_) => info!("Zombie tasks reset to pending"),
        Err(e) => warn!("Failed to reset zombies: {e}"),
    }

    // 1b. Periodic zombie recovery (every 5 minutes)
    {
        let store = Arc::clone(&store);
        const ZOMBIE_CHECK_INTERVAL_SECS: u64 = 300; // 5 minutes
        const ZOMBIE_STALE_MINUTES: u64 = 10;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(ZOMBIE_CHECK_INTERVAL_SECS)).await;
                // Reset tasks that have been "running" with no activity for ZOMBIE_STALE_MINUTES
                let query = format!(
                    "UPDATE agent_run SET status = 'failed', error_message = 'Zombie: no activity for {}m', agent_id = 'zombie-recovery' WHERE status = 'running' AND last_activity_at != NONE AND time::now() - <datetime>last_activity_at > {}m",
                    ZOMBIE_STALE_MINUTES, ZOMBIE_STALE_MINUTES
                );
                match store.db_query_raw(&query).await {
                    Ok(_) => {}
                    Err(e) => warn!("Zombie recovery check failed: {e}"),
                }
            }
        });
        info!("Periodic zombie recovery started (every {}s, stale after {}m)", ZOMBIE_CHECK_INTERVAL_SECS, ZOMBIE_STALE_MINUTES);
    }

    // 2. Process any pending tasks
    info!("Checking for pending tasks...");
    match store.list_pending().await {
        Ok(pending) => {
            info!(count = pending.len(), "Found pending tasks");
            for task in pending {
                // Check resources before scheduling
                if !resources::can_schedule(&config.resources) {
                    info!("Deferring remaining tasks until resources free up");
                    break;
                }

                let id = task.id.clone().unwrap_or_default();
                let project = task.project.clone();
                let goal = task.task_description.clone();
                info!(id = %id, project = %project, goal = %goal, "Processing pending task");

                let store = Arc::clone(&store);
                let router = Arc::clone(&router);
                let ollama = Arc::clone(&ollama);
                let publisher = publisher.clone();
                let config = config.clone();

                tokio::spawn(async move {
                    executor::handle_task(&config, router, ollama, store, publisher, &id, &project, &goal).await;
                });
            }
        }
        Err(e) => warn!("Failed to query pending tasks: {e}"),
    }

    // 2. Subscribe to NATS for real-time task events
    if let Some(sub) = subscriber {
        info!("Listening for task submissions via NATS...");
        let mut stream = sub.subscribe_all().await?;

        loop {
            match stream.next().await {
                Some(SwarmEvent::TaskSubmitted { project, task_id, goal, .. }) => {
                    info!(task_id = %task_id, project = %project, "Received task submission");

                    let store = Arc::clone(&store);
                    let router = Arc::clone(&router);
                    let ollama = Arc::clone(&ollama);
                    let publisher = publisher.clone();
                    let config = config.clone();

                    tokio::spawn(async move {
                        executor::handle_task(&config, router, ollama, store, publisher, &task_id, &project, &goal).await;
                    });
                }
                Some(_) => {} // ignore other events
                None => {
                    warn!("NATS stream closed, reconnecting in 5s...");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    break;
                }
            }
        }
    } else {
        // No NATS — fall back to polling SurrealDB
        info!("No NATS available, falling back to SurrealDB polling (every 5s)");
        loop {
            tokio::time::sleep(Duration::from_secs(config.resources.check_interval_secs)).await;
            if !resources::can_schedule(&config.resources) { continue; }
            if let Ok(pending) = store.list_pending().await {
                for task in pending {
                    if !resources::can_schedule(&config.resources) { break; }
                    let id = task.id.clone().unwrap_or_default();
                    let project = task.project.clone();
                    let goal = task.task_description.clone();

                    let store = Arc::clone(&store);
                    let router = Arc::clone(&router);
                    let ollama = Arc::clone(&ollama);
                    let publisher = publisher.clone();
                    let config = config.clone();

                    tokio::spawn(async move {
                        executor::handle_task(&config, router, ollama, store, publisher, &id, &project, &goal).await;
                    });
                }
            }
        }
    }

    Ok(())
}
