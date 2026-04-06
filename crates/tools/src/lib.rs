mod fs;
mod grep;
mod test_runner;
mod git;
mod shell;
mod tree_sitter_tools;
mod web;
mod registry;
pub mod nats_dispatch;

pub use registry::ToolRegistry;

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

/// Context provided to every tool execution.
pub struct ToolContext {
    /// Root of the repository being worked on.
    pub repo_path: PathBuf,
    /// Project name (for logging/tracking).
    pub project: String,
    /// Hard timeout for this tool execution.
    pub timeout: Duration,
}

/// Result of a tool execution, fed back to the model.
pub struct ToolResult {
    /// Output content to show the model.
    pub content: String,
    /// Whether the tool encountered an error.
    pub is_error: bool,
    /// Wall-clock time the tool took.
    pub duration_ms: u64,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>, duration_ms: u64) -> Self {
        Self { content: content.into(), is_error: false, duration_ms }
    }

    pub fn err(content: impl Into<String>, duration_ms: u64) -> Self {
        Self { content: content.into(), is_error: true, duration_ms }
    }
}

/// A deterministic tool that an agent can invoke instead of using LLM inference.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name used in tool calls (e.g., "read_file", "run_tests").
    fn name(&self) -> &str;

    /// Human-readable description for the model's system prompt.
    fn description(&self) -> &str;

    /// JSON Schema describing the parameters this tool accepts.
    fn parameters_schema(&self) -> Value;

    /// Execute the tool with the given parameters.
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult;
}
