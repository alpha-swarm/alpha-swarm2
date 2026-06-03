//! NATS request-reply DB bridge — the single query surface for everything
//! that is not the daemon itself (WASM components, TUI, eval, future remote
//! daemons). The local daemon is the sole DB owner; consumers never open a
//! SurrealDB connection.
//!
//! Subjects (one responder, queue group `db-bridge`):
//!   swarm.db.query        — read-only SurrealQL (SELECT)
//!   swarm.db.exec         — writes (verb allowlist)
//!   swarm.db.workflow.*   — typed workflow ops (list/get/pause/resume/cancel/defs)
//!   swarm.db.memory.*     — typed memory ops (store/search/recall/decay/stats)
//!
//! `swarm.db.>` is request-reply RPC, deliberately distinct from the
//! `alpha-swarm.{project}.>` pub-sub event bus.

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use knowledge_base::{KnowledgeBackend, MemoryEntry, MemoryStore};
// Canonical subject constants live in knowledge-base so server and clients
// cannot drift.
use knowledge_base::bridge_client::{DB_EXEC_SUBJECT, DB_QUERY_SUBJECT};
use swarm_workflow::WorkflowEngine;

/// Wildcard the bridge subscribes to.
const DB_BRIDGE_WILDCARD: &str = "swarm.db.>";
/// Queue group: exactly one daemon answers even if several connect.
const BRIDGE_QUEUE_GROUP: &str = "db-bridge";
/// Response payload guard below NATS' 1 MB default max payload.
const MAX_RESPONSE_BYTES: usize = 900_000;
/// Leading verbs allowed on `swarm.db.exec` statements.
const EXEC_ALLOWED_VERBS: &[&str] = &["SELECT", "CREATE", "UPDATE", "UPSERT", "DELETE", "RELATE", "INSERT"];
/// Substrings rejected anywhere in bridged SQL (admin/ddl/session control).
const FORBIDDEN_FRAGMENTS: &[&str] = &["DEFINE USER", "REMOVE ", "INFO FOR", "KILL ", "USE NS", "USE DB", "DEFINE ACCESS"];

#[derive(Debug, Deserialize)]
struct DbRequest {
    query: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct DbResponse {
    ok: bool,
    rows: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    truncated: bool,
}

impl DbResponse {
    fn ok(rows: Vec<serde_json::Value>) -> Self {
        Self { ok: true, rows: serde_json::Value::Array(rows), error: None, truncated: false }
    }
    fn err(error: impl Into<String>) -> Self {
        Self { ok: false, rows: serde_json::Value::Array(vec![]), error: Some(error.into()), truncated: false }
    }

    /// Serialize. A result that exceeds the payload guard is a HARD ERROR —
    /// never silently truncated. Partial-but-OK data reads as complete and is
    /// the worst failure mode for a dashboard; a loud error tells the caller
    /// to add a LIMIT or narrow the query.
    fn to_bytes(self) -> Vec<u8> {
        let bytes = serde_json::to_vec(&self).unwrap_or_default();
        if bytes.len() <= MAX_RESPONSE_BYTES {
            return bytes;
        }
        let n = self.rows.as_array().map(|a| a.len()).unwrap_or(0);
        warn!(bytes = bytes.len(), rows = n, max = MAX_RESPONSE_BYTES, "bridge response over cap — rejecting (add LIMIT)");
        serde_json::to_vec(&DbResponse::err(format!(
            "result {} bytes ({} rows) exceeds {}-byte cap — add a LIMIT or narrow the query",
            bytes.len(), n, MAX_RESPONSE_BYTES
        ))).unwrap_or_default()
    }
}

/// Validate bridged SQL. `read_only` restricts to a single SELECT.
fn validate_sql(query: &str, read_only: bool) -> Result<(), String> {
    let upper = query.to_uppercase();
    for frag in FORBIDDEN_FRAGMENTS {
        if upper.contains(frag) {
            return Err(format!("forbidden SQL fragment: {frag}"));
        }
    }
    // Per-statement leading-verb check.
    for stmt in upper.split(';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        let verb = stmt.split_whitespace().next().unwrap_or("");
        if read_only {
            if verb != "SELECT" {
                return Err(format!("swarm.db.query is read-only; got verb {verb}"));
            }
        } else if !EXEC_ALLOWED_VERBS.contains(&verb) {
            return Err(format!("verb not allowed on swarm.db.exec: {verb}"));
        }
    }
    Ok(())
}

/// Shared state for the bridge responder.
pub struct DbBridge {
    store: Arc<dyn KnowledgeBackend>,
    engine: Arc<WorkflowEngine>,
    memory: Arc<MemoryStore>,
}

impl DbBridge {
    pub fn new(store: Arc<dyn KnowledgeBackend>, engine: Arc<WorkflowEngine>, memory: Arc<MemoryStore>) -> Self {
        Self { store, engine, memory }
    }

    /// Subscribe and serve until the NATS connection dies. Spawn me.
    pub async fn serve(self, client: async_nats::Client) {
        let mut sub = match client
            .queue_subscribe(DB_BRIDGE_WILDCARD, BRIDGE_QUEUE_GROUP.into())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "DB bridge subscribe failed — bridge unavailable");
                return;
            }
        };
        info!(subject = DB_BRIDGE_WILDCARD, queue = BRIDGE_QUEUE_GROUP, "DB bridge listening");

        while let Some(msg) = sub.next().await {
            let Some(reply_to) = msg.reply.clone() else { continue };
            let subject = msg.subject.to_string();
            let response = self.dispatch(&subject, &msg.payload).await;
            let _ = client.publish(reply_to, response.to_bytes().into()).await;
        }
        warn!("DB bridge subscription ended");
    }

    async fn dispatch(&self, subject: &str, payload: &[u8]) -> DbResponse {
        match subject {
            DB_QUERY_SUBJECT => self.handle_sql(payload, true).await,
            DB_EXEC_SUBJECT => self.handle_sql(payload, false).await,
            s if s.starts_with("swarm.db.workflow.") => {
                self.handle_workflow(s.trim_start_matches("swarm.db.workflow."), payload).await
            }
            s if s.starts_with("swarm.db.memory.") => {
                self.handle_memory(s.trim_start_matches("swarm.db.memory."), payload).await
            }
            s if s.starts_with("swarm.db.autopilot.") => {
                self.handle_autopilot(s.trim_start_matches("swarm.db.autopilot."), payload).await
            }
            other => DbResponse::err(format!("unknown bridge subject: {other}")),
        }
    }

    /// Autopilot backlog ops: `queue` (enqueue a goal), `list` (show backlog).
    async fn handle_autopilot(&self, op: &str, payload: &[u8]) -> DbResponse {
        let body: serde_json::Value = serde_json::from_slice(payload).unwrap_or(serde_json::Value::Null);
        match op {
            "queue" => {
                let project = body.get("project").and_then(|v| v.as_str()).unwrap_or("");
                let goal = body.get("goal").and_then(|v| v.as_str()).unwrap_or("");
                if project.is_empty() || goal.is_empty() {
                    return DbResponse::err("project, goal required");
                }
                let q = "CREATE autopilot_goal CONTENT $data RETURN id";
                match self.store.query_json(q, serde_json::json!({
                    "data": { "project": project, "goal": goal, "status": "queued",
                              "created_at": chrono::Utc::now().to_rfc3339() }
                })).await {
                    Ok(rows) => DbResponse::ok(rows),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            "list" => {
                match self.store.query_json(
                    "SELECT * FROM autopilot_goal ORDER BY created_at DESC LIMIT 50",
                    serde_json::Value::Null,
                ).await {
                    Ok(rows) => DbResponse::ok(rows),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            other => DbResponse::err(format!("unknown autopilot op: {other}")),
        }
    }

    async fn handle_sql(&self, payload: &[u8], read_only: bool) -> DbResponse {
        let req: DbRequest = match serde_json::from_slice(payload) {
            Ok(r) => r,
            Err(e) => return DbResponse::err(format!("invalid request: {e}")),
        };
        if let Err(e) = validate_sql(&req.query, read_only) {
            return DbResponse::err(e);
        }
        match self.store.query_json(&req.query, req.params).await {
            Ok(rows) => DbResponse::ok(rows),
            Err(e) => DbResponse::err(format!("query failed: {e}")),
        }
    }

    /// Drop the (potentially multi-MB) output checkpoint from a workflow_run
    /// JSON before it crosses the bridge — consumers want states, not blobs.
    fn strip_checkpoint(mut v: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = v.as_object_mut() {
            obj.remove("captured_files");
        }
        v
    }

    async fn handle_workflow(&self, op: &str, payload: &[u8]) -> DbResponse {
        let body: serde_json::Value = serde_json::from_slice(payload).unwrap_or(serde_json::Value::Null);
        let run_id = body.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
        match op {
            "list" => {
                let project = body.get("project").and_then(|v| v.as_str()).unwrap_or("");
                let result = if project.is_empty() {
                    self.engine.repo().list_active().await
                } else {
                    self.engine.repo().list_runs(project).await
                };
                match result {
                    Ok(runs) => DbResponse::ok(runs.iter()
                        .filter_map(|r| serde_json::to_value(r).ok())
                        .map(Self::strip_checkpoint)
                        .collect()),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            "get" => match self.engine.repo().get_by_run_id(run_id).await {
                Ok(Some(run)) => DbResponse::ok(vec![Self::strip_checkpoint(serde_json::to_value(&run).unwrap_or_default())]),
                Ok(None) => DbResponse::ok(vec![]),
                Err(e) => DbResponse::err(e.to_string()),
            },
            "pause" => {
                if run_id.is_empty() { return DbResponse::err("run_id required"); }
                self.engine.control_for(run_id).await.request_pause();
                DbResponse::ok(vec![serde_json::json!({ "requested": "pause", "run_id": run_id })])
            }
            "resume" => {
                if run_id.is_empty() { return DbResponse::err("run_id required"); }
                self.engine.control_for(run_id).await.resume();
                // Requeue the agent_run so the scheduler picks it up; the
                // executor resumes the persisted workflow_run (no re-plan).
                let q = format!(
                    "UPDATE type::thing('agent_run', '{}') SET status = 'approved', progress_message = 'Workflow resume requested' WHERE status = 'paused'",
                    run_id.replace('\'', ""),
                );
                match self.store.db_query_raw(&q).await {
                    Ok(_) => DbResponse::ok(vec![serde_json::json!({ "requested": "resume", "run_id": run_id })]),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            "cancel" => {
                if run_id.is_empty() { return DbResponse::err("run_id required"); }
                self.engine.control_for(run_id).await.request_cancel();
                // Dormant (paused) runs are cancelled directly; live runs are
                // cancelled cooperatively by the engine between waves.
                let q = format!(
                    "UPDATE workflow_run SET state = 'cancelled', updated_at = time::now() WHERE run_id = '{}' AND state = 'paused'",
                    run_id.replace('\'', ""),
                );
                let _ = self.store.db_query_raw(&q).await;
                DbResponse::ok(vec![serde_json::json!({ "requested": "cancel", "run_id": run_id })])
            }
            "defs" => match self.engine.repo().list_defs().await {
                Ok(defs) => DbResponse::ok(defs.iter().filter_map(|d| serde_json::to_value(d).ok()).collect()),
                Err(e) => DbResponse::err(e.to_string()),
            },
            "run-from-def" => {
                let project = body.get("project").and_then(|v| v.as_str()).unwrap_or("");
                let goal = body.get("goal").and_then(|v| v.as_str()).unwrap_or("");
                let def_name = body.get("def_name").and_then(|v| v.as_str()).unwrap_or("");
                let files: Vec<String> = body.get("files")
                    .and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                if project.is_empty() || goal.is_empty() || def_name.is_empty() {
                    return DbResponse::err("project, goal, def_name required");
                }
                match self.engine.create_from_def(self.store.as_ref(), project, goal, def_name, files).await {
                    Ok(run_id) => DbResponse::ok(vec![serde_json::json!({ "run_id": run_id })]),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            other => DbResponse::err(format!("unknown workflow op: {other}")),
        }
    }

    async fn handle_memory(&self, op: &str, payload: &[u8]) -> DbResponse {
        let body: serde_json::Value = serde_json::from_slice(payload).unwrap_or(serde_json::Value::Null);
        let project = body.get("project").and_then(|v| v.as_str()).unwrap_or("");
        match op {
            "store" => {
                let entry: MemoryEntry = match serde_json::from_value(body) {
                    Ok(e) => e,
                    Err(e) => return DbResponse::err(format!("invalid memory entry: {e}")),
                };
                match self.memory.store(entry).await {
                    Ok(id) => DbResponse::ok(vec![serde_json::json!({ "id": id })]),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            "search" => {
                let namespaces: Vec<String> = body.get("namespaces")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let ns_refs: Vec<&str> = namespaces.iter().map(|s| s.as_str()).collect();
                let query = body.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let top_k = body.get("top_k").and_then(|v| v.as_u64())
                    .unwrap_or(knowledge_base::memory::DEFAULT_TOP_K as u64) as usize;
                match self.memory.search_text(&ns_refs, project, query, top_k).await {
                    Ok(hits) => DbResponse::ok(hits.iter().filter_map(|h| {
                        // Strip embeddings from bridge responses (size).
                        serde_json::to_value(h).ok().map(|mut v| {
                            if let Some(entry) = v.get_mut("entry").and_then(|e| e.as_object_mut()) {
                                entry.remove("embedding");
                            }
                            v
                        })
                    }).collect()),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            "recall" => {
                let namespace = body.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                let key = body.get("key").and_then(|v| v.as_str()).unwrap_or("");
                match self.memory.recall(namespace, project, key).await {
                    Ok(Some(entry)) => DbResponse::ok(vec![serde_json::to_value(&entry).unwrap_or_default()]),
                    Ok(None) => DbResponse::ok(vec![]),
                    Err(e) => DbResponse::err(e.to_string()),
                }
            }
            "decay" => match self.memory.decay(project).await {
                Ok(pruned) => DbResponse::ok(vec![serde_json::json!({ "pruned": pruned })]),
                Err(e) => DbResponse::err(e.to_string()),
            },
            "stats" => match self.memory.pattern_hit_rate(project).await {
                Ok(stats) => DbResponse::ok(vec![stats]),
                Err(e) => DbResponse::err(e.to_string()),
            },
            other => DbResponse::err(format!("unknown memory op: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_subject_is_read_only() {
        assert!(validate_sql("SELECT * FROM agent_run", true).is_ok());
        assert!(validate_sql("DELETE FROM agent_run", true).is_err());
        assert!(validate_sql("UPDATE agent_run SET x = 1", true).is_err());
    }

    #[test]
    fn exec_allows_crud_only() {
        assert!(validate_sql("CREATE project SET name = 'x'", false).is_ok());
        assert!(validate_sql("UPDATE agent_run SET status = 'approved'", false).is_ok());
        assert!(validate_sql("DELETE FROM agent_run", false).is_ok());
        assert!(validate_sql("DEFINE TABLE hax SCHEMALESS", false).is_err());
        assert!(validate_sql("KILL 'uuid'", false).is_err());
    }

    #[test]
    fn forbidden_fragments_rejected_everywhere() {
        assert!(validate_sql("SELECT * FROM x; USE NS other", true).is_err());
        assert!(validate_sql("CREATE x; REMOVE TABLE agent_run", false).is_err());
        assert!(validate_sql("SELECT * FROM x WHERE n = 'DEFINE USER'", true).is_err());
    }

    #[test]
    fn multi_statement_exec_checks_each_verb() {
        assert!(validate_sql("CREATE a SET x = 1; UPDATE b SET y = 2;", false).is_ok());
        assert!(validate_sql("CREATE a SET x = 1; SLEEP 10s;", false).is_err());
    }

    #[test]
    fn oversized_response_fails_loud() {
        let big_row = serde_json::json!({ "data": "x".repeat(100_000) });
        let rows = vec![big_row; 20]; // ~2MB
        let bytes = DbResponse::ok(rows).to_bytes();
        assert!(bytes.len() <= MAX_RESPONSE_BYTES);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        // Hard error, NOT partial data.
        assert_eq!(parsed["ok"], serde_json::Value::Bool(false));
        assert!(parsed["error"].as_str().unwrap().contains("exceeds"));
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn normal_response_passes_through() {
        let bytes = DbResponse::ok(vec![serde_json::json!({"x": 1})]).to_bytes();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["ok"], serde_json::Value::Bool(true));
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
    }
}
