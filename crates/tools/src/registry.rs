use std::collections::HashMap;
use std::time::Instant;

use serde_json::Value;
use tracing::{info, warn};

use crate::{Tool, ToolContext, ToolResult};

/// Registry of available tools. Provides prompt generation and dispatch.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    /// Create a registry pre-loaded with all built-in tools.
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();
        reg.register(Box::new(crate::fs::ReadFileTool));
        reg.register(Box::new(crate::fs::WriteFileTool));
        reg.register(Box::new(crate::fs::DeleteFileTool));
        reg.register(Box::new(crate::fs::ListFilesTool));
        reg.register(Box::new(crate::grep::GrepTool));
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
        reg
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Generate the AVAILABLE TOOLS section for the model's system prompt.
    pub fn tools_prompt(&self) -> String {
        let mut prompt = String::from("AVAILABLE TOOLS:\nCall tools with <<<TOOL tool_name\\n{\"param\": \"value\"}\\n>>>.\nUse tools for mechanical operations. Use <<<AGENT>>> for creative work needing LLM.\n\nTOOLS:\n");
        let mut names: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
        names.sort();
        for name in names {
            let tool = &self.tools[name];
            prompt.push_str(&format!("- {}  — {}\n  params: {}\n", name, tool.description(), tool.parameters_schema()));
        }
        prompt
    }

    /// Execute a tool by name. Returns an error result if the tool doesn't exist.
    pub async fn execute(&self, name: &str, params: Value, ctx: &ToolContext) -> ToolResult {
        let Some(tool) = self.tools.get(name) else {
            return ToolResult::err(format!("Unknown tool: {name}"), 0);
        };

        let start = Instant::now();
        info!(tool = name, "Executing tool");

        let result = tool.execute(params, ctx).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        if result.is_error {
            warn!(tool = name, duration_ms, "Tool failed");
        } else {
            info!(tool = name, duration_ms, output_len = result.content.len(), "Tool completed");
        }

        ToolResult {
            duration_ms,
            ..result
        }
    }

    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Convert all tools to Ollama-compatible tool definitions for native tool calling.
    pub fn to_ollama_tools(&self) -> Vec<serde_json::Value> {
        let mut names: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
        names.sort();
        names.iter().map(|name| {
            let tool = &self.tools[*name];
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name(),
                    "description": tool.description(),
                    "parameters": tool.parameters_schema(),
                }
            })
        }).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}
