use std::process::Command;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

const MAX_DIFF_BYTES: usize = 20_000;

pub struct GitDiffTool;

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str { "git_diff" }
    fn description(&self) -> &str { "Show current uncommitted changes (git diff)" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string", "description": "Specific file to diff (optional)"}}})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str());
        let mut cmd = Command::new("git");
        cmd.arg("diff").current_dir(&ctx.repo_path);
        if let Some(p) = path { cmd.arg(p); }

        match cmd.output() {
            Ok(out) => {
                let diff = String::from_utf8_lossy(&out.stdout);
                if diff.is_empty() {
                    ToolResult::ok("No changes", 0)
                } else if diff.len() > MAX_DIFF_BYTES {
                    ToolResult::ok(format!("{}...\n(truncated, {} bytes total)", &diff[..MAX_DIFF_BYTES], diff.len()), 0)
                } else {
                    ToolResult::ok(diff.to_string(), 0)
                }
            }
            Err(e) => ToolResult::err(format!("git diff failed: {e}"), 0),
        }
    }
}

pub struct GitStatusTool;

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str { "git_status" }
    fn description(&self) -> &str { "Show working tree status (git status --short)" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: Value, ctx: &ToolContext) -> ToolResult {
        match Command::new("git").args(["status", "--short"]).current_dir(&ctx.repo_path).output() {
            Ok(out) => {
                let status = String::from_utf8_lossy(&out.stdout);
                if status.is_empty() {
                    ToolResult::ok("Working tree clean", 0)
                } else {
                    ToolResult::ok(status.to_string(), 0)
                }
            }
            Err(e) => ToolResult::err(format!("git status failed: {e}"), 0),
        }
    }
}
