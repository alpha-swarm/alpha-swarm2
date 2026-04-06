use std::process::Command;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

const MAX_MATCHES: usize = 100;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { "Search for a pattern in files (uses ripgrep if available, else grep -rn)" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regex pattern to search for"},
                "path": {"type": "string", "description": "Directory or file to search in (default: repo root)"},
                "glob": {"type": "string", "description": "File glob filter (e.g., '*.rs')"}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let pattern = params.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
        if pattern.is_empty() {
            return ToolResult::err("Missing 'pattern' parameter", 0);
        }
        let search_path = params.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let glob_filter = params.get("glob").and_then(|g| g.as_str());

        let full_path = ctx.repo_path.join(search_path);

        // Try ripgrep first (faster), fallback to grep
        let output = if Command::new("rg").arg("--version").output().is_ok() {
            let mut cmd = Command::new("rg");
            cmd.args(["-n", "--max-count", &MAX_MATCHES.to_string(), pattern])
                .current_dir(&full_path);
            if let Some(g) = glob_filter {
                cmd.args(["--glob", g]);
            }
            cmd.output()
        } else {
            let mut cmd = Command::new("grep");
            cmd.args(["-rn", "--max-count", &MAX_MATCHES.to_string(), pattern, "."])
                .current_dir(&full_path);
            cmd.output()
        };

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.is_empty() {
                    ToolResult::ok("No matches found", 0)
                } else {
                    ToolResult::ok(stdout.to_string(), 0)
                }
            }
            Err(e) => ToolResult::err(format!("grep failed: {e}"), 0),
        }
    }
}
