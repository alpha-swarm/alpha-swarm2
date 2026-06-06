//! GitHub Issues as the 1:1 ticketing backend.
//!
//! One background poll loop, two directions:
//!   - INGEST: open issues labelled `trigger_label` → `autopilot_goal` rows
//!     (tagged `external_id = "owner/repo#N"`), then label + comment the issue.
//!   - RECONCILE: each `agent_run` carrying an `external_id` → sync its status
//!     to the issue as a `swarm:*` label + a comment (and a PR link on pass).
//!
//! The `swarm:*` label IS the last-synced-state marker, so reconcile is
//! idempotent (skip when the issue already carries the target label) — no
//! comment spam. Auth is ambient `gh` (deny-by-default: every `gh` failure is
//! a logged warning, never blocks the loop or autopilot). Polling, not
//! webhooks (the daemon is behind NAT).

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use knowledge_base::KnowledgeBackend;
use swarm_config::GithubConfig;

/// Daemon-managed lifecycle labels. `swarm` (the trigger) is human-applied.
const STATE_LABELS: &[&str] = &[
    "swarm:queued",
    "swarm:running",
    "swarm:done",
    "swarm:failed",
    "swarm:skipped",
];
const LABEL_QUEUED: &str = "swarm:queued";
/// Lower bound on the poll interval regardless of config.
const MIN_POLL_SECS: u64 = 15;

/// Run `gh` with the given args (blocking, off the async runtime). Ok(stdout)
/// on success, Err(stderr) otherwise.
async fn gh(args: Vec<String>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("gh")
            .args(&args)
            .output()
            .map_err(|e| format!("gh spawn failed: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    })
    .await
    .unwrap_or_else(|e| Err(format!("gh join error: {e}")))
}

/// Map a run status to the issue's lifecycle label.
fn status_to_label(status: &str) -> &'static str {
    match status {
        "passed" => "swarm:done",
        "failed" => "swarm:failed",
        "skipped" | "cancelled" => "swarm:skipped",
        // pending / planning / planned / approved / running
        _ => "swarm:running",
    }
}

/// SurrealQL single-quoted-string escape (raw queries, no params).
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Spawn the GitHub sync loop. No-op (returns) when disabled.
pub fn spawn(cfg: GithubConfig, store: Arc<dyn KnowledgeBackend>) {
    if !cfg.enabled {
        info!("GitHub sync disabled (set [github] enabled = true to opt in)");
        return;
    }
    tokio::spawn(async move {
        // Resolve owner/repo once: config value, else infer from the daemon
        // repo's origin remote via gh.
        let repo = if !cfg.repo.is_empty() {
            cfg.repo.clone()
        } else {
            match gh(vec![
                "repo".into(), "view".into(),
                "--json".into(), "nameWithOwner".into(),
                "-q".into(), ".nameWithOwner".into(),
            ]).await {
                Ok(s) => s.trim().to_string(),
                Err(e) => {
                    warn!(error = %e, "github_sync: cannot resolve repo (set [github] repo); disabling");
                    return;
                }
            }
        };
        if repo.is_empty() {
            warn!("github_sync: empty repo; disabling");
            return;
        }
        info!(repo = %repo, trigger = %cfg.trigger_label, poll_secs = cfg.poll_secs, "GitHub sync ENABLED");

        let interval = Duration::from_secs(cfg.poll_secs.max(MIN_POLL_SECS));
        loop {
            if let Err(e) = ingest(store.as_ref(), &cfg, &repo).await {
                warn!(error = %e, "github_sync: ingest failed");
            }
            if let Err(e) = reconcile(store.as_ref(), &repo).await {
                warn!(error = %e, "github_sync: reconcile failed");
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// Pull open trigger-labelled issues → queue any not yet ingested.
async fn ingest(store: &dyn KnowledgeBackend, cfg: &GithubConfig, repo: &str) -> Result<(), String> {
    let out = gh(vec![
        "issue".into(), "list".into(),
        "-R".into(), repo.into(),
        "--label".into(), cfg.trigger_label.clone(),
        "--state".into(), "open".into(),
        "--json".into(), "number,title,body,labels".into(),
    ]).await?;
    let issues: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    let Some(arr) = issues.as_array() else { return Ok(()) };

    for issue in arr {
        let number = issue.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
        if number == 0 {
            continue;
        }
        let external_id = format!("{repo}#{number}");

        // Already processed? (label marker OR an existing goal row — belt + braces)
        let labels: Vec<String> = issue.get("labels").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(String::from)).collect())
            .unwrap_or_default();
        if labels.iter().any(|l| STATE_LABELS.contains(&l.as_str())) {
            continue;
        }
        let existing = store.query_json(
            &format!("SELECT id FROM autopilot_goal WHERE external_id = '{}'", esc(&external_id)),
            serde_json::Value::Null,
        ).await.unwrap_or_default();
        if !existing.is_empty() {
            continue;
        }

        let title = issue.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let body = issue.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let goal = format!("{title}\n\n{body}");
        let create = format!(
            "CREATE autopilot_goal SET project = '{}', goal = '{}', status = 'queued', \
                 external_id = '{}', created_at = time::now()",
            esc(&cfg.project), esc(&goal), esc(&external_id),
        );
        store.db_query_raw(&create).await.map_err(|e| e.to_string())?;

        let n = number.to_string();
        let _ = gh(vec!["issue".into(), "edit".into(), n.clone(), "-R".into(), repo.into(),
            "--add-label".into(), LABEL_QUEUED.into()]).await;
        let _ = gh(vec!["issue".into(), "comment".into(), n, "-R".into(), repo.into(),
            "--body".into(), "🤖 Queued as a swarm ticket — the agent will pick this up.".into()]).await;
        info!(issue = number, %external_id, "github_sync: ingested issue → autopilot_goal");
    }
    Ok(())
}

/// Sync each ticketed run's status back to its issue (label + comment).
async fn reconcile(store: &dyn KnowledgeBackend, repo: &str) -> Result<(), String> {
    // created_at IS in the projection (the embedded store rejects ORDER BY on
    // a field that isn't selected).
    let rows = store.query_json(
        "SELECT id, status, external_id, created_at FROM agent_run \
             WHERE external_id != NONE ORDER BY created_at DESC LIMIT 100",
        serde_json::Value::Null,
    ).await.map_err(|e| e.to_string())?;

    let prefix = format!("{repo}#");
    for r in rows {
        let ext = r.get("external_id").and_then(|v| v.as_str()).unwrap_or("");
        if !ext.starts_with(&prefix) {
            continue;
        }
        let Some(number) = ext.rsplit('#').next().and_then(|n| n.parse::<i64>().ok()) else { continue };
        let status = r.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let target = status_to_label(status);

        // Current labels — skip if already synced to the target state.
        let cur: Vec<String> = gh(vec![
            "issue".into(), "view".into(), number.to_string(), "-R".into(), repo.into(),
            "--json".into(), "labels".into(), "-q".into(), "[.labels[].name]".into(),
        ]).await.ok()
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default();
        if cur.iter().any(|l| l == target) {
            continue;
        }

        let n = number.to_string();
        // Swap state labels: drop any stale swarm:* (except target), add target.
        for old in STATE_LABELS {
            if *old != target && cur.iter().any(|l| l == old) {
                let _ = gh(vec!["issue".into(), "edit".into(), n.clone(), "-R".into(), repo.into(),
                    "--remove-label".into(), (*old).into()]).await;
            }
        }
        let _ = gh(vec!["issue".into(), "edit".into(), n.clone(), "-R".into(), repo.into(),
            "--add-label".into(), target.into()]).await;

        let comment = match status {
            "passed" => {
                let branch = format!("swarm/issue-{number}");
                let pr = gh(vec!["pr".into(), "list".into(), "-R".into(), repo.into(),
                    "--head".into(), branch, "--json".into(), "url".into(), "-q".into(), ".[0].url".into()])
                    .await.ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                match pr {
                    Some(url) => format!("✅ Quality gate passed. PR ready for review: {url}"),
                    None => "✅ Quality gate passed — changes landed.".to_string(),
                }
            }
            "failed" => "❌ Run failed the quality gate — see the dashboard for details.".to_string(),
            "skipped" | "cancelled" => "⏭️ Run skipped.".to_string(),
            _ => "🔧 Agent started working on this ticket.".to_string(),
        };
        let _ = gh(vec!["issue".into(), "comment".into(), n, "-R".into(), repo.into(),
            "--body".into(), comment]).await;
        info!(issue = number, status, "github_sync: synced run status → issue");
    }
    Ok(())
}
