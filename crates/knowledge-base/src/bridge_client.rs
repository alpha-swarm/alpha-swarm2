//! NATS DB-bridge client for native consumers (TUI, eval, future remote
//! daemons in `mode = nats`). The agent-daemon serves these subjects — see
//! `agent-daemon/src/db_bridge.rs`. Subject constants are canonical HERE so
//! server and clients cannot drift.

use anyhow::{Context, Result};
use std::time::Duration;

/// Read-only SurrealQL (SELECT) over the bridge.
pub const DB_QUERY_SUBJECT: &str = "swarm.db.query";
/// Write SurrealQL (verb allowlist) over the bridge.
pub const DB_EXEC_SUBJECT: &str = "swarm.db.exec";
/// Typed workflow ops prefix (`list/get/pause/resume/cancel/defs`).
pub const DB_WORKFLOW_SUBJECT_PREFIX: &str = "swarm.db.workflow.";
/// Typed memory ops prefix (`store/search/recall/decay/stats`).
pub const DB_MEMORY_SUBJECT_PREFIX: &str = "swarm.db.memory.";
/// Typed code-graph ops prefix (`build/entity/relations/neighbors`).
pub const DB_GRAPH_SUBJECT_PREFIX: &str = "swarm.db.graph.";
/// Default request-reply timeout for bridge calls.
pub const DEFAULT_BRIDGE_TIMEOUT_SECS: u64 = 30;

/// Thin request-reply client for the daemon's DB bridge.
pub struct NatsDbClient {
    client: async_nats::Client,
    timeout: Duration,
}

impl NatsDbClient {
    pub async fn connect(nats_url: &str) -> Result<Self> {
        let client = async_nats::connect(nats_url).await
            .context("NATS connect for DB bridge failed")?;
        Ok(Self { client, timeout: Duration::from_secs(DEFAULT_BRIDGE_TIMEOUT_SECS) })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Route a statement to query/exec by its leading verb.
    fn subject_for(sql: &str) -> &'static str {
        let verb = sql.trim_start().split_whitespace().next().unwrap_or("").to_uppercase();
        if verb == "SELECT" { DB_QUERY_SUBJECT } else { DB_EXEC_SUBJECT }
    }

    async fn request_raw(&self, subject: &str, payload: serde_json::Value) -> Result<serde_json::Value> {
        let response = tokio::time::timeout(
            self.timeout,
            self.client.request(subject.to_string(), payload.to_string().into()),
        ).await.context("bridge request timed out")?
            .context("bridge request failed")?;
        serde_json::from_slice(&response.payload).context("bridge response parse failed")
    }

    /// Execute SurrealQL over the bridge; returns the rows of the first
    /// statement. Errors carry the bridge-side message.
    pub async fn query(&self, sql: &str, params: serde_json::Value) -> Result<Vec<serde_json::Value>> {
        let parsed = self.request_raw(
            Self::subject_for(sql),
            serde_json::json!({ "query": sql, "params": params }),
        ).await?;
        if parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(parsed.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default())
        } else {
            anyhow::bail!(
                "bridge error: {}",
                parsed.get("error").and_then(|e| e.as_str()).unwrap_or("unknown")
            )
        }
    }

    /// Invoke a typed workflow op (`list/get/pause/resume/cancel/defs`).
    pub async fn workflow_op(&self, op: &str, body: serde_json::Value) -> Result<Vec<serde_json::Value>> {
        let parsed = self.request_raw(&format!("{DB_WORKFLOW_SUBJECT_PREFIX}{op}"), body).await?;
        if parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(parsed.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default())
        } else {
            anyhow::bail!(
                "bridge error: {}",
                parsed.get("error").and_then(|e| e.as_str()).unwrap_or("unknown")
            )
        }
    }

    /// Invoke a typed memory op (`store/search/recall/decay/stats`).
    pub async fn memory_op(&self, op: &str, body: serde_json::Value) -> Result<Vec<serde_json::Value>> {
        let parsed = self.request_raw(&format!("{DB_MEMORY_SUBJECT_PREFIX}{op}"), body).await?;
        if parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(parsed.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default())
        } else {
            anyhow::bail!(
                "bridge error: {}",
                parsed.get("error").and_then(|e| e.as_str()).unwrap_or("unknown")
            )
        }
    }

    /// Invoke a typed code-graph op (`build/entity/relations/neighbors`).
    pub async fn graph_op(&self, op: &str, body: serde_json::Value) -> Result<Vec<serde_json::Value>> {
        let parsed = self.request_raw(&format!("{DB_GRAPH_SUBJECT_PREFIX}{op}"), body).await?;
        if parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(parsed.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default())
        } else {
            anyhow::bail!(
                "bridge error: {}",
                parsed.get("error").and_then(|e| e.as_str()).unwrap_or("unknown")
            )
        }
    }
}
