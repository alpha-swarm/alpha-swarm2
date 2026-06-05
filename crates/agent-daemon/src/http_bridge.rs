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
        // Serve the Leptos dashboard bundle for any non-API path (so the daemon
        // is the single endpoint: API + UI on the same origin → no CORS).
        .fallback_service(tower_http::services::ServeDir::new(DASHBOARD_DIR))
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
