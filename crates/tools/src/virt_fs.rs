//! WASI-portable file tools using FileProvider (zero-disk).

use serde_json::{json, Value};
use crate::{Tool, ToolContext, ToolResult};

const MAX_READ_BYTES: usize = 100_000;

pub struct VirtReadFileTool;

#[async_trait::async_trait]
impl Tool for VirtReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read file contents (from blobstore or disk)" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("Missing 'path' parameter", 0);
        }

        // Try FileProvider first (zero-disk), fall back to std::fs
        if let Some(ref fp) = ctx.file_provider {
            if let Ok(fp) = fp.lock() {
                match fp.read_file(path) {
                    Ok(content) => {
                        if content.len() > MAX_READ_BYTES {
                            return ToolResult::ok(format!("{}... (truncated)", &content[..MAX_READ_BYTES]), 0);
                        }
                        return ToolResult::ok(content, 0);
                    }
                    Err(_) => {} // Fall through to disk
                }
            }
        }

        // Disk fallback
        let full = ctx.repo_path.join(path);
        match std::fs::read_to_string(&full) {
            Ok(content) => {
                if content.len() > MAX_READ_BYTES {
                    ToolResult::ok(format!("{}... (truncated)", &content[..MAX_READ_BYTES]), 0)
                } else {
                    ToolResult::ok(content, 0)
                }
            }
            Err(e) => ToolResult::err(format!("Cannot read {path}: {e}"), 0),
        }
    }
}

pub struct VirtWriteFileTool;

#[async_trait::async_trait]
impl Tool for VirtWriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let content = params.get("content").and_then(|c| c.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("Missing 'path' parameter", 0);
        }

        if let Some(ref fp) = ctx.file_provider {
            if let Ok(mut fp) = fp.lock() {
                match fp.write_file(path, content) {
                    Ok(()) => return ToolResult::ok(format!("Written {} bytes to {path}", content.len()), 0),
                    Err(_) => {}
                }
            }
        }

        // Disk fallback
        let full = ctx.repo_path.join(path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&full, content) {
            Ok(()) => ToolResult::ok(format!("Written {} bytes to {path}", content.len()), 0),
            Err(e) => ToolResult::err(format!("Cannot write {path}: {e}"), 0),
        }
    }
}

pub struct VirtListFilesTool;

#[async_trait::async_trait]
impl Tool for VirtListFilesTool {
    fn name(&self) -> &str { "list_files" }
    fn description(&self) -> &str { "List files in workspace" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"pattern": {"type": "string"}}})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let pattern = params.get("pattern").and_then(|p| p.as_str()).unwrap_or("");

        if let Some(ref fp) = ctx.file_provider {
            if let Ok(fp) = fp.lock() {
                let files = fp.list_files();
                let filtered: Vec<&String> = if pattern.is_empty() {
                    files.iter().collect()
                } else {
                    files.iter().filter(|f| f.contains(pattern)).collect()
                };
                return ToolResult::ok(filtered.iter().map(|f| f.as_str()).collect::<Vec<_>>().join("\n"), 0);
            }
        }

        ToolResult::err("No file provider available", 0)
    }
}
