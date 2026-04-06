use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn, debug};

use crate::{Tool, ToolContext, ToolResult};

/// Timeout for remote tool calls via NATS.
const REMOTE_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// Request payload sent over NATS to tool workers.
#[derive(Serialize)]
struct RemoteToolRequest {
    name: String,
    params_json: String,
    repo_path: String,
    project: String,
    timeout_ms: u64,
}

/// Response payload received from tool workers.
#[derive(Deserialize)]
struct RemoteToolResponse {
    content: String,
    is_error: bool,
    duration_ms: u64,
}

/// Dispatches tool calls to remote WASI workers via NATS request-reply.
pub struct NatsToolDispatcher {
    client: async_nats::Client,
    subject_prefix: String,
}

impl NatsToolDispatcher {
    pub fn new(client: async_nats::Client, subject_prefix: impl Into<String>) -> Self {
        Self {
            client,
            subject_prefix: subject_prefix.into(),
        }
    }

    /// Call a remote tool worker via NATS request-reply.
    pub async fn call(&self, name: &str, params: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let subject = format!("{}.{}", self.subject_prefix, name);

        let request = RemoteToolRequest {
            name: name.to_string(),
            params_json: serde_json::to_string(&params).unwrap_or_default(),
            repo_path: ctx.repo_path.to_string_lossy().to_string(),
            project: ctx.project.clone(),
            timeout_ms: ctx.timeout.as_millis() as u64,
        };

        let payload = serde_json::to_vec(&request).map_err(|e| format!("serialize: {e}"))?;
        let start = Instant::now();

        debug!(subject = %subject, "Dispatching tool call via NATS");

        let reply = tokio::time::timeout(REMOTE_TOOL_TIMEOUT, self.client.request(subject.clone(), payload.into()))
            .await
            .map_err(|_| format!("NATS tool call timeout after {:?}", REMOTE_TOOL_TIMEOUT))?
            .map_err(|e| format!("NATS request failed: {e}"))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        let response: RemoteToolResponse = serde_json::from_slice(&reply.payload)
            .map_err(|e| format!("deserialize response: {e}"))?;

        info!(tool = name, remote_duration_ms = response.duration_ms, total_duration_ms = duration_ms, "Remote tool call completed");

        Ok(ToolResult {
            content: response.content,
            is_error: response.is_error,
            duration_ms,
        })
    }
}
