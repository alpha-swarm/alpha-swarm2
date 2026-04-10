//! Quality check tools for the agent iteration loop.
//! Agent can run fmt/lint/build/test during the tool loop and fix errors.

use serde_json::{json, Value};
use crate::{Tool, ToolContext, ToolResult};

/// Run cargo fmt --check on the workspace.
pub struct RunFmtTool;

#[async_trait::async_trait]
impl Tool for RunFmtTool {
    fn name(&self) -> &str { "run_fmt" }
    fn description(&self) -> &str { "Check code formatting (cargo fmt --check). Returns OK or list of files that need formatting." }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: Value, ctx: &ToolContext) -> ToolResult {
        run_cmd("cargo fmt -- --check", &ctx.repo_path)
    }
}

/// Run cargo clippy on the workspace.
pub struct RunLintTool;

#[async_trait::async_trait]
impl Tool for RunLintTool {
    fn name(&self) -> &str { "run_lint" }
    fn description(&self) -> &str { "Run linter (cargo clippy). Returns warnings and errors." }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: Value, ctx: &ToolContext) -> ToolResult {
        run_cmd("cargo clippy", &ctx.repo_path)
    }
}

/// Run cargo check on the workspace.
pub struct RunBuildTool;

#[async_trait::async_trait]
impl Tool for RunBuildTool {
    fn name(&self) -> &str { "run_build" }
    fn description(&self) -> &str { "Check if code compiles (cargo check). Returns OK or compilation errors." }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: Value, ctx: &ToolContext) -> ToolResult {
        run_cmd("cargo check", &ctx.repo_path)
    }
}

/// Run cargo test --lib on the workspace.
pub struct RunTestTool;

#[async_trait::async_trait]
impl Tool for RunTestTool {
    fn name(&self) -> &str { "run_test" }
    fn description(&self) -> &str { "Run unit tests (cargo test --lib). Returns test results." }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: Value, ctx: &ToolContext) -> ToolResult {
        run_cmd("cargo test --lib", &ctx.repo_path)
    }
}

/// Run a shell command in the repo dir, capture output.
fn run_cmd(cmd: &str, cwd: &std::path::Path) -> ToolResult {
    let start = std::time::Instant::now();
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let (program, args) = parts.split_first().unwrap_or((&"echo", &[]));

    match std::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
    {
        Ok(output) => {
            let duration = start.elapsed().as_millis() as u64;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let content = if output.status.success() {
                if stdout.is_empty() && stderr.is_empty() {
                    "OK — no issues found".to_string()
                } else {
                    format!("OK\n{}{}", stdout, stderr).chars().take(5000).collect()
                }
            } else {
                format!("FAILED (exit {})\n{}{}", output.status.code().unwrap_or(-1), stderr, stdout)
                    .chars().take(5000).collect()
            };

            if output.status.success() {
                ToolResult::ok(content, duration)
            } else {
                ToolResult::err(content, duration)
            }
        }
        Err(e) => ToolResult::err(format!("Command failed: {e}"), 0),
    }
}
