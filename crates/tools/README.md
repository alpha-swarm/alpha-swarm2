# swarm-tools

Deterministic tool library for alpha-swarm agents. Tools execute instantly (0 LLM tokens) for mechanical operations. Models choose between tools and LLM inference at each step.

## Available Tools

| Tool | Module | Description |
|------|--------|-------------|
| `read_file` | fs | Read file contents (max 100KB) |
| `write_file` | fs | Write file, create parent directories |
| `delete_file` | fs | Delete a file |
| `list_files` | fs | Glob pattern file listing (max 500 files) |
| `grep` | grep | Search with ripgrep (fallback to grep) |
| `ts_rename` | tree_sitter | AST-aware symbol rename (Rust) |
| `ts_find` | tree_sitter | Find symbol occurrences in AST |
| `ts_signatures` | tree_sitter | Extract function/struct/impl signatures |
| `run_tests` | test_runner | Auto-detect cargo/npm/go, run tests |
| `git_diff` | git | Show uncommitted changes |
| `git_status` | git | Working tree status |
| `web_search` | web | DuckDuckGo search (no API key) |
| `fetch_url` | web | Fetch URL, extract text |
| `search_crates` | web | Search crates.io API |
| `run_command` | shell | Run allowlisted commands (cargo, npm, go, etc.) |

## Architecture

```rust
// Tool trait — implement this to add a new tool
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;  // JSON Schema
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult;
}

// Registry with NATS remote dispatch
let mut tools = ToolRegistry::with_defaults();  // 15 built-in tools
tools = tools.with_nats_dispatcher(dispatcher); // try remote first, local fallback
```

## Execution Modes

1. **Local** — tool executes in-process on the daemon
2. **NATS remote** — tool call dispatched to WASI worker via NATS request-reply
3. **Ollama native** — model uses `tools` API parameter (qwen2.5 family)
4. **Text fallback** — model outputs `<<<TOOL name\n{params}\n>>>` blocks

## Adding a New Tool

1. Create a new file in `src/` (e.g., `src/my_tool.rs`)
2. Implement the `Tool` trait
3. Register in `src/registry.rs` `with_defaults()`
4. Add to `src/lib.rs` module list
