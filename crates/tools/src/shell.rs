use std::process::Command;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

const MAX_OUTPUT_BYTES: usize = 10_000;

/// Allowlisted commands that are safe to run.
const ALLOWED_COMMANDS: &[&str] = &[
    "cargo", "rustc", "rustfmt", "clippy-driver",
    "npm", "npx", "node",
    "go", "gofmt",
    "python3", "pip3",
    "wc", "sort", "uniq", "head", "tail", "cat",
    "ls", "find", "tree",
];

pub struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str { "run_command" }
    fn description(&self) -> &str { "Run a shell command (allowlisted: cargo, npm, go, python3, etc.)" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The command to run"},
                "args": {"type": "array", "items": {"type": "string"}, "description": "Command arguments"}
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let command = params.get("command").and_then(|c| c.as_str()).unwrap_or("");
        if command.is_empty() {
            return ToolResult::err("Missing 'command' parameter", 0);
        }

        if !ALLOWED_COMMANDS.contains(&command) {
            return ToolResult::err(
                format!("Command '{command}' not in allowlist. Allowed: {}", ALLOWED_COMMANDS.join(", ")),
                0,
            );
        }

        let args: Vec<String> = params.get("args")
            .and_then(|a| a.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        match Command::new(command).args(&args).current_dir(&ctx.repo_path).output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = format!("{stdout}{stderr}");
                let truncated = if combined.len() > MAX_OUTPUT_BYTES {
                    format!("{}...\n(truncated)", &combined[..MAX_OUTPUT_BYTES])
                } else {
                    combined
                };
                let exit = out.status.code().unwrap_or(-1);
                if out.status.success() {
                    ToolResult::ok(format!("exit {exit}\n{truncated}"), 0)
                } else {
                    ToolResult::err(format!("exit {exit}\n{truncated}"), 0)
                }
            }
            Err(e) => ToolResult::err(format!("Failed to run {command}: {e}"), 0),
        }
    }
}
