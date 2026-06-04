//! Autonomous operation driver.
//!
//! Opt-in (config `[autopilot] enabled`, default false). On each tick, when
//! the system is idle and under the daily run cap, the driver:
//!   1. auto-approves any autonomous (`agent_id = autopilot`) runs sitting in
//!      `planned` (so they execute without a human), and
//!   2. if nothing autonomous is in flight, drains the oldest queued
//!      `autopilot_goal` into a new `planning` run tagged `agent_id=autopilot`.
//!
//! Safety: backlog-driven only (no LLM goal-invention in v1), one run at a
//! time, hard `max_runs_per_day` ceiling. The normal pipeline (plan → approve
//! → workflow execute) does the rest.

use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

use knowledge_base::KnowledgeBackend;
use swarm_config::AutopilotConfig;
use swarm_events::NatsScheduler;

/// Wakes the autopilot loop to check the backlog immediately. Signalled when a
/// run completes so continuous mode picks up the next goal gap-free, instead of
/// waiting for the next poll tick.
static PICKUP: OnceLock<Arc<Notify>> = OnceLock::new();
fn pickup() -> &'static Arc<Notify> {
    PICKUP.get_or_init(|| Arc::new(Notify::new()))
}
/// Signal the autopilot to re-check the backlog now (call on run completion).
pub fn notify_pickup() {
    pickup().notify_one();
}

/// agent_id stamped at creation (cosmetic; the planner reassigns it).
pub const AUTOPILOT_AGENT_ID: &str = "autopilot";
/// Durable tag on autonomous runs. Unlike agent_id (which the lifecycle
/// rewrites to 'planner'/'daemon'), `source` is set once and never touched,
/// so auto-approve + the daily cap can reliably find autonomous runs.
pub const AUTOPILOT_SOURCE: &str = "autopilot";

/// Spawn the autopilot driver loop. No-op (returns immediately) when disabled.
pub fn spawn(
    cfg: AutopilotConfig,
    store: Arc<dyn KnowledgeBackend>,
    _scheduler: Option<Arc<NatsScheduler>>,
    max_runs: usize,
) {
    if !cfg.enabled {
        info!("Autopilot disabled (set [autopilot] enabled = true to opt in)");
        return;
    }
    info!(
        tick_secs = cfg.tick_secs,
        max_runs_per_day = cfg.max_runs_per_day,
        auto_approve = cfg.auto_approve,
        continuous = cfg.continuous,
        "Autopilot ENABLED — autonomous backlog execution"
    );
    tokio::spawn(async move {
        // Continuous mode: short fallback poll + event-driven wake on run
        // completion (notify_pickup). Otherwise the periodic tick.
        let interval = if cfg.continuous {
            Duration::from_secs(cfg.tick_secs.clamp(5, 20))
        } else {
            Duration::from_secs(cfg.tick_secs.max(10))
        };
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = pickup().notified() => {}
            }
            if let Err(e) = tick(&cfg, store.as_ref(), max_runs).await {
                warn!(error = %e, "autopilot tick failed");
            }
        }
    });
}

async fn tick(
    cfg: &AutopilotConfig,
    store: &dyn KnowledgeBackend,
    max_runs: usize,
) -> anyhow::Result<()> {
    // 1. Auto-approve autonomous planned runs so they execute hands-free.
    if cfg.auto_approve {
        let approved = store.query_json(
            "UPDATE agent_run SET status = 'approved', \
                 progress_message = 'Autopilot auto-approved' \
             WHERE source = $src AND status = 'planned' RETURN id",
            serde_json::json!({ "src": AUTOPILOT_SOURCE }),
        ).await?;
        if !approved.is_empty() {
            info!(count = approved.len(), "autopilot auto-approved planned runs");
        }
    }

    // 2. Fill the run pipeline up to `max_runs` (parallel goals). Each run is
    //    workspace-isolated and the main loop's execution slots cap real
    //    concurrency, so this just keeps the backlog feeding the slots without
    //    unbounded fan-out. Counts every non-terminal run (human or autopilot);
    //    the executing run is included, so at most `max_runs` are ever in flight.
    if active_run_count(store).await? >= max_runs.max(1) as i64 {
        return Ok(());
    }

    // 3. Daily cap (cost guardrail) — bypassed in continuous mode (the quality
    //    gate is the real guard; local inference has no per-run $ cost).
    if !cfg.continuous {
        let today_count = autonomous_runs_today(store).await?;
        if today_count >= cfg.max_runs_per_day as i64 {
            return Ok(());
        }
    }

    // 4. Drain the oldest queued backlog goal.
    // NOTE: surrealdb requires ORDER BY fields to appear in the projection.
    let rows = store.query_json(
        "SELECT id, project, goal, created_at FROM autopilot_goal WHERE status = 'queued' ORDER BY created_at ASC LIMIT 1",
        serde_json::Value::Null,
    ).await?;
    let Some(row) = rows.into_iter().next() else { return Ok(()) };
    let goal_id = row.get("id").map(|v| v.to_string().trim_matches('"').to_string()).unwrap_or_default();
    let project = row.get("project").and_then(|v| v.as_str()).unwrap_or("default").to_string();
    let goal = row.get("goal").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if goal.is_empty() {
        return Ok(());
    }

    // Mark the backlog row consumed, then create the run.
    store.db_query_raw(&format!(
        "UPDATE {goal_id} SET status = 'started', started_at = time::now()"
    )).await?;

    let create = format!(
        "CREATE agent_run SET project = '{}', task_description = '{}', status = 'planning', \
             agent_id = '{}', source = '{}', model_used = 'auto', created_at = time::now(), \
             files_modified = [], tokens_input = 0, tokens_output = 0, duration_ms = 0",
        project.replace('\'', ""),
        goal.replace('\'', ""),
        AUTOPILOT_AGENT_ID,
        AUTOPILOT_SOURCE,
    );
    store.db_query_raw(&create).await?;
    info!(project = %project, goal = %goal, continuous = cfg.continuous, "autopilot started autonomous goal");
    Ok(())
}

/// Count non-terminal runs (any source) — the live pipeline depth. Used to cap
/// how many runs are queued/in-flight at once (parallel-run fan-out guard).
async fn active_run_count(store: &dyn KnowledgeBackend) -> anyhow::Result<i64> {
    let rows = store.query_json(
        "SELECT count() AS c FROM agent_run \
         WHERE status IN ['pending', 'planning', 'planned', 'approved', 'running'] GROUP ALL",
        serde_json::Value::Null,
    ).await?;
    Ok(rows.first().and_then(|v| v.get("c")).and_then(|c| c.as_i64()).unwrap_or(0))
}

/// Count autonomous runs created since the start of the current UTC day.
async fn autonomous_runs_today(store: &dyn KnowledgeBackend) -> anyhow::Result<i64> {
    let rows = store.query_json(
        "SELECT count() AS c FROM agent_run \
         WHERE source = $src AND created_at > time::floor(time::now(), 1d) GROUP ALL",
        serde_json::json!({ "src": AUTOPILOT_SOURCE }),
    ).await?;
    Ok(rows.first().and_then(|v| v.get("c")).and_then(|c| c.as_i64()).unwrap_or(0))
}
