//! Quality check tools for the agent iteration loop.
//! Agent can run fmt/lint/build/test during the tool loop and fix errors.

use serde_json::{json, Value};
use crate::{Tool, ToolContext, ToolResult};

/// Detect which crate a file belongs to by walking up to find Cargo.toml.
fn detect_crate(repo: &std::path::Path, file_path: &str) -> Option<String> {
    let full = repo.join(file_path);
    let mut dir = full.parent()?;
    loop {
        let cargo = dir.join("Cargo.toml");
        if cargo.exists() {
            let content = std::fs::read_to_string(&cargo).ok()?;
            for line in content.lines() {
                if let Some(name) = line.strip_prefix("name = ") {
                    return Some(name.trim().trim_matches('"').to_string());
                }
            }
        }
        dir = dir.parent()?;
        if dir == repo { break; }
    }
    None
}

/// Build a scoped cargo check command from the ToolContext.
fn scoped_check_cmd(ctx: &ToolContext) -> String {
    // Try to detect crate from project name (which is often the edited file's crate)
    let crate_dir = ctx.repo_path.join("crates");
    if crate_dir.exists() {
        // Check if there's a recent file modification hint in the project field
        if let Some(crate_name) = detect_crate(&ctx.repo_path, &ctx.project) {
            return format!("cargo check -p {crate_name}");
        }
    }
    "cargo check".to_string()
}

pub struct RunFmtTool;

#[async_trait::async_trait]
impl Tool for RunFmtTool {
    fn name(&self) -> &str { "run_fmt" }
    fn description(&self) -> &str { "Check formatting (cargo fmt --check)." }
    fn parameters_schema(&self) -> Value { json!({"type": "object", "properties": {}}) }
    async fn execute(&self, _params: Value, ctx: &ToolContext) -> ToolResult {
        run_cmd("cargo fmt -- --check", &ctx.repo_path)
    }
}

pub struct RunLintTool;

#[async_trait::async_trait]
impl Tool for RunLintTool {
    fn name(&self) -> &str { "run_lint" }
    fn description(&self) -> &str { "Run linter (cargo clippy)." }
    fn parameters_schema(&self) -> Value { json!({"type": "object", "properties": {}}) }
    async fn execute(&self, _params: Value, ctx: &ToolContext) -> ToolResult {
        run_cmd("cargo clippy", &ctx.repo_path)
    }
}

pub struct RunBuildTool;

#[async_trait::async_trait]
impl Tool for RunBuildTool {
    fn name(&self) -> &str { "run_build" }
    fn description(&self) -> &str { "Check compilation (cargo check). Auto-scopes to the edited crate when possible." }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"crate_name": {"type": "string", "description": "Optional: specific crate to check (e.g. swarm-orchestrator). Omit for auto-detect."}}})
    }
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let cmd = if let Some(name) = params.get("crate_name").and_then(|v| v.as_str()) {
            format!("cargo check -p {name}")
        } else {
            scoped_check_cmd(ctx)
        };
        run_cmd(&cmd, &ctx.repo_path)
    }
}

pub struct RunCheckTool;

#[async_trait::async_trait]
impl Tool for RunCheckTool {
    fn name(&self) -> &str { "run_check" }
    fn description(&self) -> &str { "Run all quality checks in parallel (fmt + build + clippy). Use INSTEAD of separate run_fmt/run_build/run_lint." }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"crate_name": {"type": "string", "description": "Optional: specific crate to check."}}})
    }
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let crate_flag = if let Some(name) = params.get("crate_name").and_then(|v| v.as_str()) {
            format!(" -p {name}")
        } else {
            // Auto-detect crate from project context
            scoped_check_cmd(ctx).replace("cargo check", "").to_string()
        };
        let cwd = ctx.repo_path.clone();
        let start = std::time::Instant::now();

        let cmds = vec![
            ("fmt", "cargo fmt -- --check".to_string()),
            ("build", format!("cargo check{crate_flag}")),
            ("clippy", format!("cargo clippy{crate_flag}")),
        ];

        let handles: Vec<_> = cmds.into_iter().map(|(name, cmd)| {
            let cwd = cwd.clone();
            std::thread::spawn(move || (name.to_string(), run_cmd(&cmd, &cwd)))
        }).collect();

        let mut parts = Vec::new();
        let mut any_err = false;
        let per_check_limit = MAX_OUTPUT_CHARS / 3;
        for h in handles {
            if let Ok((name, result)) = h.join() {
                if result.is_error { any_err = true; }
                let content: String = if result.content.len() > per_check_limit {
                    truncate_end(&result.content, per_check_limit)
                } else { result.content };
                parts.push(format!("[{}] {}", name, content));
            }
        }

        let duration = start.elapsed().as_millis() as u64;
        let content = parts.join("\n\n");
        if any_err { ToolResult::err(content, duration) } else { ToolResult::ok(content, duration) }
    }
}

pub struct RunTestTool;

#[async_trait::async_trait]
impl Tool for RunTestTool {
    fn name(&self) -> &str { "run_test" }
    fn description(&self) -> &str { "Run unit tests (cargo test --lib). Auto-scopes to edited crate." }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"crate_name": {"type": "string", "description": "Optional: specific crate to test."}}})
    }
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let cmd = if let Some(name) = params.get("crate_name").and_then(|v| v.as_str()) {
            format!("cargo test --lib -p {name}")
        } else {
            "cargo test --lib".to_string()
        };
        run_cmd(&cmd, &ctx.repo_path)
    }
}

/// Max output chars to avoid flooding context.
const MAX_OUTPUT_CHARS: usize = 3000;

/// Truncate from start, keeping the last N chars (errors are at the bottom).
fn truncate_end(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    let skip = s.len() - max;
    let boundary = s.ceil_char_boundary(skip);
    format!("...[{skip} chars truncated]\n{}", &s[boundary..])
}

fn run_cmd(cmd: &str, cwd: &std::path::Path) -> ToolResult {
    let start = std::time::Instant::now();
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let (program, args) = parts.split_first().unwrap_or((&"echo", &[]));

    match std::process::Command::new(program).args(args).current_dir(cwd).output() {
        Ok(output) => {
            let duration = start.elapsed().as_millis() as u64;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let content = if output.status.success() {
                if stdout.is_empty() && stderr.is_empty() {
                    "OK".to_string()
                } else {
                    format!("OK\n{}{}", stdout, stderr).chars().take(MAX_OUTPUT_CHARS).collect()
                }
            } else {
                // Keep END of output — error messages are at the bottom
                let combined = format!("FAILED (exit {})\n{}{}", output.status.code().unwrap_or(-1), stderr, stdout);
                truncate_end(&combined, MAX_OUTPUT_CHARS)
            };

            if output.status.success() { ToolResult::ok(content, duration) } else { ToolResult::err(content, duration) }
        }
        Err(e) => ToolResult::err(format!("Command failed: {e}"), 0),
    }
}
