mod prompt;
mod parser;
pub mod code_utils;

// Re-export FileProvider from virt-git (canonical location)
pub use virt_git::{FileProvider, DiskFileProvider, VirtFileProvider};

// Agent execution requires native deps (tokio, knowledge-base, etc.)
#[cfg(feature = "native")]
mod agent;

#[cfg(feature = "native")]
pub use agent::{Agent, AgentResult, KnowledgeConfig, AgentProgress};
pub use code_utils::fuzzy_replace;
pub use parser::{FileEdit, ToolCall, AgentAction, parse_edits, parse_actions};
pub use prompt::{AgentType, build_prompt, build_prompt_with_type, build_tool_prompt};
