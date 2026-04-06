use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

/// Maximum file size to read (to avoid feeding huge files to the model).
const MAX_READ_BYTES: usize = 100_000;

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the contents of a file" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("Missing 'path' parameter", 0);
        }
        let full = ctx.repo_path.join(path);
        match std::fs::read_to_string(&full) {
            Ok(content) => {
                if content.len() > MAX_READ_BYTES {
                    ToolResult::ok(format!("{}\n... (truncated, {} bytes total)", &content[..MAX_READ_BYTES], content.len()), 0)
                } else {
                    ToolResult::ok(content, 0)
                }
            }
            Err(e) => ToolResult::err(format!("Cannot read {path}: {e}"), 0),
        }
    }
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file (creates parent directories)" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let content = params.get("content").and_then(|c| c.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("Missing 'path' parameter", 0);
        }
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

pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &str { "delete_file" }
    fn description(&self) -> &str { "Delete a file" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("Missing 'path' parameter", 0);
        }
        let full = ctx.repo_path.join(path);
        match std::fs::remove_file(&full) {
            Ok(()) => ToolResult::ok(format!("Deleted {path}"), 0),
            Err(e) => ToolResult::err(format!("Cannot delete {path}: {e}"), 0),
        }
    }
}

pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str { "list_files" }
    fn description(&self) -> &str { "List files matching a glob pattern (e.g., 'src/**/*.rs')" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"pattern": {"type": "string"}}, "required": ["pattern"]})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let pattern = params.get("pattern").and_then(|p| p.as_str()).unwrap_or("**/*");
        let full_pattern = ctx.repo_path.join(pattern).to_string_lossy().to_string();

        let mut files = Vec::new();
        const MAX_FILES: usize = 500;
        match glob::glob(&full_pattern) {
            Ok(paths) => {
                for entry in paths.flatten() {
                    if files.len() >= MAX_FILES { break; }
                    if let Ok(rel) = entry.strip_prefix(&ctx.repo_path) {
                        files.push(rel.to_string_lossy().to_string());
                    }
                }
                ToolResult::ok(files.join("\n"), 0)
            }
            Err(e) => ToolResult::err(format!("Invalid glob pattern: {e}"), 0),
        }
    }
}
