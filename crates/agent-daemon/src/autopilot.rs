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

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use knowledge_base::KnowledgeBackend;
use swarm_config::AutopilotConfig;
use swarm_events::NatsScheduler;

/// agent_id tag marking autonomously-created runs.
pub const AUTOPILOT_AGENT_ID: &str = "autopilot";

/// Spawn the autopilot driver loop. No-op (returns immediately) when disabled.
pub fn spawn(
    cfg: AutopilotConfig,
    store: Arc<dyn KnowledgeBackend>,
    scheduler: Option<Arc<NatsScheduler>>,
) {
    if !cfg.enabled {
        info!("Autopilot disabled (set [autopilot] enabled = true to opt in)");
        return;
    }
    info!(
        tick_secs = cfg.tick_secs,
        max_runs_per_day = cfg.max_runs_per_day,
        auto_approve = cfg.auto_approve,
        "Autopilot ENABLED — autonomous backlog execution"
    );
    tokio::spawn(async move {
        let interval = Duration::from_secs(cfg.tick_secs.max(10));
        loop {
            tokio::time::sleep(interval).await;
            if let Err(e) = tick(&cfg, store.as_ref(), scheduler.as_deref()).await {
                warn!(error = %e, "autopilot tick failed");
            }
        }
    });
}

async fn tick(
    cfg: &AutopilotConfig,
    store: &dyn KnowledgeBackend,
    scheduler: Option<&NatsScheduler>,
) -> anyhow::Result<()> {
    // 1. Auto-approve autonomous planned runs so they execute hands-free.
    if cfg.auto_approve {
        let approved = store.query_json(
            "UPDATE agent_run SET status = 'approved', \
                 progress_message = 'Autopilot auto-approved' \
             WHERE agent_id = $aid AND status = 'planned' RETURN id",
            serde_json::json!({ "aid": AUTOPILOT_AGENT_ID }),
        ).await?;
        if !approved.is_empty() {
            info!(count = approved.len(), "autopilot auto-approved planned runs");
        }
    }

    // 2. Only start new work when idle (one autonomous run at a time).
    let lock_free = match scheduler {
        Some(s) => s.is_execution_lock_free().await,
        None => true,
    };
    if !lock_free {
        return Ok(());
    }
    // Anything (human or autopilot) already pending/in-flight? defer.
    let pending = store.list_pending().await.unwrap_or_default();
    if !pending.is_empty() {
        return Ok(());
    }

    // 3. Daily cap (cost guardrail).
    let today_count = autonomous_runs_today(store).await?;
    if today_count >= cfg.max_runs_per_day as i64 {
        return Ok(());
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
             agent_id = '{}', model_used = 'auto', created_at = time::now(), \
             files_modified = [], tokens_input = 0, tokens_output = 0, duration_ms = 0",
        project.replace('\'', ""),
        goal.replace('\'', ""),
        AUTOPILOT_AGENT_ID,
    );
    store.db_query_raw(&create).await?;
    info!(project = %project, goal = %goal, today = today_count + 1, "autopilot started autonomous goal");
    Ok(())
}

/// Count autonomous runs created since local midnight (UTC day boundary).
async fn autonomous_runs_today(store: &dyn KnowledgeBackend) -> anyhow::Result<i64> {
    let rows = store.query_json(
        "SELECT count() AS c FROM agent_run \
         WHERE agent_id = $aid AND created_at > time::floor(time::now(), 1d) GROUP ALL",
        serde_json::json!({ "aid": AUTOPILOT_AGENT_ID }),
    ).await?;
    Ok(rows.first().and_then(|v| v.get("c")).and_then(|c| c.as_i64()).unwrap_or(0))
}
