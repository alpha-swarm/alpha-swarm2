/// Agent daemon: distributed task executor coordinated via NATS KV.
///
/// Primary mode: NATS KV scheduler (watch for tasks, claim via leases, heartbeat).
/// Fallback mode: SurrealDB polling (if NATS unavailable).
///
/// Usage: cargo run -p agent-daemon
mod repo;
mod executor;
mod git_pr;
pub mod resources;
pub mod provider_client;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn};

use inference_client::{ClaudeBackend, InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeStore;
use swarm_config::SwarmConfig;
use swarm_events::{EventPublisher, NatsScheduler, scheduler::HostResources};

/// How often to poll SurrealDB in fallback mode.
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// How often to publish resource snapshots.
const RESOURCE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How often to renew leases for running tasks.
const LEASE_RENEWAL_INTERVAL: Duration = Duration::from_secs(120);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,agent_core=info,swarm_orchestrator=info")
        .init();

    let config = SwarmConfig::load();
    let daemon_id = format!("daemon-{}-{}", hostname(), &uuid::Uuid::new_v4().to_string()[..8]);
    info!(daemon_id = %daemon_id, "Agent daemon starting");
    info!(ollama = %config.ollama.url, surrealdb = %config.surrealdb.url, nats = %config.nats.url);

    // Setup inference router
    let mut router = InferenceRouter::new();
    if !config.claude.api_key.is_empty() {
        info!(model = %config.claude.model, "Claude backend enabled");
        router = router.add_backend(ClaudeBackend::new(&config.claude.api_key).with_model(&config.claude.model));
    }

    // Register Ollama backends from providers config (if any)
    if !config.providers.is_empty() {
        let mut sorted = config.providers.clone();
        sorted.sort_by_key(|p| p.priority);
        for provider in &sorted {
            if provider.provider_type == "ollama" {
                info!(url = %provider.url, priority = provider.priority, "Adding Ollama provider");
                router = router.add_backend(OllamaBackend::new(&provider.url));
            }
        }
    } else {
        // Fallback: use single [ollama] config
        router = router.add_backend(OllamaBackend::new(&config.ollama.url));
    }
    let router = Arc::new(router);
    let ollama = Arc::new(OllamaBackend::new(&config.ollama.url));

    // Warm model into Ollama memory (background, non-blocking)
    {
        let model = config.tiers.agent.model.clone();
        let ollama_warm = Arc::clone(&ollama);
        tokio::spawn(async move {
            info!(model = %model, "Warming model into Ollama memory...");
            match ollama_warm.embed(&model, "warmup").await {
                Ok(_) => info!(model = %model, "Model warmed"),
                Err(e) => warn!(model = %model, error = %e, "Model warmup failed (will load on first use)"),
            }
        });
    }

    // Connect to SurrealDB (always needed for run storage)
    let store = Arc::new(
        KnowledgeStore::connect(&config.surrealdb.url, &config.surrealdb.namespace, &config.surrealdb.database).await?
    );

    // Connect to NATS for events
    let publisher = match EventPublisher::connect(&config.nats.url).await {
        Ok(p) => { info!("NATS publisher connected"); Some(Arc::new(p)) }
        Err(e) => { warn!("NATS publisher unavailable: {e}"); None }
    };

    // Try NATS KV scheduler (primary mode)
    let scheduler = match NatsScheduler::connect(&config.nats.url, &daemon_id).await {
        Ok(s) => { info!("NATS KV scheduler connected"); Some(Arc::new(s)) }
        Err(e) => { warn!("NATS KV unavailable, will use SurrealDB polling: {e}"); None }
    };

    // Reset zombie tasks from previous daemon instance
    info!("Recovering zombie tasks...");
    let _ = store.db_query_raw(
        "UPDATE agent_run SET status = 'pending', agent_id = 'recovered' WHERE status = 'running'"
    ).await;

    // Start resource heartbeat
    {
        let store = Arc::clone(&store);
        let scheduler = scheduler.clone();
        let res_config = config.resources.clone();
        let daemon_id = daemon_id.clone();
        tokio::spawn(async move {
            loop {
                let snapshots = resources::check_all_hosts(&res_config).await;

                // Write to SurrealDB (for web-ui dashboard)
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

                // Also publish to NATS KV (for distributed scheduling)
                if let Some(sched) = &scheduler {
                    let local = snapshots.iter().find(|s| s.host_type == "local");
                    if let Some(snap) = local {
                        let _ = sched.publish_resources(&HostResources {
                            daemon_id: daemon_id.clone(),
                            host: snap.host.clone(),
                            cpu_percent: snap.cpu_percent,
                            ram_percent: snap.ram_percent,
                            disk_percent: snap.disk_percent,
                            available_models: snap.ollama_models.iter().map(|m| m.name.clone()).collect(),
                            updated_at: chrono::Utc::now().to_rfc3339(),
                        }).await;
                    }
                }

                tokio::time::sleep(RESOURCE_HEARTBEAT_INTERVAL).await;
            }
        });
    }

    // Process initial pending tasks (first one only)
    process_pending(&config, &router, &ollama, &store, &publisher, &scheduler).await;

    // Main loop: poll SurrealDB, run 1 goal at a time via NATS KV execution lock
    info!("Polling SurrealDB every {:?} (1 goal at a time via NATS KV lock)", FALLBACK_POLL_INTERVAL);
    loop {
        tokio::time::sleep(FALLBACK_POLL_INTERVAL).await;
        if !resources::can_schedule(&config.resources) { continue; }

        let lock_free = match &scheduler {
            Some(sched) => sched.is_execution_lock_free().await,
            None => true,
        };

        if lock_free {
            // Execute first approved/pending task
            if let Ok(pending) = store.list_pending().await {
                if let Some(task) = pending.into_iter().next() {
                    let id = task.id.clone().unwrap_or_default();
                    if let Some(sched) = &scheduler {
                        match sched.try_acquire_execution_lock(&id).await {
                            Ok(true) => { info!(id = %id, "Acquired execution lock"); }
                            Ok(false) => { continue; }
                            Err(_) => { continue; }
                        }
                        let _ = sched.try_claim(&id).await;
                    }
                    let project = task.project.clone();
                    let goal = task.task_description.clone();
                    let status = serde_json::to_string(&task.status).unwrap_or_default().trim_matches('"').to_string();
                    spawn_task(config.clone(), Arc::clone(&router), Arc::clone(&ollama), Arc::clone(&store), publisher.clone(), scheduler.clone(), id, project, goal, status);
                }
            }
        } else {
            // Execution lock held — pre-plan queued tasks instead of waiting
            if let Ok(all) = store.list_pending().await {
                for task in all {
                    let status = serde_json::to_string(&task.status).unwrap_or_default().trim_matches('"').to_string();
                    if status != "planning" { continue; }
                    let id = task.id.clone().unwrap_or_default();
                    info!(id = %id, "Pre-planning queued task while execution lock is held");
                    let project = task.project.clone();
                    let goal = task.task_description.clone();
                    // Run planning only (not execution)
                    executor::handle_task(&config, Arc::clone(&router), Arc::clone(&ollama), Arc::clone(&store), publisher.clone(), &id, &project, &goal, "planning").await;
                    break; // One at a time
                }
            }
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Process any tasks already pending in SurrealDB (first one only).
async fn process_pending(
    config: &SwarmConfig,
    router: &Arc<InferenceRouter>,
    ollama: &Arc<OllamaBackend>,
    store: &Arc<KnowledgeStore>,
    publisher: &Option<Arc<EventPublisher>>,
    scheduler: &Option<Arc<NatsScheduler>>,
) {
    match store.list_pending().await {
        Ok(pending) => {
            info!(count = pending.len(), "Found pending tasks");
            if let Some(task) = pending.into_iter().next() {
                if !resources::can_schedule(&config.resources) {
                    info!("Deferring tasks until resources free up");
                    return;
                }

                let id = task.id.clone().unwrap_or_default();

                if let Some(sched) = scheduler {
                    match sched.try_acquire_execution_lock(&id).await {
                        Ok(true) => { info!(id = %id, "Acquired execution lock"); }
                        Ok(false) => { info!(id = %id, "Execution lock held, deferring"); return; }
                        Err(e) => { warn!(id = %id, error = %e, "Execution lock error"); return; }
                    }
                    let _ = sched.try_claim(&id).await;
                }

                let project = task.project.clone();
                let goal = task.task_description.clone();
                let status = serde_json::to_string(&task.status).unwrap_or_default().trim_matches('"').to_string();

                spawn_task(config.clone(), Arc::clone(router), Arc::clone(ollama), Arc::clone(store), publisher.clone(), scheduler.clone(), id, project, goal, status);
            }
        }
        Err(e) => warn!("Failed to query pending tasks: {e}"),
    }
}

/// Primary mode: watch NATS KV for new tasks.
#[allow(dead_code)]
async fn run_nats_kv_loop(
    config: &SwarmConfig,
    router: &Arc<InferenceRouter>,
    ollama: &Arc<OllamaBackend>,
    store: &Arc<KnowledgeStore>,
    publisher: &Option<Arc<EventPublisher>>,
    scheduler: &Arc<NatsScheduler>,
) {

    let watcher = match scheduler.watch_tasks().await {
        Ok(w) => w,
        Err(e) => {
            warn!("Failed to watch NATS KV tasks, falling back to polling: {e}");
            run_surreal_poll_loop(config, router, ollama, store, publisher).await;
            return;
        }
    };

    info!("Watching NATS KV for new tasks...");

    tokio::pin!(watcher);

    // Also poll SurrealDB periodically for tasks submitted directly (web-ui → SurrealDB)
    let mut poll_interval = tokio::time::interval(FALLBACK_POLL_INTERVAL);

    loop {
        tokio::select! {
            biased;  // Poll SurrealDB first — web-ui tasks go there, not NATS KV

            _ = poll_interval.tick() => {
                // Check SurrealDB for tasks submitted via web-ui
                if let Ok(pending) = store.list_pending().await {
                    if !pending.is_empty() {
                        info!(count = pending.len(), "SurrealDB poll found pending tasks");
                    }
                    for task in pending {
                        if !resources::can_schedule(&config.resources) { break; }
                        let id = task.id.clone().unwrap_or_default();

                        // Try NATS KV claim, proceed even if it fails
                        match scheduler.try_claim(&id).await {
                            Ok(true) => { info!(id = %id, "Claimed via NATS KV (poll)"); }
                            Ok(false) => { info!(id = %id, "NATS KV claim failed (poll), proceeding anyway"); }
                            Err(e) => { warn!(id = %id, error = %e, "NATS KV claim error (poll)"); }
                        }

                        let project = task.project.clone();
                        let goal = task.task_description.clone();
                        let status = serde_json::to_string(&task.status).unwrap_or_default().trim_matches('"').to_string();

                        spawn_task(config.clone(), Arc::clone(router), Arc::clone(ollama), Arc::clone(store), publisher.clone(), Some(Arc::clone(scheduler)), id, project, goal, status);
                    }
                }
            }
        }
    }
}

/// Fallback mode: poll SurrealDB when NATS is unavailable.
#[allow(dead_code)]
async fn run_surreal_poll_loop(
    config: &SwarmConfig,
    router: &Arc<InferenceRouter>,
    ollama: &Arc<OllamaBackend>,
    store: &Arc<KnowledgeStore>,
    publisher: &Option<Arc<EventPublisher>>,
) {
    loop {
        tokio::time::sleep(FALLBACK_POLL_INTERVAL).await;
        if !resources::can_schedule(&config.resources) { continue; }

        if let Ok(pending) = store.list_pending().await {
            for task in pending {
                if !resources::can_schedule(&config.resources) { break; }
                let id = task.id.clone().unwrap_or_default();
                let project = task.project.clone();
                let goal = task.task_description.clone();
                let status = serde_json::to_string(&task.status).unwrap_or_default().trim_matches('"').to_string();

                spawn_task(config.clone(), Arc::clone(router), Arc::clone(ollama), Arc::clone(store), publisher.clone(), None, id, project, goal, status);
            }
        }
    }
}

/// Spawn a task with lease management and NATS KV execution lock.
#[allow(clippy::too_many_arguments)]
fn spawn_task(
    config: SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<KnowledgeStore>,
    publisher: Option<Arc<EventPublisher>>,
    scheduler: Option<Arc<NatsScheduler>>,
    id: String,
    project: String,
    goal: String,
    status: String,
) {
    tokio::spawn(async move {
        info!(id = %id, "Goal starting execution");

        // Start lease + execution lock renewal heartbeat
        let lease_handle = if let Some(sched) = &scheduler {
            let sched = Arc::clone(sched);
            let id = id.clone();
            Some(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(LEASE_RENEWAL_INTERVAL).await;
                    let _ = sched.renew_lease(&id).await;
                    let _ = sched.renew_execution_lock(&id).await;
                }
            }))
        } else {
            None
        };

        // Execute the task
        executor::handle_task(&config, router, ollama, store, publisher, &id, &project, &goal, &status).await;

        // Release execution lock + task lease
        if let Some(sched) = &scheduler {
            let _ = sched.release_execution_lock().await;
            let _ = sched.release_lease(&id).await;
        }
        if let Some(handle) = lease_handle {
            handle.abort();
        }
    });
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".into())
}
