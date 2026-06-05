//! Minimal HTTP shim for WASM components.
//!
//! `wash dev` (wash 2.x) only provides the in-memory `wasmcloud:messaging`
//! plugin, so components can't reach the NATS DB bridge from a dev host. This
//! shim binds the address components already use for their HTTP fallback
//! (`config.surrealdb.url`, historically the external SurrealDB `/sql`
//! endpoint) and speaks just enough of that contract:
//!
//!   POST /sql            — SurrealQL text body → [{"status":"OK","result":[...]}, ...]
//!   POST /workflow/{op}  — JSON body → {"ok":..,"rows":[..]} (same as NATS bridge)
//!   GET  /health         — liveness
//!
//! The daemon stays the sole DB owner; this is a second front door to the
//! same bridge logic. Binds loopback only.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::Router;
use futures::{Stream, StreamExt};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{info, warn};

use knowledge_base::KnowledgeBackend;
use swarm_workflow::WorkflowEngine;

/// Statement prefixes silently dropped: components prepend idempotent session
/// context (`USE NS .. DB ..`, `DEFINE TABLE IF NOT EXISTS ..`) that the
/// embedded store already guarantees.
const STRIPPED_PREFIXES: &[&str] = &["USE ", "DEFINE TABLE"];
/// Leading verbs allowed on /sql statements (same policy as the NATS bridge).
const ALLOWED_VERBS: &[&str] = &["SELECT", "CREATE", "UPDATE", "UPSERT", "DELETE", "RELATE", "INSERT"];
/// Built Leptos dashboard bundle (trunk dist), relative to the daemon workdir.
///
/// This constant holds the directory path where the Leptos dashboard bundle is located.
/// The dashboard is served by the HTTP shim for WASM components, allowing access to the
/// built web application.
const DASHBOARD_DIR: &str = "dashboard-leptos/dist";

#[derive(Clone)]
struct Shared {
    store: Arc<dyn KnowledgeBackend>,
    engine: Arc<WorkflowEngine>,
    nats_url: String,
}

/// Serve the shim on `addr` (e.g. "127.0.0.1:8001"). Spawn me.
pub async fn serve(addr: String, store: Arc<dyn KnowledgeBackend>, engine: Arc<WorkflowEngine>, nats_url: String) {
    let shared = Shared { store, engine, nats_url };
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/sql", post(handle_sql))
        .route("/workflow/{op}", post(handle_workflow))
        // Server-Sent Events: bridge NATS SwarmEvents → the dashboard so it
        // refreshes in real time instead of only polling.
        .route("/events", get(handle_events))
        // Review: accumulated swarm/auto commits + open PRs (git log / gh).
        .route("/review", get(handle_review))
        // Graph: goals ↔ runs/agents ↔ SONA patterns/trajectories ↔ knowledge
        // (files), assembled from the real join tables into {nodes, edges}.
        .route("/graph", get(handle_graph))
        // Serve the Leptos dashboard bundle for any non-API path (so the daemon
        // is the single endpoint: API + UI on the same origin → no CORS).
        .fallback_service(tower_http::services::ServeDir::new(DASHBOARD_DIR))
        // no-cache so the browser always picks up a freshly-built bundle (the
        // bundle filename is content-hashed, but index.html must revalidate).
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        ))
        .with_state(shared);

    // Bind all interfaces on the configured port so the dashboard + /sql are
    // reachable cross-machine (e.g. http://picur:8001/), not just localhost.
    // Exposes /sql on the LAN — intended for the trusted local lattice.
    let port = addr.rsplit(':').next().unwrap_or("8001");
    let bind = format!("0.0.0.0:{port}");
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            warn!(bind, error = %e, "HTTP bridge bind failed — shim unavailable");
            return;
        }
    };
    info!(bind, "HTTP bridge listening (API /sql + Leptos dashboard)");
    if let Err(e) = axum::serve(listener, app).await {
        warn!(error = %e, "HTTP bridge server ended");
    }
}

/// SSE stream of swarm events. Subscribes to the NATS event bus
/// (`alpha-swarm.>`) per client and forwards each subject as an SSE message, so
/// the dashboard can refresh on real activity instead of only polling. Connect
/// failure → an empty stream (the dashboard keeps its poll fallback).
async fn handle_events(
    State(shared): State<Shared>,
) -> Sse<Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>> {
    let sub = match async_nats::connect(&shared.nats_url).await {
        Ok(client) => client.subscribe("alpha-swarm.>").await.ok(),
        Err(e) => {
            warn!(error = %e, "SSE: NATS connect failed — empty event stream");
            None
        }
    };
    let stream: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = match sub {
        Some(sub) => Box::pin(sub.map(|msg| Ok(Event::default().data(msg.subject.to_string())))),
        None => Box::pin(futures::stream::empty()),
    };
    Sse::new(stream)
}

/// Review feed: the loop's accumulated `swarm/auto` commits (not yet in
/// origin/main) + open PRs. Shells `git`/`gh` in the daemon's repo dir
/// (spawn_blocking); each piece degrades to empty on error.
async fn handle_review() -> axum::Json<serde_json::Value> {
    let out = tokio::task::spawn_blocking(|| {
        use std::process::Command;
        let commits = Command::new("git")
            .args(["log", "origin/main..swarm/auto", "--oneline", "--no-color", "-30"])
            .output().ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect::<Vec<_>>())
            .unwrap_or_default();
        let prs = Command::new("gh")
            .args(["pr", "list", "--state", "open", "--json", "number,title,headRefName", "--limit", "30"])
            .output().ok()
            .filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
            .unwrap_or_else(|| serde_json::json!([]));
        serde_json::json!({ "commits": commits, "prs": prs })
    }).await.unwrap_or_else(|_| serde_json::json!({ "commits": [], "prs": [] }));
    axum::Json(out)
}

/// Graph feed: the whole loop as a connected graph —
///
///   GOAL ──spawns──▶ RUN/AGENT ──guided(✓/✗)──▶ PATTERN (SONA)
///                       │   ├──recorded──▶ TRAJECTORY (SONA)
///                       │   └──touched───▶ FILE (knowledge / code_entity)
///
/// All edges come from the real link tables (no LLM-recorded fields, which are
/// unpopulated): goal↔run by task-text match, run↔pattern via
/// `pattern_effectiveness`, run↔trajectory via the trajectory's `key`, run↔file
/// via `files_modified`. Returns `{nodes:[{id,kind,label,...}], edges:[...]}`.
/// Each piece degrades to empty on error so a partial graph still renders.
async fn handle_graph(State(shared): State<Shared>) -> axum::Json<serde_json::Value> {
    use serde_json::{json, Value};
    async fn q(store: &Arc<dyn KnowledgeBackend>, sql: &str) -> Vec<Value> {
        store.query_json(sql, Value::Null).await.unwrap_or_default()
    }
    let s = |v: &Value, k: &str| -> String {
        match v.get(k) {
            Some(Value::String(x)) => x.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        }
    };
    let clip = |t: &str, n: usize| -> String {
        let t = t.trim();
        if t.chars().count() > n { format!("{}…", t.chars().take(n).collect::<String>()) } else { t.to_string() }
    };

    let goals = q(&shared.store, "SELECT goal, status, created_at FROM autopilot_goal ORDER BY created_at DESC LIMIT 40").await;
    let runs = q(&shared.store, "SELECT id, task_description, status, model_used, files_modified, created_at FROM agent_run ORDER BY created_at DESC LIMIT 40").await;
    let patterns = q(&shared.store, "SELECT key, content, use_count FROM memory_entry WHERE namespace = 'patterns' ORDER BY use_count DESC LIMIT 40").await;
    let trajectories = q(&shared.store, "SELECT key FROM memory_entry WHERE namespace = 'trajectories' LIMIT 80").await;
    let effs = q(&shared.store, "SELECT pattern_id, run_id, run_succeeded FROM pattern_effectiveness LIMIT 500").await;
    let entity_counts = q(&shared.store, "SELECT file, count() AS c FROM code_entity GROUP BY file").await;

    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();

    // RUN nodes — keyed by their record id so links resolve directly.
    let mut run_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in &runs {
        let id = s(r, "id");
        if id.is_empty() { continue; }
        run_ids.insert(id.clone());
        nodes.push(json!({
            "id": id, "kind": "run", "label": clip(&s(r, "task_description"), 46),
            "status": s(r, "status"), "model": s(r, "model_used"),
        }));
    }

    // GOAL nodes — link to runs by exact task-text match (the loop copies the
    // queued goal verbatim into agent_run.task_description).
    for (i, g) in goals.iter().enumerate() {
        let text = s(g, "goal");
        if text.trim().is_empty() { continue; }
        let gid = format!("goal:{i}");
        nodes.push(json!({ "id": gid, "kind": "goal", "label": clip(&text, 46), "status": s(g, "status") }));
        let needle = text.trim();
        for r in &runs {
            if s(r, "task_description").trim() == needle {
                edges.push(json!({ "src": gid, "dst": s(r, "id"), "kind": "spawns" }));
            }
        }
    }

    // PATTERN nodes — pattern_effectiveness.pattern_id is "patterns:{proj}:{key}",
    // so match a pattern's key as the id suffix.
    for p in &patterns {
        let key = s(p, "key");
        if key.is_empty() { continue; }
        let uses = p.get("use_count").and_then(|v| v.as_i64()).unwrap_or(0);
        nodes.push(json!({ "id": format!("pattern:{key}"), "kind": "pattern", "label": clip(&s(p, "content"), 44), "uses": uses }));
    }
    for e in &effs {
        let run_id = s(e, "run_id");
        if !run_ids.contains(&run_id) { continue; }
        let pid = s(e, "pattern_id");
        // Find which pattern node this effectiveness row points at.
        if let Some(p) = patterns.iter().find(|p| { let k = s(p, "key"); !k.is_empty() && pid.ends_with(&k) }) {
            let ok = e.get("run_succeeded").and_then(|v| v.as_bool()).unwrap_or(false);
            edges.push(json!({ "src": run_id, "dst": format!("pattern:{}", s(p, "key")), "kind": "guided", "ok": ok }));
        }
    }

    // TRAJECTORY nodes — the trajectory's key IS the run it was recorded from.
    for (i, t) in trajectories.iter().enumerate() {
        let run_id = s(t, "key");
        if !run_ids.contains(&run_id) { continue; }
        let tid = format!("traj:{i}");
        nodes.push(json!({ "id": tid, "kind": "trajectory", "label": "trajectory" }));
        edges.push(json!({ "src": run_id, "dst": tid, "kind": "recorded" }));
    }

    // FILE nodes (knowledge) — files each run modified, annotated with the
    // code_entity symbol count for that file.
    let counts: std::collections::HashMap<String, i64> = entity_counts.iter()
        .map(|e| (s(e, "file"), e.get("c").and_then(|v| v.as_i64()).unwrap_or(0)))
        .collect();
    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in &runs {
        let run_id = s(r, "id");
        let files = r.get("files_modified").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for f in files {
            let Some(path) = f.as_str() else { continue };
            let fid = format!("file:{path}");
            if seen_files.insert(path.to_string()) {
                let base = path.rsplit('/').next().unwrap_or(path);
                nodes.push(json!({ "id": fid, "kind": "file", "label": base, "path": path, "symbols": counts.get(path).copied().unwrap_or(0) }));
            }
            edges.push(json!({ "src": run_id, "dst": fid, "kind": "touched" }));
        }
    }

    axum::Json(json!({ "nodes": nodes, "edges": edges }))
}

/// Execute a SurrealQL text body, one statement at a time, replying in the
/// external SurrealDB `/sql` response shape the components already parse.
async fn handle_sql(State(shared): State<Shared>, body: String) -> (StatusCode, String) {
    let mut results: Vec<serde_json::Value> = Vec::new();

    for stmt in body.split(';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        let upper = stmt.to_uppercase();
        if STRIPPED_PREFIXES.iter().any(|p| upper.starts_with(p)) {
            // Session-context statement — already satisfied by the embedded store.
            results.push(serde_json::json!({ "status": "OK", "result": serde_json::Value::Null }));
            continue;
        }
        let verb = upper.split_whitespace().next().unwrap_or("");
        if !ALLOWED_VERBS.contains(&verb) {
            return (
                StatusCode::FORBIDDEN,
                serde_json::json!([{ "status": "ERR", "result": format!("verb not allowed: {verb}") }]).to_string(),
            );
        }
        match shared.store.query_json(stmt, serde_json::Value::Null).await {
            Ok(rows) => results.push(serde_json::json!({ "status": "OK", "result": rows })),
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    serde_json::json!([{ "status": "ERR", "result": e.to_string() }]).to_string(),
                );
            }
        }
    }

    (StatusCode::OK, serde_json::Value::Array(results).to_string())
}

/// Typed workflow ops — same semantics as the NATS bridge subjects.
async fn handle_workflow(
    State(shared): State<Shared>,
    Path(op): Path<String>,
    body: String,
) -> (StatusCode, String) {
    let body: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let run_id = body.get("run_id").and_then(|v| v.as_str()).unwrap_or("");

    // Output checkpoints (captured_files) stay server-side — strip from
    // bridge responses.
    fn strip_checkpoint(mut v: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = v.as_object_mut() {
            obj.remove("captured_files");
        }
        v
    }

    let result: Result<Vec<serde_json::Value>, String> = match op.as_str() {
        "list" => {
            let project = body.get("project").and_then(|v| v.as_str()).unwrap_or("");
            let r = if project.is_empty() {
                shared.engine.repo().list_active().await
            } else {
                shared.engine.repo().list_runs(project).await
            };
            r.map(|runs| runs.iter()
                    .filter_map(|x| serde_json::to_value(x).ok())
                    .map(strip_checkpoint)
                    .collect())
                .map_err(|e| e.to_string())
        }
        "get" => shared.engine.repo().get_by_run_id(run_id).await
            .map(|o| o.into_iter()
                .filter_map(|x| serde_json::to_value(&x).ok())
                .map(strip_checkpoint)
                .collect())
            .map_err(|e| e.to_string()),
        "defs" => shared.engine.repo().list_defs().await
            .map(|d| d.iter().filter_map(|x| serde_json::to_value(x).ok()).collect())
            .map_err(|e| e.to_string()),
        "run-from-def" => {
            let project = body.get("project").and_then(|v| v.as_str()).unwrap_or("");
            let goal = body.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let def_name = body.get("def_name").and_then(|v| v.as_str()).unwrap_or("");
            let files: Vec<String> = body.get("files")
                .and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
            if project.is_empty() || goal.is_empty() || def_name.is_empty() {
                Err("project, goal, def_name required".to_string())
            } else {
                shared.engine.create_from_def(shared.store.as_ref(), project, goal, def_name, files).await
                    .map(|run_id| vec![serde_json::json!({ "run_id": run_id })])
                    .map_err(|e| e.to_string())
            }
        }
        "pause" if !run_id.is_empty() => {
            shared.engine.control_for(run_id).await.request_pause();
            Ok(vec![serde_json::json!({ "requested": "pause", "run_id": run_id })])
        }
        "resume" if !run_id.is_empty() => {
            shared.engine.control_for(run_id).await.resume();
            let q = format!(
                "UPDATE type::thing('agent_run', '{}') SET status = 'approved', progress_message = 'Workflow resume requested' WHERE status = 'paused'",
                run_id.replace('\'', ""),
            );
            shared.store.db_query_raw(&q).await
                .map(|_| vec![serde_json::json!({ "requested": "resume", "run_id": run_id })])
                .map_err(|e| e.to_string())
        }
        "cancel" if !run_id.is_empty() => {
            shared.engine.control_for(run_id).await.request_cancel();
            let q = format!(
                "UPDATE workflow_run SET state = 'cancelled', updated_at = time::now() WHERE run_id = '{}' AND state = 'paused'",
                run_id.replace('\'', ""),
            );
            let _ = shared.store.db_query_raw(&q).await;
            Ok(vec![serde_json::json!({ "requested": "cancel", "run_id": run_id })])
        }
        other => Err(format!("unknown workflow op: {other}")),
    };

    match result {
        Ok(rows) => (
            StatusCode::OK,
            serde_json::json!({ "ok": true, "rows": rows, "truncated": false }).to_string(),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            serde_json::json!({ "ok": false, "rows": [], "error": e }).to_string(),
        ),
    }
}
