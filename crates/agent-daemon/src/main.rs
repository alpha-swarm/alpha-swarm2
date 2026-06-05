/// Agent daemon: distributed task executor coordinated via NATS KV.
///
/// Primary mode: NATS KV scheduler (watch for tasks, claim via leases, heartbeat).
/// Fallback mode: SurrealDB polling (if NATS unavailable).
///
/// Usage: cargo run -p agent-daemon
mod repo;
mod executor;
mod security_scan;
mod git_pr;
mod hooks;
mod db_bridge;
mod http_bridge;
mod autopilot;
pub mod resources;
pub mod provider_client;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn};

use inference_client::{ClaudeBackend, InferenceRouter, OllamaBackend};
use knowledge_base::{KnowledgeBackend, KnowledgeStore};
use swarm_config::SwarmConfig;
use swarm_events::{EventPublisher, NatsScheduler, scheduler::HostResources};

/// How often to poll SurrealDB in fallback mode.
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// How often to publish resource snapshots.
const RESOURCE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How often to renew leases for running tasks.
const LEASE_RENEWAL_INTERVAL: Duration = Duration::from_secs(120);
/// Text used by the startup embedding-dimension probe.
const EMBED_PROBE_TEXT: &str = "probe";
/// How often unused stale memory entries are pruned (per project).
const MEMORY_DECAY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often to re-ping models to keep them resident in Ollama. Under Ollama's
/// 5-minute default idle-unload window; with keep_alive=-1 this is a safety net
/// (also re-warms after an Ollama restart).
const WARM_INTERVAL: Duration = Duration::from_secs(240);
/// Shared, warm cargo target dir for the AGENT's quality tools
/// (run_check/run_build). Each per-run workspace otherwise compiles from a COLD
/// target (~100s/check — the loop's real bottleneck); a process-wide shared dir
/// makes those checks incremental (~seconds after the first build). The gate
/// keeps its own GATE_TARGET_DIR, so the two never contend on cargo's lock.
const AGENT_TARGET_DIR: &str = "/tmp/alpha-swarm/agent-target";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,agent_core=info,swarm_orchestrator=info")
        .init();

    // Warm, shared cargo target for agent quality tools (run_check/run_build) so
    // mid-iteration `cargo check`/`clippy` are incremental, not cold full builds.
    // Respect an explicit override; set before any task/thread is spawned.
    if std::env::var_os("CARGO_TARGET_DIR").is_none() {
        let _ = std::fs::create_dir_all(AGENT_TARGET_DIR);
        unsafe { std::env::set_var("CARGO_TARGET_DIR", AGENT_TARGET_DIR); }
        info!(dir = AGENT_TARGET_DIR, "Warm shared cargo target dir for agent checks");
    }

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

    // Ollama keep_alive (empty string → None → Ollama's 5m default).
    let keep_alive = (!config.ollama.keep_alive.is_empty()).then(|| config.ollama.keep_alive.clone());

    // Register Ollama backends from providers config (if any)
    if !config.providers.is_empty() {
        let mut sorted = config.providers.clone();
        sorted.sort_by_key(|p| p.priority);
        for provider in &sorted {
            match provider.provider_type.as_str() {
                "ollama" => {
                    info!(url = %provider.url, priority = provider.priority, "Adding Ollama provider");
                    router = router.add_backend(OllamaBackend::with_keep_alive(&provider.url, keep_alive.clone()));
                }
                // Any OpenAI-compatible endpoint: openai, groq, together, vllm,
                // lmstudio, deepinfra, etc. — set `url` to the provider's base.
                "openai" | "openai-compat" | "groq" | "together" | "vllm" | "lmstudio" => {
                    info!(url = %provider.url, model = %provider.model, kind = %provider.provider_type, "Adding OpenAI-compatible provider");
                    router = router.add_backend(inference_client::OpenAICompatBackend::new(
                        &provider.url, &provider.api_key, &provider.model,
                    ));
                }
                // Gemini via its OpenAI-compatible surface. Default endpoint
                // when `url` is omitted.
                "gemini" | "google" => {
                    let url = if provider.url.is_empty() {
                        "https://generativelanguage.googleapis.com/v1beta/openai"
                    } else { provider.url.as_str() };
                    info!(url, model = %provider.model, "Adding Gemini (OpenAI-compat) provider");
                    router = router.add_backend(inference_client::OpenAICompatBackend::new(
                        url, &provider.api_key, &provider.model,
                    ));
                }
                "claude" | "anthropic" => {
                    info!(model = %provider.model, "Adding Claude provider");
                    router = router.add_backend(ClaudeBackend::new(&provider.api_key).with_model(&provider.model));
                }
                other => { warn!(provider_type = other, "Unknown provider type, skipping"); }
            }
        }
    } else {
        // Fallback: use single [ollama] config
        router = router.add_backend(OllamaBackend::with_keep_alive(&config.ollama.url, keep_alive.clone()));
    }
    let router = Arc::new(router);
    let ollama = Arc::new(OllamaBackend::with_keep_alive(&config.ollama.url, keep_alive.clone()));

    // Embedding-dimension probe: a wrong-dimension embed model corrupts every
    // vector table and is rejected by the HNSW indexes — fail fast on mismatch.
    // A transient Ollama outage only warns (model may load later).
    match ollama.embed(&config.defaults.embed_model, EMBED_PROBE_TEXT).await {
        Ok(v) if v.len() != knowledge_base::EMBED_DIM => {
            anyhow::bail!(
                "Embed model '{}' returns {}-dim vectors but EMBED_DIM is {} — configure a matching model (e.g. nomic-embed-text)",
                config.defaults.embed_model, v.len(), knowledge_base::EMBED_DIM
            );
        }
        Ok(_) => info!(model = %config.defaults.embed_model, dim = knowledge_base::EMBED_DIM, "Embed dimension probe OK"),
        Err(e) => warn!(model = %config.defaults.embed_model, error = %e, "Embed probe skipped (Ollama unreachable)"),
    }

    // Keep the pipeline's models resident in Ollama. keep_alive=-1 on every
    // request already prevents idle-unload; this also PRE-warms on startup and
    // re-pings under the 5-minute window so the model is hot before the first /
    // next queue item (no cold reload between back-to-back jobs) and recovers
    // after an Ollama restart. Warms every distinct chat tier + the embed model.
    {
        let ollama_warm = Arc::clone(&ollama);
        // Each tier PINS its own model (preferred_model = tier.model in
        // agent.rs / planner.rs), so a run touches all of them — keep every
        // distinct tier model co-resident, not just the router's fallback pick.
        // (Set OLLAMA_MAX_LOADED_MODELS >= number of distinct models on the host.)
        let mut chat_models = vec![
            config.tiers.orchestrator.model.clone(),
            config.tiers.agent.model.clone(),
            config.tiers.worker.model.clone(),
        ];
        chat_models.sort();
        chat_models.dedup();
        chat_models.retain(|m| !m.is_empty());
        let embed_model = config.defaults.embed_model.clone();
        // Warm at the orchestrator context window, NOT max_tokens=1: a tiny
        // ceiling makes num_ctx collapse to ~1, which Ollama treats as "use the
        // model default" and loads llama3.3:70b at its full 131072 context
        // (~81GB VRAM) — starving the box so the 32b/embed models can't co-load.
        // A sane ceiling keeps the resident KV cache (hence VRAM) small.
        let warm_ctx = config.tiers.orchestrator.context_window;
        tokio::spawn(async move {
            use inference_client::InferenceBackend;
            let opts = inference_client::InferenceOptions { max_tokens: Some(warm_ctx), ..Default::default() };
            info!(chat = ?chat_models, embed = %embed_model, "Warming tier models into Ollama memory (keep_alive)");
            loop {
                let messages = vec![inference_client::ChatMessage::user("warm")];
                for m in &chat_models {
                    if let Err(e) = ollama_warm.chat(m, &messages, &opts).await {
                        warn!(model = %m, error = %e, "warm ping failed (will load on first use)");
                    }
                }
                if !embed_model.is_empty() {
                    let _ = ollama_warm.embed(&embed_model, "warm").await;
                }
                tokio::time::sleep(WARM_INTERVAL).await;
            }
        });
    }

    // Open the DB (embedded surrealkv by default — this daemon is the sole
    // owner; mode=remote is the external-server escape hatch). Held as the
    // KnowledgeBackend trait so the storage tech stays swappable.
    let store: Arc<dyn KnowledgeBackend> =
        Arc::new(KnowledgeStore::connect_with(&config.surrealdb).await?);

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

    // Clear stale execution locks AND per-task leases from previous daemon
    // crashes. Without the lease purge, recovered tasks stay wedged: their
    // `lease.*` claims survive the crash and every poll sees them "already
    // claimed, deferring".
    if let Some(ref sched) = scheduler {
        let _ = sched.release_execution_lock().await;
        match sched.release_all_leases().await {
            Ok(n) => info!(purged = n, "Cleared stale execution locks + task leases"),
            Err(e) => warn!(error = %e, "Failed to purge stale task leases"),
        }
    }

    // Workflow engine: persisted DAG runs, pause/resume, adaptive replanning.
    let wf_engine = Arc::new(swarm_workflow::WorkflowEngine::new(
        swarm_workflow::WorkflowRepo::new(Arc::clone(&store)),
        publisher.clone(),
    ));
    match swarm_workflow::seed_templates(wf_engine.repo()).await {
        Ok(n) => info!(templates = n, "Workflow templates seeded"),
        Err(e) => warn!("Workflow template seeding failed: {e}"),
    }
    // Crash recovery: workflows left 'running' resume through the normal task
    // flow — their agent_run rows were just reset to 'pending' above, and the
    // executor resumes the persisted workflow_run instead of re-planning.
    match wf_engine.repo().list_active().await {
        Ok(active) if !active.is_empty() => {
            info!(count = active.len(), "Active workflow runs found (will resume via task flow)");
        }
        Ok(_) => {}
        Err(e) => warn!("Workflow recovery scan failed: {e}"),
    }

    // HTTP /sql shim for WASM components in embedded mode (wash dev's
    // messaging plugin is in-memory only; components fall back to this HTTP
    // contract on the old SurrealDB address).
    if config.surrealdb.mode == swarm_config::SurrealMode::Embedded {
        tokio::spawn(http_bridge::serve(
            config.surrealdb.url.clone(),
            Arc::clone(&store),
            Arc::clone(&wf_engine),
            config.nats.url.clone(),
        ));
    }

    let memory = Arc::new(knowledge_base::MemoryStore::new(
        Arc::clone(&store),
        Arc::clone(&ollama),
        config.defaults.embed_model.clone(),
    ));

    // Embedded Wassette WASM tool host: load configured tool components, grant
    // their capabilities, and install the process-global tool set so agent runs
    // surface them as ordinary tools (no-op when [wassette] enabled = false).
    match tool_host::install_from_config(&config.wassette).await {
        Ok(0) => {}
        Ok(n) => info!(tools = n, "WASM tool host ready"),
        Err(e) => warn!(error = %e, "WASM tool host init failed (agents run without WASM tools)"),
    }

    // Embedded ruvector ANN index (HNSW + SIMD, pure-Rust) for memory
    // retrieval. Ephemeral cache — rebuilt from SurrealDB (authoritative) here.
    if config.learning.enabled {
        match knowledge_base::rvindex::RvIndex::init(&config.surrealdb.ruvector_path, knowledge_base::EMBED_DIM) {
            Ok(()) => match memory.rebuild_index().await {
                Ok(n) => info!(indexed = n, "ruvector ANN index ready"),
                Err(e) => warn!(error = %e, "ruvector rebuild failed (degraded to cosine scan)"),
            },
            Err(e) => warn!(error = %e, "ruvector init failed (degraded to cosine scan)"),
        }
    }

    // NATS DB bridge: the query surface for native consumers (TUI, eval,
    // event-consumer, remote daemons).
    match async_nats::connect(&config.nats.url).await {
        Ok(bridge_client) => {
            let bridge = db_bridge::DbBridge::new(
                Arc::clone(&store),
                Arc::clone(&wf_engine),
                Arc::clone(&memory),
            );
            tokio::spawn(bridge.serve(bridge_client));
        }
        Err(e) => warn!("DB bridge NATS connect failed (bridge unavailable): {e}"),
    }

    // Memory hygiene: periodically prune unused stale entries per project.
    if config.learning.enabled {
        let decay_store = Arc::clone(&store);
        let decay_memory = Arc::clone(&memory);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(MEMORY_DECAY_INTERVAL).await;
                let projects = decay_store
                    .query_json("SELECT name FROM project", serde_json::Value::Null)
                    .await
                    .unwrap_or_default();
                for row in projects {
                    let Some(name) = row.get("name").and_then(|n| n.as_str()) else { continue };
                    match decay_memory.decay(name).await {
                        Ok(0) => {}
                        Ok(pruned) => info!(project = name, pruned, "Memory decay pruned entries"),
                        Err(e) => warn!(project = name, error = %e, "Memory decay failed"),
                    }
                }
            }
        });
    }

    // Autonomous operation (opt-in; OFF by default). Drains the autopilot_goal
    // backlog when idle + under the daily cap.
    autopilot::spawn(config.autopilot.clone(), Arc::clone(&store), scheduler.clone(), config.resources.max_concurrent_runs);

    // Start resource heartbeat
    {
        let store = Arc::clone(&store);
        let scheduler = scheduler.clone();
        let res_config = config.resources.clone();
        let daemon_id = daemon_id.clone();
        tokio::spawn(async move {
            loop {
                let snapshots = resources::check_all_hosts(&res_config).await;

                // Write to SurrealDB (for dashboard). DELETE + CREATEs run in one
                // transaction so concurrent reads never observe an empty table.
                let mut tx = String::from("BEGIN TRANSACTION; DELETE FROM resource_snapshot;");
                for snap in &snapshots {
                    let models_json = serde_json::to_string(&snap.ollama_models).unwrap_or_else(|_| "[]".into());
                    tx.push_str(&format!(
                        " CREATE resource_snapshot SET host='{}', host_type='{}', cpu_percent={:.1}, ram_total_mb={}, ram_used_mb={}, ram_percent={:.1}, disk_total_gb={:.1}, disk_free_gb={:.1}, disk_percent={:.1}, ollama_models={}, timestamp=time::now();",
                        snap.host, snap.host_type,
                        snap.cpu_percent, snap.ram_total_mb, snap.ram_used_mb, snap.ram_percent,
                        snap.disk_total_gb, snap.disk_free_gb, snap.disk_percent,
                        models_json,
                    ));
                }
                tx.push_str(" COMMIT TRANSACTION;");
                let _ = store.db_query_raw(&tx).await;

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
    process_pending(&config, &router, &ollama, &store, &publisher, &scheduler, &wf_engine).await;

    // Main loop: poll SurrealDB, run 1 goal at a time via NATS KV execution lock
    info!("Polling SurrealDB every {:?} (1 goal at a time via NATS KV lock)", FALLBACK_POLL_INTERVAL);
    loop {
        tokio::time::sleep(FALLBACK_POLL_INTERVAL).await;
        if !resources::can_schedule(&config.resources) { continue; }
        // Effective cap adapts to live memory pressure when dynamic_slots is on;
        // otherwise the static max_concurrent_runs.
        let max_runs = resources::effective_slots(&config.resources);

        // Fill free execution slots with pending tasks (planning OR execution).
        // Every run gets an isolated workspace + its own gate, so concurrent runs
        // cannot corrupt each other; the slot count caps real concurrency and the
        // per-task lease (try_claim) dedups so a task is never spawned twice. When
        // all slots are busy we stop and retry next poll.
        let Ok(pending) = store.list_pending().await else { continue };
        for task in pending {
            if !resources::can_schedule(&config.resources) { break; }
            let id = task.id.clone().unwrap_or_default();
            if id.is_empty() { continue; }
            let project = task.project.clone();
            let goal = task.task_description.clone();
            let status = serde_json::to_string(&task.status).unwrap_or_default().trim_matches('"').to_string();

            match &scheduler {
                Some(sched) => {
                    // Per-task claim (atomic dedup) — skip tasks already in-flight.
                    if !matches!(sched.try_claim(&id).await, Ok(true)) { continue; }
                    match sched.try_acquire_execution_slot(&id, max_runs).await {
                        Ok(Some(slot)) => {
                            spawn_task(config.clone(), Arc::clone(&router), Arc::clone(&ollama), Arc::clone(&store), publisher.clone(), scheduler.clone(), Arc::clone(&wf_engine), id, project, goal, status, Some(slot));
                        }
                        _ => {
                            // No free slot — release the claim so it's re-picked next poll.
                            let _ = sched.release_lease(&id).await;
                            break;
                        }
                    }
                }
                None => {
                    // No scheduler: strictly serial — one per poll.
                    spawn_task(config.clone(), Arc::clone(&router), Arc::clone(&ollama), Arc::clone(&store), publisher.clone(), None, Arc::clone(&wf_engine), id, project, goal, status, None);
                    break;
                }
            }
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Process any tasks already pending in SurrealDB (first one only).
#[allow(clippy::too_many_arguments)]
async fn process_pending(
    config: &SwarmConfig,
    router: &Arc<InferenceRouter>,
    ollama: &Arc<OllamaBackend>,
    store: &Arc<dyn KnowledgeBackend>,
    publisher: &Option<Arc<EventPublisher>>,
    scheduler: &Option<Arc<NatsScheduler>>,
    wf_engine: &Arc<swarm_workflow::WorkflowEngine>,
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

                let slot = if let Some(sched) = scheduler {
                    if !matches!(sched.try_claim(&id).await, Ok(true)) {
                        info!(id = %id, "Already claimed, deferring");
                        return;
                    }
                    match sched.try_acquire_execution_slot(&id, resources::effective_slots(&config.resources)).await {
                        Ok(Some(s)) => Some(s),
                        _ => { let _ = sched.release_lease(&id).await; info!(id = %id, "No free slot, deferring"); return; }
                    }
                } else { None };

                let project = task.project.clone();
                let goal = task.task_description.clone();
                let status = serde_json::to_string(&task.status).unwrap_or_default().trim_matches('"').to_string();

                spawn_task(config.clone(), Arc::clone(router), Arc::clone(ollama), Arc::clone(store), publisher.clone(), scheduler.clone(), Arc::clone(wf_engine), id, project, goal, status, slot);
            }
        }
        Err(e) => warn!("Failed to query pending tasks: {e}"),
    }
}

/// Primary mode: watch NATS KV for new tasks.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
async fn run_nats_kv_loop(
    config: &SwarmConfig,
    router: &Arc<InferenceRouter>,
    ollama: &Arc<OllamaBackend>,
    store: &Arc<dyn KnowledgeBackend>,
    publisher: &Option<Arc<EventPublisher>>,
    scheduler: &Arc<NatsScheduler>,
    wf_engine: &Arc<swarm_workflow::WorkflowEngine>,
) {

    let watcher = match scheduler.watch_tasks().await {
        Ok(w) => w,
        Err(e) => {
            warn!("Failed to watch NATS KV tasks, falling back to polling: {e}");
            run_surreal_poll_loop(config, router, ollama, store, publisher, wf_engine).await;
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

                        spawn_task(config.clone(), Arc::clone(router), Arc::clone(ollama), Arc::clone(store), publisher.clone(), Some(Arc::clone(scheduler)), Arc::clone(wf_engine), id, project, goal, status, None);
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
    store: &Arc<dyn KnowledgeBackend>,
    publisher: &Option<Arc<EventPublisher>>,
    wf_engine: &Arc<swarm_workflow::WorkflowEngine>,
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

                spawn_task(config.clone(), Arc::clone(router), Arc::clone(ollama), Arc::clone(store), publisher.clone(), None, Arc::clone(wf_engine), id, project, goal, status, None);
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
    store: Arc<dyn KnowledgeBackend>,
    publisher: Option<Arc<EventPublisher>>,
    scheduler: Option<Arc<NatsScheduler>>,
    wf_engine: Arc<swarm_workflow::WorkflowEngine>,
    id: String,
    project: String,
    goal: String,
    status: String,
    // Execution slot held for this run (None = no scheduler / legacy global lock).
    // Released on completion; renewed by the heartbeat so it doesn't TTL-expire.
    slot: Option<usize>,
) {
    tokio::spawn(async move {
        info!(id = %id, slot = ?slot, "Goal starting execution");

        // Start lease + execution slot renewal heartbeat
        let lease_handle = if let Some(sched) = &scheduler {
            let sched = Arc::clone(sched);
            let id = id.clone();
            Some(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(LEASE_RENEWAL_INTERVAL).await;
                    let _ = sched.renew_lease(&id).await;
                    match slot {
                        Some(s) => { let _ = sched.renew_execution_slot(s, &id).await; }
                        None => { let _ = sched.renew_execution_lock(&id).await; }
                    }
                }
            }))
        } else {
            None
        };

        // Execute the task
        executor::handle_task(&config, router, ollama, store, publisher, wf_engine, &id, &project, &goal, &status).await;

        // Release execution slot/lock + task lease
        if let Some(sched) = &scheduler {
            match slot {
                Some(s) => { let _ = sched.release_execution_slot(s).await; }
                None => { let _ = sched.release_execution_lock().await; }
            }
            let _ = sched.release_lease(&id).await;
        }
        if let Some(handle) = lease_handle {
            handle.abort();
        }
        // Wake the autopilot to pick up the next backlog goal immediately
        // (continuous-loop, gap-free pickup).
        autopilot::notify_pickup();
    });
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".into())
}
