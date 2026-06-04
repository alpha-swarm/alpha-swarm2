use std::collections::HashMap;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolResult};

/// Registry of available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    #[cfg(feature = "native")]
    nats_dispatcher: Option<crate::nats_dispatch::NatsToolDispatcher>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            #[cfg(feature = "native")]
            nats_dispatcher: None,
        }
    }

    /// Create with WASI-portable tools only (no native deps).
    pub fn with_virt_defaults() -> Self {
        let mut reg = Self::new();
        reg.register(Box::new(crate::virt_fs::VirtReadFileTool));
        reg.register(Box::new(crate::virt_fs::VirtWriteFileTool));
        reg.register(Box::new(crate::virt_fs::VirtListFilesTool));
        reg.register(Box::new(crate::virt_grep::VirtGrepTool));
        reg
    }

    /// Create with all built-in tools (native + virt).
    #[cfg(feature = "native")]
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();
        // Use virt tools (they fall back to disk when no FileProvider)
        reg.register(Box::new(crate::virt_fs::VirtReadFileTool));
        reg.register(Box::new(crate::virt_fs::VirtWriteFileTool));
        reg.register(Box::new(crate::fs::DeleteFileTool));
        reg.register(Box::new(crate::virt_fs::VirtListFilesTool));
        reg.register(Box::new(crate::virt_grep::VirtGrepTool));
        reg.register(Box::new(crate::test_runner::RunTestsTool));
        reg.register(Box::new(crate::git::GitDiffTool));
        reg.register(Box::new(crate::git::GitStatusTool));
        reg.register(Box::new(crate::shell::RunCommandTool));
        reg.register(Box::new(crate::tree_sitter_tools::TreeSitterRenameTool));
        reg.register(Box::new(crate::tree_sitter_tools::TreeSitterFindTool));
        reg.register(Box::new(crate::tree_sitter_tools::TreeSitterSignaturesTool));
        reg.register(Box::new(crate::web::WebSearchTool));
        reg.register(Box::new(crate::web::FetchUrlTool));
        reg.register(Box::new(crate::web::SearchCratesTool));
        // Quality tools for edit → test → fix iteration
        reg.register(Box::new(crate::quality_tools::RunCheckTool));
        reg.register(Box::new(crate::quality_tools::RunFmtTool));
        reg.register(Box::new(crate::quality_tools::RunLintTool));
        reg.register(Box::new(crate::quality_tools::RunBuildTool));
        reg.register(Box::new(crate::quality_tools::RunTestTool));
        reg
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    #[cfg(feature = "native")]
    pub fn with_nats_dispatcher(mut self, dispatcher: crate::nats_dispatch::NatsToolDispatcher) -> Self {
        self.nats_dispatcher = Some(dispatcher);
        self
    }

    /// Register the agent-facing memory tools (recall/store over the daemon's
    /// NATS DB bridge). Only call when a NATS client is available so the
    /// prompt's tool list stays honest.
    #[cfg(feature = "native")]
    pub fn with_memory_tools(mut self, client: async_nats::Client) -> Self {
        self.register(Box::new(crate::memory_tools::MemoryRecallTool::new(client.clone())));
        self.register(Box::new(crate::memory_tools::MemoryStoreTool::new(client)));
        self
    }

    /// Surface the process-global WASM tool set (if installed) as normal tools.
    /// No-op when no embedded Wassette host has been initialized.
    #[cfg(feature = "native")]
    pub fn with_wasm_tools(mut self) -> Self {
        if let Some(set) = crate::wasm_tools::wasm_tools() {
            for spec in &set.specs {
                self.register(Box::new(crate::wasm_tools::WasmTool::new(
                    set.host.clone(),
                    spec.clone(),
                )));
            }
        }
        self
    }

    /// Get tool names for inclusion in agent prompts.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Execute a tool by name.
    pub async fn execute(&self, name: &str, params: Value, ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();

        // Try NATS remote dispatch first (native only)
        #[cfg(feature = "native")]
        if let Some(ref dispatcher) = self.nats_dispatcher {
            if let Ok(result) = dispatcher.call(name, params.clone(), ctx).await {
                return result;
            }
        }

        // Local execution
        if let Some(tool) = self.tools.get(name) {
            let mut result = tool.execute(params, ctx).await;
            result.duration_ms = start.elapsed().as_millis() as u64;

            #[cfg(feature = "native")]
            if result.is_error {
                tracing::warn!(tool = name, mode = "local", duration_ms = result.duration_ms, "Tool failed");
            } else {
                tracing::info!(tool = name, mode = "local", duration_ms = result.duration_ms, output_len = result.content.len(), "Tool completed");
            }

            result
        } else {
            ToolResult::err(format!("Unknown tool: {name}"), 0)
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
