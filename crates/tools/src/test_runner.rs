use std::process::Command;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

/// Maximum output from test commands to avoid flooding the model context.
const MAX_TEST_OUTPUT_BYTES: usize = 10_000;

pub struct RunTestsTool;

#[async_trait]
impl Tool for RunTestsTool {
    fn name(&self) -> &str { "run_tests" }
    fn description(&self) -> &str { "Run tests. Detects toolchain (cargo test, npm test, go test). Optional pattern to filter." }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Test name/pattern filter (optional)"}
            }
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let pattern = params.get("pattern").and_then(|p| p.as_str()).unwrap_or("");

        // Detect toolchain
        let (cmd, args) = if ctx.repo_path.join("Cargo.toml").exists() {
            if pattern.is_empty() {
                ("cargo", vec!["test", "--", "--nocapture"])
            } else {
                ("cargo", vec!["test", pattern, "--", "--nocapture"])
            }
        } else if ctx.repo_path.join("package.json").exists() {
            if pattern.is_empty() {
                ("npm", vec!["test"])
            } else {
                ("npm", vec!["test", "--", pattern])
            }
        } else if ctx.repo_path.join("go.mod").exists() {
            if pattern.is_empty() {
                ("go", vec!["test", "./..."])
            } else {
                ("go", vec!["test", "-run", pattern, "./..."])
            }
        } else {
            return ToolResult::err("No recognized test toolchain found (Cargo.toml, package.json, go.mod)", 0);
        };

        let output = Command::new(cmd)
            .args(&args)
            .current_dir(&ctx.repo_path)
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = format!("{stdout}\n{stderr}");
                let truncated = if combined.len() > MAX_TEST_OUTPUT_BYTES {
                    format!("{}...\n(truncated, {} bytes total)", &combined[..MAX_TEST_OUTPUT_BYTES], combined.len())
                } else {
                    combined
                };
                let passed = out.status.success();
                let prefix = if passed { "TESTS PASSED" } else { "TESTS FAILED" };
                if passed {
                    ToolResult::ok(format!("{prefix}\n{truncated}"), 0)
                } else {
                    ToolResult::err(format!("{prefix}\n{truncated}"), 0)
                }
            }
            Err(e) => ToolResult::err(format!("Failed to run {cmd}: {e}"), 0),
        }
    }
}
