//! Agent-facing memory tools: let the LLM recall past solutions and store
//! learnings mid-task. Talk to the daemon's DB bridge over NATS
//! (`swarm.db.memory.*`) — NO knowledge-base/surrealdb dependency, keeping
//! this crate WASI-portable (tools are native-gated, bridge is reached by
//! request-reply only).

use serde_json::Value;

use crate::{Tool, ToolContext, ToolResult};

/// Canonical memory-op subject prefix (see knowledge-base bridge_client).
const DB_MEMORY_SUBJECT_PREFIX: &str = "swarm.db.memory.";
/// Request-reply timeout for memory ops.
const MEMORY_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Default hits returned by memory_recall.
const RECALL_TOP_K: u64 = 3;
/// Namespaces an agent may write to.
const AGENT_WRITABLE_NAMESPACES: &[&str] = &["solutions", "errors"];

async fn memory_request(
    client: &async_nats::Client,
    op: &str,
    body: Value,
) -> Result<Value, String> {
    let subject = format!("{DB_MEMORY_SUBJECT_PREFIX}{op}");
    let response = tokio::time::timeout(
        MEMORY_TOOL_TIMEOUT,
        client.request(subject, body.to_string().into()),
    )
    .await
    .map_err(|_| "memory bridge timed out".to_string())?
    .map_err(|e| format!("memory bridge: {e}"))?;
    let parsed: Value = serde_json::from_slice(&response.payload)
        .map_err(|e| format!("memory bridge json: {e}"))?;
    if parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(parsed.get("rows").cloned().unwrap_or(Value::Array(vec![])))
    } else {
        Err(parsed.get("error").and_then(|e| e.as_str()).unwrap_or("unknown").to_string())
    }
}

/// Semantic recall from agent memory (patterns/solutions/errors).
pub struct MemoryRecallTool {
    client: async_nats::Client,
}

impl MemoryRecallTool {
    pub fn new(client: async_nats::Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Semantic search over this project's agent memory (past proven solutions, distilled patterns, known errors). Use before attempting something that may have been solved before."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "What you are trying to do or the error you hit" },
                "namespaces": {
                    "type": "array", "items": { "type": "string" },
                    "description": "Memory namespaces to search (default: patterns, solutions, errors)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let query = params.get("query").and_then(|q| q.as_str()).unwrap_or("");
        if query.is_empty() {
            return ToolResult::err("query is required", 0);
        }
        let namespaces = params.get("namespaces").cloned()
            .unwrap_or_else(|| serde_json::json!(["patterns", "solutions", "errors"]));

        match memory_request(&self.client, "search", serde_json::json!({
            "namespaces": namespaces,
            "project": ctx.project,
            "query": query,
            "top_k": RECALL_TOP_K,
        })).await {
            Ok(rows) => {
                let hits = rows.as_array().cloned().unwrap_or_default();
                if hits.is_empty() {
                    return ToolResult::ok("No relevant memories found.", start.elapsed().as_millis() as u64);
                }
                let rendered: Vec<String> = hits.iter().filter_map(|h| {
                    let entry = h.get("entry")?;
                    let ns = entry.get("namespace")?.as_str()?;
                    let content = entry.get("content")?.as_str()?;
                    let sim = h.get("similarity").and_then(|s| s.as_f64()).unwrap_or(0.0);
                    Some(format!("[{ns} | sim {sim:.2}]\n{content}"))
                }).collect();
                ToolResult::ok(rendered.join("\n---\n"), start.elapsed().as_millis() as u64)
            }
            Err(e) => ToolResult::err(e, start.elapsed().as_millis() as u64),
        }
    }
}

/// Store a learning into agent memory.
pub struct MemoryStoreTool {
    client: async_nats::Client,
}

impl MemoryStoreTool {
    pub fn new(client: async_nats::Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Store a reusable learning into agent memory. namespace 'solutions' for working approaches, 'errors' for pitfalls. Keep content compact (what worked / what to avoid, and why)."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "namespace": { "type": "string", "enum": AGENT_WRITABLE_NAMESPACES },
                "key": { "type": "string", "description": "Stable short slug for this learning" },
                "content": { "type": "string", "description": "The learning itself (compact)" }
            },
            "required": ["namespace", "key", "content"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let namespace = params.get("namespace").and_then(|n| n.as_str()).unwrap_or("");
        let key = params.get("key").and_then(|k| k.as_str()).unwrap_or("");
        let content = params.get("content").and_then(|c| c.as_str()).unwrap_or("");
        if !AGENT_WRITABLE_NAMESPACES.contains(&namespace) {
            return ToolResult::err(
                format!("namespace must be one of {AGENT_WRITABLE_NAMESPACES:?}"), 0,
            );
        }
        if key.is_empty() || content.is_empty() {
            return ToolResult::err("key and content are required", 0);
        }

        match memory_request(&self.client, "store", serde_json::json!({
            "namespace": namespace,
            "key": key,
            "content": content,
            "metadata": { "source": "agent-tool" },
            "project": ctx.project,
            "created_at": "",
            "last_used_at": "",
            "use_count": 0,
        })).await {
            Ok(rows) => {
                let id = rows.get(0).and_then(|r| r.get("id")).and_then(|i| i.as_str()).unwrap_or("?");
                ToolResult::ok(format!("Stored memory {id}"), start.elapsed().as_millis() as u64)
            }
            Err(e) => ToolResult::err(e, start.elapsed().as_millis() as u64),
        }
    }
}
