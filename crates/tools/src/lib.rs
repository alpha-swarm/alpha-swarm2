// WASI-portable tools (no native deps)
mod virt_fs;
mod virt_grep;

// Native-only tools (need tokio, Command, reqwest, tree-sitter, etc.)
#[cfg(feature = "native")]
#[allow(dead_code)]
mod fs;
#[cfg(feature = "native")]
#[allow(dead_code)]
mod grep;
#[cfg(feature = "native")]
mod quality_tools;
#[cfg(feature = "native")]
mod test_runner;
#[cfg(feature = "native")]
mod git;
#[cfg(feature = "native")]
mod shell;
#[cfg(feature = "native")]
mod tree_sitter_tools;
#[cfg(feature = "native")]
pub mod codegraph;
#[cfg(feature = "native")]
mod web;
#[cfg(feature = "native")]
pub mod nats_dispatch;
#[cfg(feature = "native")]
pub mod memory_tools;
#[cfg(feature = "native")]
pub mod wasm_tools;

pub mod registry;
pub use registry::ToolRegistry;

use serde_json::Value;

/// Context provided to every tool execution.
pub struct ToolContext {
    /// Root of the repository (for disk tools).
    pub repo_path: std::path::PathBuf,
    /// Project name.
    pub project: String,
    /// Hard timeout.
    pub timeout: std::time::Duration,
    /// Optional FileProvider for zero-disk tools.
    pub file_provider: Option<std::sync::Arc<std::sync::Mutex<Box<dyn virt_git::FileProvider>>>>,
}

/// Result of a tool execution.
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
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

/// A deterministic tool.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult;
}
