# alpha-swarm: Phased Implementation Plan

## Feasibility Summary

| Component | Status | Notes |
|---|---|---|
| WasmCloud orchestration | **GREEN** | Mature lattice, NATS-based, production-proven |
| WasmCloud subprocess spawning | **RED** | Components can't shell out — need native capability providers as wrappers |
| WasmCloud WASI filesystem | **YELLOW** | Interface exists but sandboxed — need custom provider or WASI Virt |
| Ollama CLI/API | **GREEN** | Full HTTP API at localhost:11434, model metadata queryable |
| SurrealDB vector search | **GREEN** | HNSW indexing, hybrid BM25+vector fusion, Rust SDK stable |
| Rust to wasm32-wasip2 | **GREEN** | Tier 2 stable since Rust 1.82, full networking + file I/O |
| Multi-machine lattice | **GREEN** | 3-node NATS cluster + JetStream, Helm charts available |

**Critical architectural insight**: WasmCloud's security model prevents components from spawning processes or accessing host filesystem directly. All external tool access (Ollama, git, build tools) must go through **native capability providers** that wrap CLI/API calls and expose them via wRPC to WASI components.

**Steering decision**: Rust-only agents. Use Ollama HTTP API (not CLI). Build native providers for: Ollama, git/filesystem, and build toolchain.

---

## Phase 0: Foundation (Week 1-2)

**Goal**: Prove a single local agent can call Ollama, edit a file, and run a test — no WasmCloud yet.

### Tasks
1. Scaffold Rust workspace (`agent-core`, `ollama-client`, `quality-gate` crates)
2. Write `ollama-client` crate: thin wrapper over Ollama HTTP API (`/api/tags`, `/api/generate`, `/api/chat`)
3. Write `agent-core`: minimal agent loop — prompt -> LLM response -> parse tool calls -> execute -> loop
4. Write `quality-gate`: run `cargo check`/`cargo test`/`cargo fmt --check` on a target repo, return pass/fail + output

### Checkpoint 0
```
TEST: Run the agent binary pointing at a local Rust repo.
      Agent calls Ollama (qwen2.5-coder:7b), gets a code change suggestion,
      applies it as a diff, runs quality-gate, reports pass/fail.
VERIFY: cargo test in the workspace passes.
VERIFY: ollama list shows the model, agent discovers it automatically.
```

### Milestone: "Hello Agent"
Single CLI binary that does one-shot code modification with local Ollama + validation.

---

## Phase 1: Knowledge Base (Week 3-4)

**Goal**: Agents persist their work to SurrealDB so future agents can learn from it.

### Tasks
1. Add `knowledge-base` crate wrapping SurrealDB Rust SDK
2. Schema design: `task`, `agent_run`, `diff`, `conversation`, `error` tables
3. Vector embeddings for diffs and conversations (use `fastembed` crate — no API key needed)
4. Store: agent ID, prompt, response, diff produced, quality-gate result, embedding
5. Query: "find similar past diffs", "find past errors on this file", "what are parallel agents doing"

### Checkpoint 1
```
TEST: Run 3 agents sequentially on the same repo with different tasks.
      Agent 2 queries SurrealDB and sees Agent 1's work.
      Agent 3 sees both Agent 1 and Agent 2's work.
VERIFY: surrealdb vector search returns relevant past diffs (cosine similarity > 0.7).
VERIFY: An agent given a task that was already completed finds it and skips.
VERIFY: An agent given a task where a previous agent failed avoids the same approach.
```

### Milestone: "Agents Learn"
Agents query shared knowledge before acting, avoid repeated mistakes.

---

## Phase 2: WasmCloud Integration (Week 5-7)

**Goal**: Wrap the working agent into WasmCloud components + providers.

### Tasks
1. Build native **Ollama capability provider** (Rust binary, exposes model listing + inference via wRPC)
2. Build native **VirtFS capability provider** (Rust binary, handles git clone/checkout, exposes wasi:filesystem to components)
3. Build native **QualityGate capability provider** (Rust binary, runs build/lint/fmt/test toolchain)
4. Compile agent-core into a WASI component (wasm32-wasip2) that imports these 3 capabilities
5. Deploy locally: single WasmCloud host, NATS server, 3 providers + 1 agent component
6. Write WIT interfaces for each provider

### Checkpoint 2
```
TEST: wadm deploy the application manifest.
      Agent component starts, discovers models via Ollama provider,
      reads files via VirtFS provider, produces a diff,
      quality-gate provider validates it.
VERIFY: wash get inventory shows host with 3 providers + 1 component.
VERIFY: Same end-to-end flow as Phase 0 but running inside WasmCloud.
VERIFY: Agent component is <5MB wasm binary.
```

### Milestone: "WasmCloud Native"
Agent runs as a WASI component orchestrated by WasmCloud with capability providers.

---

## Phase 3: Multi-Agent Orchestration (Week 8-10)

**Goal**: Run multiple agents in parallel on the same repo, coordinated via SurrealDB.

### Tasks
1. Build **Orchestrator component** — accepts a high-level task, decomposes into sub-tasks, spawns agent components
2. Orchestrator uses Claude API (or large Ollama model) for task decomposition
3. Add SurrealDB capability provider (expose knowledge-base queries to WASI components)
4. Implement task locking in SurrealDB (agent claims a task, others skip it)
5. Git worktree isolation: VirtFS provider creates separate worktrees per agent
6. Implement diff merging: orchestrator collects diffs from agents, applies sequentially, re-validates

### Checkpoint 3
```
TEST: Give orchestrator a task: "add logging to all public functions in crate X".
      Orchestrator decomposes into N sub-tasks (one per module).
      N agents run in parallel, each on its own worktree.
      No two agents touch the same file.
VERIFY: SurrealDB shows N agent_run records with non-overlapping file sets.
VERIFY: All diffs merge cleanly onto main branch.
VERIFY: Quality gate passes on the merged result.
VERIFY: Total time < N x single-agent time (parallelism works).
```

### Milestone: "Swarm"
Multiple agents work on a codebase simultaneously without conflicts.

---

## Phase 4: Distributed Execution (Week 11-14)

**Goal**: Run WasmCloud lattice across multiple machines.

### Machines

| Machine | SSH Host | Role | Hardware | Tailscale IP |
|---|---|---|---|---|
| Local | — | Orchestrator, CLI, SurrealDB | MacBook | 100.79.38.122 |
| csatapaci | csatapaci | Inference (big models) | M2 Max, 96GB RAM, 3.6TB | 100.81.10.8 |
| malna (RPi) | malna | NATS quorum, quality gate, SurrealDB replica | 8GB RAM, 512GB SSD, ARM64 | TBD |

### Current Status (Phase 4a — 2 machines)
- [x] NATS 2-node cluster: local (:14222) <-> csatapaci (:4222)
- [x] Cross-machine NATS messaging verified
- [x] WasmCloud 2.0 host on csatapaci running agent-worker component
- [x] Agent-worker HTTP endpoint reachable from local (http://100.81.10.8:8000/)
- [ ] Agent-worker calls Ollama on csatapaci via wasi:http/outgoing-handler
- [ ] Swarm CLI submits goals that execute on csatapaci

### Phase 4b — Add malna (RPi) as 3rd node

**Prerequisites**: SSH key auth to malna (same setup as csatapaci — chmod 755 ~, add ed25519 key to authorized_keys)

**Tasks**:
1. Install NATS server on malna (`apt install nats-server` or build from source for ARM64)
2. Add malna NATS config (infra/nats-malna.conf) routing to both local and csatapaci
3. 3-node NATS cluster with JetStream R3 replication — tolerates 1 node failure
4. Install SurrealDB on malna — run as replica or shared store accessible from all nodes
5. Install wash 2.0 on malna, start WasmCloud host
6. Deploy quality-gate component on malna (runs cargo check/test — no GPU needed)
7. Tag hosts with labels: local=orchestrator, csatapaci=inference, malna=infra+quality

### NATS Cluster Config (3 nodes)

```
Node 1 (local):     100.79.38.122:14222 (client) / :16222 (cluster)
Node 2 (csatapaci): 100.81.10.8:4222   (client) / :6222  (cluster)
Node 3 (malna):     TBD:4222           (client) / :6222  (cluster)
Cluster name: alpha-swarm
JetStream R3: every stream replicated to all 3 nodes
Quorum: 2/3 — survives any single node failure
```

### Checkpoint 4
```
TEST: 3 machines in the lattice. Submit a task from local.
      Orchestrator decomposes on local.
      Agent-worker runs on csatapaci (calls Ollama locally).
      Quality gate runs on malna.
VERIFY: NATS cluster shows 3 nodes.
VERIFY: Kill one NATS node — cluster continues (quorum 2/3).
VERIFY: Agent on csatapaci uses Ollama models only available there.
VERIFY: SurrealDB queries from all machines return consistent results.
```

### Milestone: "Distributed Swarm"
Agents run across machines, floating to where resources are available.

---

## Phase 5: Local-First Agent Maturity (Week 15-18)

**Goal**: Polish the agent loop for real-world code tasks, optimize for local inference.

### Tasks
1. Model routing logic: classify tasks by complexity -> assign appropriate model size
   - Simple (rename, fmt fix) -> qwen2.5-coder:7b
   - Medium (add function, refactor) -> deepseek-coder-v2:16b
   - Complex (architecture change) -> Claude API or qwen2.5-coder:32b
2. Retry with escalation: if small model fails quality gate, retry with larger model
3. Context optimization: only feed relevant files (use SurrealDB vector search on file embeddings)
4. Agent specialization: lint-fixer agent, test-writer agent, refactorer agent (same binary, different system prompts)
5. Conversation streaming: store intermediate reasoning steps in SurrealDB for debugging
6. Metrics dashboard: success rate, token usage, quality gate pass rate, time per task

### Checkpoint 5
```
TEST: Submit 20 diverse code tasks to the system.
      Track: which model was chosen, pass/fail, retry count, time.
VERIFY: >70% first-attempt pass rate on quality gate.
VERIFY: Model routing chose small models for simple tasks (>80% of the time).
VERIFY: Retry-with-escalation recovered at least 50% of initial failures.
VERIFY: No task took >5 minutes on local hardware.
```

### Milestone: "Production Local Agents"
System reliably handles real code tasks with intelligent model routing and self-healing.

---

## Progression

```
Phase 0: Ollama only, single agent, CLI binary
Phase 1: + SurrealDB (local), still CLI
Phase 2: + WasmCloud (local single-host)
Phase 3: + multi-agent (still single machine)
Phase 4: + multi-machine (opt-in, not required)
Phase 5: + smart local model routing
Phase 6: + distributed daemon via wasmCloud plugins + resource-aware scheduling
Phase 7: + tool-use agents (MCP-style tools, LSP, web search, structured operations)
```

Each phase is **independently useful**. You can stop at any phase and have a working system.

---

## Phase 6: Distributed Agent Daemon + Resource Awareness (Future)

**Goal**: Agent daemon runs as a wasmCloud plugin on any machine in the lattice. Tasks are routed to the machine with the best resources for the job.

### Why

Currently the agent-daemon is a single native binary on one machine. It calls Ollama over the network (adds latency) and can't leverage multiple machines' resources. To truly distribute:

1. Daemon should run on every machine (next to local Ollama = zero inference latency)
2. Tasks should be routed based on resource availability
3. Git clone/quality gate should run where there's disk space and the right toolchain

### wasmCloud Plugin Approach

Convert the daemon into a WASI component + custom host plugins:

```wit
// Custom host plugin interfaces
interface git-ops {
    clone: func(url: string, path: string) -> result<string, string>;
    pull: func(path: string) -> result<_, string>;
    worktree-create: func(repo: string, branch: string) -> result<string, string>;
    diff: func(path: string) -> result<string, string>;
    apply-diff: func(repo: string, diff: string) -> result<_, string>;
}

interface shell-exec {
    run: func(cmd: string, args: list<string>, cwd: string) -> result<exec-result, string>;
}
```

The daemon WASI component imports these interfaces. The wasmCloud host provides them as native plugins. Deploy on any machine via the lattice.

### Resource-Aware Scheduling

Each host reports capabilities:

```toml
[labels]
alpha-swarm-role = "inference"
alpha-swarm-cpu-cores = "10"
alpha-swarm-ram-gb = "96"
alpha-swarm-disk-free-gb = "2700"
alpha-swarm-gpu = "false"
alpha-swarm-ollama-models = "qwen2.5-coder:7b,deepseek-coder:33b,codellama:34b"
alpha-swarm-toolchains = "rust,node,go"
```

Task routing heuristics:
- Inference → machine with the needed model loaded + most free RAM
- Git clone → machine with most free disk
- Quality gate (cargo test) → machine with the toolchain installed + most free CPU
- Complex task → machine with the largest model available
- Simple task → any machine with a small model

### Atomic Task Claiming

Multiple daemons watching SurrealDB need atomic claiming:

```sql
UPDATE agent_run
  SET status = 'running', claimed_by = $host
  WHERE status = 'pending'
    AND claimed_by IS NONE
  LIMIT 1
  RETURN BEFORE
```

First daemon to execute this wins. Others get empty result and skip.

### Tasks
1. Define custom WIT interfaces for git-ops and shell-exec
2. Implement host plugins in Rust for wasmCloud 2.0
3. Port agent-daemon to a WASI component importing these interfaces
4. Add resource reporting to each host (labels or heartbeat)
5. Implement resource-aware task routing in the daemon
6. Atomic task claiming in SurrealDB
7. Deploy and test across local + csatapaci + malna

### Checkpoint 6
```
TEST: Submit a task from the web UI.
      Daemon on csatapaci picks it up (has the model locally).
      Inference runs at localhost speed (not over network).
      Quality gate runs on malna (has the toolchain).
VERIFY: Task claimed atomically (no double execution).
VERIFY: Resource labels reported correctly per host.
VERIFY: Inference latency significantly lower (localhost vs network).
VERIFY: Killing one daemon doesn't lose pending tasks.
```

### Milestone: "Truly Distributed Swarm"
Agents float to the machine with the best resources. No single point of failure. Add a machine → it joins the swarm automatically.

---

## Phase 7: Tool-Use Agents — Tree-Sitter, LSP, Tests as Leaf Nodes (Future)

**Goal**: Leaf-node agents use deterministic tools (tree-sitter, LSP, test runners, grep, web search) instead of LLM inference for mechanical operations. The orchestrator model decides WHEN to use a tool vs when to use an LLM sub-agent. This makes simple operations instant, accurate, and free — LLM tokens are only spent on reasoning and creative code generation.

### Why

Current agents do everything via LLM text generation → parse `<<<EDIT>>>` blocks. This is:
- **Slow**: LLM generates 500 tokens to rename a variable (tree-sitter does it in 2ms)
- **Error-prone**: `--- OLD` block matching fails on whitespace, indentation, encoding
- **Wasteful**: don't need AI to delete a file, run tests, or find references
- **Blind**: agents don't know if the code compiles until the quality gate runs — too late

### Key Insight: Leaf Nodes Are Tools, Not Agents

The orchestrator decomposes a goal into a tree:

```
Goal: "refactor auth module — rename UserAuth to AuthService, add tests"
  │
  Orchestrator (qwen2.5:72b — reasoning, planning)
  │
  ├── 🔧 tree_sitter_rename("UserAuth", "AuthService")     ← instant, 0 tokens
  │     └── Finds all usages across AST, renames precisely
  │
  ├── 🔧 lsp_diagnostics("src/auth/")                       ← 50ms, 0 tokens
  │     └── Reports compile errors after rename
  │
  ├── 🤖 LLM Agent: "fix compile errors from rename"        ← LLM, ~200 tokens
  │     └── reads diagnostics, generates targeted fix
  │
  ├── 🔧 run_tests("cargo test auth")                       ← deterministic
  │     └── Returns pass/fail + output
  │
  ├── 🤖 LLM Agent: "write unit tests for AuthService"      ← LLM, ~1000 tokens
  │     └── needs creativity → LLM is the right tool
  │
  ├── 🔧 run_tests("cargo test auth")                       ← verify new tests pass
  │
  └── 🔧 web_search("rust best practices auth service")     ← network, 0 tokens
        └── context for the orchestrator's next decision
```

The model CHOOSES between `🔧 tool` and `🤖 agent` at each step. Simple/mechanical → tool. Creative/ambiguous → LLM agent.

### Architecture

```
crates/tools/                    ← NEW crate
├── src/
│   ├── lib.rs                   ← Tool trait + ToolRegistry
│   ├── fs.rs                    ← read_file, write_file, delete_file, list_files
│   ├── grep.rs                  ← grep, ripgrep wrapper
│   ├── tree_sitter.rs           ← parse, rename_symbol, find_references, extract_functions
│   ├── lsp.rs                   ← diagnostics, go_to_definition, completions, rename
│   ├── test_runner.rs           ← run_tests, run_single_test, test_coverage
│   ├── git.rs                   ← diff, commit, branch, status
│   ├── web.rs                   ← web_search, fetch_url, search_crates, search_docs
│   └── shell.rs                 ← run_command (sandboxed)
```

### Tool Trait (MCP-compatible)

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;  // JSON Schema
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolContext {
    pub repo_path: PathBuf,
    pub project: String,
    pub timeout: Duration,
}

pub struct ToolResult {
    pub content: String,       // Output to feed back to model
    pub is_error: bool,
    pub duration_ms: u64,
    pub tokens_saved: u32,     // Estimated tokens this would have cost via LLM
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn available_tools_prompt(&self) -> String { ... }  // For model system prompt
    pub async fn execute(&self, name: &str, params: Value, ctx: &ToolContext) -> ToolResult { ... }
}
```

### Tree-Sitter Tools (the big win)

Tree-sitter gives us structural code understanding without LLM:

```rust
// tree_sitter.rs
pub struct TreeSitterTool { /* language grammars loaded at init */ }

impl TreeSitterTool {
    /// Parse file → AST → find all occurrences of symbol
    pub fn find_symbol(&self, file: &str, symbol: &str) -> Vec<Location> { ... }
    
    /// Rename symbol across file (AST-aware, not text replace)
    pub fn rename_symbol(&self, file: &str, old: &str, new: &str) -> Result<String> { ... }
    
    /// Extract all function/struct/impl signatures from a file
    pub fn extract_signatures(&self, file: &str) -> Vec<Signature> { ... }
    
    /// Find all imports/uses of a module
    pub fn find_imports(&self, file: &str, module: &str) -> Vec<Location> { ... }
    
    /// Get the AST context around a line (function body, impl block, etc.)
    pub fn context_at_line(&self, file: &str, line: u32) -> ASTContext { ... }
}
```

Languages supported: Rust (primary), TypeScript, Go, Python — via `tree-sitter-{lang}` crates.

### LSP Tools (compile-time intelligence)

Start a language server (rust-analyzer, tsserver) and query it:

```rust
// lsp.rs
pub struct LspTool { client: LspClient }

impl LspTool {
    /// Get all diagnostics (errors, warnings) for a file or project
    pub async fn diagnostics(&self, file: Option<&str>) -> Vec<Diagnostic> { ... }
    
    /// Find all references to a symbol at a position
    pub async fn references(&self, file: &str, line: u32, col: u32) -> Vec<Location> { ... }
    
    /// Go to definition
    pub async fn definition(&self, file: &str, line: u32, col: u32) -> Option<Location> { ... }
    
    /// Semantic rename (cross-file, type-aware)
    pub async fn rename(&self, file: &str, line: u32, col: u32, new_name: &str) -> Vec<FileEdit> { ... }
    
    /// Get completions at position (useful for model to know what's valid)
    pub async fn completions(&self, file: &str, line: u32, col: u32) -> Vec<Completion> { ... }
}
```

LSP startup is slow (~2-5s for rust-analyzer) but subsequent queries are fast (<100ms). Keep the server alive across tool calls within a task.

### Test Runner Tools

```rust
// test_runner.rs
pub struct TestRunnerTool;

impl TestRunnerTool {
    /// Run all tests, return structured results
    pub async fn run_all(&self, ctx: &ToolContext) -> TestResults { ... }
    
    /// Run specific test by name/pattern
    pub async fn run_test(&self, ctx: &ToolContext, pattern: &str) -> TestResults { ... }
    
    /// Run only tests affected by changed files (via git diff)
    pub async fn run_affected(&self, ctx: &ToolContext) -> TestResults { ... }
}

pub struct TestResults {
    pub passed: u32,
    pub failed: u32,
    pub failures: Vec<TestFailure>,  // name, output, location
    pub duration_ms: u64,
}
```

### Web Search Tools

```rust
// web.rs
pub struct WebSearchTool { api_key: Option<String> }

impl WebSearchTool {
    /// Search the web (via SearXNG, Brave Search API, or similar)
    pub async fn search(&self, query: &str, max_results: usize) -> Vec<SearchResult> { ... }
    
    /// Fetch and extract text from a URL
    pub async fn fetch_url(&self, url: &str) -> String { ... }
    
    /// Search crates.io for a Rust crate
    pub async fn search_crates(&self, query: &str) -> Vec<CrateInfo> { ... }
}
```

### How The Model Chooses Tools

The orchestrator's system prompt includes available tools:

```
AVAILABLE TOOLS:
You can call tools by outputting JSON blocks. Use tools for mechanical operations.
Use LLM sub-agents only for creative/ambiguous tasks that need reasoning.

TOOLS:
- tree_sitter_rename(file, old_name, new_name) → renames symbol in AST (instant)
- tree_sitter_find(file, symbol) → finds all occurrences (instant)
- lsp_diagnostics(file?) → returns compile errors/warnings (fast)
- lsp_references(file, line, col) → finds all references (fast)
- lsp_rename(file, line, col, new_name) → semantic rename across project (fast)
- run_tests(pattern?) → runs tests, returns pass/fail + output
- read_file(path) → reads file content
- write_file(path, content) → writes file
- grep(pattern, path?) → searches for text
- web_search(query) → searches the internet
- fetch_url(url) → fetches web page content
- sub_agent(description, files, complexity) → spawns LLM agent for complex work

OUTPUT FORMAT:
<<<TOOL tool_name
{"param": "value"}
>>>

or for LLM work:

<<<AGENT
{"description": "...", "files": [...], "complexity": "medium"}
>>>
```

The orchestrator loop:
1. Model receives goal + repo context + tool list
2. Model outputs tool calls OR agent spawns
3. Daemon executes tools (instant) or agents (LLM)
4. Results fed back to model
5. Model decides next action
6. Repeat until `<<<DONE>>>` or fuel exhausted

### Fuel Budget (already partially implemented)

The existing fuel system in `executor.rs` (time, tokens, iterations) applies to the tool-use loop. Tools are "free" (0 tokens) but still count against time. This naturally incentivizes the model to use tools over LLM calls.

### Tool Calling Protocol

**Primary: Ollama Native Tool Use** (implemented)

Models that support the Ollama `tools` API parameter (qwen2.5 family) use the native protocol:
- Tools passed as JSON schema in the `tools` field of `/api/chat`
- Model returns `tool_calls` array in the response message
- Results fed back as `role: "tool"` messages
- Compact, no extra tokens wasted on format overhead
- Model is trained on this format → higher accuracy

**Fallback: Text-Based <<<TOOL>>> Protocol** (implemented)

For models that don't support native tools (deepseek-coder, codellama):
- Tools described in the system prompt as text
- Model outputs `<<<TOOL tool_name\n{params}\n>>>` blocks
- Results concatenated and fed back as user messages
- Less token-efficient but works with any model

**Future Options (documented for later)**

1. **Context Window Management**: Keep rolling window of last N tool results, summarize older ones into a single history message. Prevents context from growing unboundedly.

2. **Structured Result Truncation**: Instead of full file contents, return signatures and relevant sections. E.g., `read_file` returns: "234 lines, 8 functions (main, parse, apply). Lines 45-67: [relevant section]".

3. **Tool Result Caching**: Cache results of deterministic tools (read_file, list_files) within a session. Skip re-execution if params haven't changed.

4. **Streaming Tool Results**: For long-running tools (tests, builds), stream partial results back to model to enable early abort.

5. **MCP Protocol**: Full Model Context Protocol compatibility for external tool servers. Would allow connecting to any MCP-compatible tool provider.

### Implementation Order

1. ✅ **`crates/tools/` crate**: `Tool` trait, `ToolRegistry`, `ToolContext`, `ToolResult`
2. ✅ **Filesystem tools**: `read_file`, `write_file`, `delete_file`, `list_files`, `grep`
3. ✅ **Git tools**: `diff`, `status`
4. ✅ **Test runner tools**: `run_tests` (auto-detect cargo/npm/go)
5. ✅ **Shell tool**: `run_command` (allowlisted commands)
6. ✅ **Tool-use loop in agent-core**: `run_with_tools()` with text-based protocol
7. ✅ **Ollama native tool calling**: `chat_with_tools()` with `tools` API parameter
8. **Tree-sitter tools**: `rename_symbol`, `find_symbol`, `extract_signatures` (Rust first)
9. **LSP tools**: start rust-analyzer, query diagnostics/references/rename
10. **Web search tools**: `web_search`, `fetch_url`
11. **Wire into orchestrator**: runner uses `run_with_tools` instead of `run` for tool-capable models
12. **Dashboard**: show tool calls in agent detail (tool name, params, result, duration)

### Dependencies

```toml
# crates/tools/Cargo.toml
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
tree-sitter-typescript = "0.23"
tower-lsp = "0.20"        # LSP client
reqwest = "0.12"           # web fetch
```

### Milestone: "Tool-Using Agents"
Models choose between deterministic tools and LLM agents at each step. Renames take 2ms instead of 47s. Test failures surface immediately instead of after the quality gate. Web search provides context before code generation. LLM tokens are spent only where creativity is needed.

---

## Phase 8: Interactive Planning Before Execution (Future)

**Goal**: User reviews and iterates on the plan BEFORE agents start executing. The biggest model generates a plan, the user can refine it with free-text feedback, and only approved plans get executed. All plan versions are persisted.

### Why

Currently: submit task → daemon immediately plans and executes → 20+ min later, results may be wrong. User has no control over what agents will do, which files they'll touch, or how the goal is decomposed. A bad plan wastes significant compute on the 72B/33B models.

### New Status Flow

```
pending → planning → planned → approved → running → passed/failed
                       ↑          │
                       └──────────┘  (user feedback → re-plan)
```

### Data Model: `goal_plan` table

```rust
pub struct GoalPlan {
    pub id: Option<String>,
    pub run_id: String,              // FK to agent_run
    pub project: String,
    pub goal: String,
    pub version: u32,                // increments on each iteration
    pub sub_tasks: Vec<PlannedTask>,
    pub model_used: String,
    pub tokens_input: u32,
    pub tokens_output: u32,
    pub duration_ms: u64,
    pub user_feedback: Option<String>,
    pub status: String,              // "draft" | "approved" | "rejected"
    pub context_files: Vec<String>,  // files the planner read
    pub web_searches: Vec<String>,   // searches performed during planning
    pub reasoning: String,           // planner's explanation
    pub created_at: String,
}

pub struct PlannedTask {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub complexity: String,
    pub rationale: String,
}
```

### API Endpoints

- `POST /api/plan` — submit goal for planning only (no execution)
- `GET /api/plans/{run_id}` — get all plan versions for a run
- `POST /api/plans/{run_id}/feedback` — refine plan with user feedback
- `POST /api/plans/{run_id}/approve` — approve and start execution
- `POST /api/plans/{run_id}/edit` — user directly edits subtasks

### Daemon Behavior

The daemon handles new statuses:
- `planning`: generate plan via LLM, store as GoalPlan, set status to `planned`, STOP
- `planned`: wait for user action (approve/feedback/edit)
- `approved`: load approved plan, execute agents using it (skip re-planning)
- `pending`: legacy behavior (plan + execute immediately)

### Re-planning with Feedback

When user sends feedback, the planner prompt includes:
```
PREVIOUS PLAN (version 2):
[JSON of previous subtasks]

USER FEEDBACK:
"Remove the frontend tasks, focus only on backend. Also search for how 
other projects handle auth middleware before implementing."

Generate an improved plan addressing the feedback.
```

The model can also trigger web searches during planning to gather context.

### Frontend: Plan Review Page

New page at `/project/{name}/plan/{run_id}`:
- Goal description at top
- Version history (tabs)
- Subtask cards: description, files, complexity, rationale (editable)
- Planner's reasoning (collapsible)
- Feedback textarea
- Actions: "Refine Plan", "Approve & Run", "Edit Manually"

### Implementation Order

1. Add `GoalPlan` + `PlannedTask` to schema
2. Extend `RunStatus` with `Planning`, `Planned`, `Approved`
3. Add API endpoints
4. Update daemon to handle planning/approved statuses
5. Update planner to accept previous plan + feedback
6. Build plan review page in Leptos frontend
7. Add "Plan first" toggle to submit page

### Milestone: "Human-in-the-Loop Planning"
User submits a goal, reviews the AI's plan, iterates with feedback, and only approved plans execute. Plan history is persisted. No more wasted 20-minute runs on bad decompositions.

---

## Phase 9: Resilience Hardening (Future)

**Goal**: Fix critical failure modes identified in the system audit.

### Critical Fixes

1. **Ollama/Claude request timeouts** — add 5-minute hard timeout on all inference calls, 60s on embeddings
2. **Atomic task claiming** — use SurrealDB conditional update (`WHERE status = 'pending'`) to prevent duplicate execution
3. **Periodic zombie recovery** — run every 5 minutes (not just startup), check `last_activity_at` staleness
4. **SurrealDB reconnection** — exponential backoff reconnect, queue updates locally during outage

### High-Priority Fixes

5. **Git operation timeouts** — 30s timeout on push/clone/pull
6. **Worktree merge rollback** — `git reset --hard` if merge fails, report conflict to executor
7. **Frontend error display** — show API errors to user, only clear form on success
8. **Disk space monitoring** — add `max_disk_percent` to resource config, check before scheduling
9. **Result set pagination** — LIMIT on all DB queries, add offset/limit to API

### Medium-Priority Fixes

10. **NATS reconnection loop** — actually reconnect after disconnect instead of falling through
11. **SQL injection prevention** — parameterized queries in web-ui
12. **Executor backpressure** — semaphore on concurrent task spawns in daemon main loop
13. **Claude rate limit handling** — parse `retry-after` header, backoff instead of failing

### Implementation Order

1. Add timeout to Ollama/Claude HTTP clients (low effort, critical impact)
2. Implement atomic task claiming
3. Add periodic zombie recovery + heartbeat
4. SurrealDB reconnection with local queue
5. Git operation timeouts
6. Worktree merge rollback
7. Frontend error state
8. Disk monitoring + result pagination

### Milestone: "Production Resilient"
System recovers from daemon crashes, network partitions, database outages, and disk full conditions. No silent failures. Users always see what went wrong.

---

## Phase 10: WASI Tool Workers — Distributed, Sandboxed Tool Execution (Future)

**Goal**: Each tool runs as a WASI component on wasmCloud. Tools are sandboxed, distributable across the lattice, and hot-deployable. The daemon dispatches tool calls over NATS; any machine with the right capabilities can execute them.

### Why

Current tool execution has three problems:

1. **Locality**: Tools run on the daemon's machine. If the model runs on csatapaci but files are on the local machine, every `read_file` call crosses the network twice (tool call → daemon → read → result → model). Moving tool execution to where the data lives eliminates this.

2. **No sandboxing**: `run_command` and `write_file` have direct host access. A malicious model response could `rm -rf /` (we have an allowlist, but it's defense-in-depth). WASI components are sandboxed by default — they can only access explicitly granted capabilities.

3. **No scalability**: 10 parallel agents calling `run_tests` all compete for the same machine's CPU. With WASI workers distributed across the lattice, each `run_tests` call can run on a different machine.

### Architecture

```
Model (csatapaci, Ollama)
  │
  │ "call read_file(src/main.rs)"
  │
  ▼
Agent Loop (daemon, any machine)
  │
  │ Publishes tool call to NATS subject: swarm.tools.read_file
  │
  ▼
NATS (cluster)
  │
  │ Routed to nearest machine with file access
  │
  ▼
Tool Worker (WASI component on wasmCloud)
  │
  ├── wasi:filesystem → reads src/main.rs (sandboxed to repo dir)
  │
  │ Returns result via NATS reply
  │
  ▼
Agent Loop (receives result, feeds to model)
```

### WASI Component Per Tool Category

```
components/
├── tool-fs/              ← read_file, write_file, delete_file, list_files
│   ├── wit/tool-fs.wit   ← uses wasi:filesystem
│   └── src/lib.rs
├── tool-git/             ← git_diff, git_status, git_commit
│   ├── wit/tool-git.wit  ← uses wasi:cli/run (sandboxed to git commands)
│   └── src/lib.rs
├── tool-test/            ← run_tests
│   ├── wit/tool-test.wit ← uses wasi:cli/run (sandboxed to cargo/npm/go)
│   └── src/lib.rs
├── tool-search/          ← grep, ts_find, ts_rename, ts_signatures
│   ├── wit/tool-search.wit ← pure computation, no host access needed
│   └── src/lib.rs
├── tool-web/             ← web_search, fetch_url, search_crates
│   ├── wit/tool-web.wit  ← uses wasi:http/outgoing-handler
│   └── src/lib.rs
```

### WIT Interface for Tools

Each tool component implements a common interface:

```wit
package swarm:tools@0.1.0;

interface tool-executor {
    record tool-params {
        name: string,
        params-json: string,
        repo-path: string,
        project: string,
        timeout-ms: u64,
    }

    record tool-result {
        content: string,
        is-error: bool,
        duration-ms: u64,
    }

    execute: func(params: tool-params) -> tool-result;
}
```

### Dispatch: NATS-Based Tool Routing

The agent loop (daemon or WASI component) publishes tool calls to NATS:

```
Subject: swarm.tools.{tool_name}
Payload: { "params": {...}, "repo_path": "/tmp/alpha-swarm/repos/myproject", "timeout_ms": 60000 }
Reply: { "content": "...", "is_error": false, "duration_ms": 12 }
```

wasmCloud routes the message to the nearest component with the right capabilities:
- `tool-fs` must run on a machine with access to the repo directory
- `tool-web` can run anywhere with internet
- `tool-search` (tree-sitter, grep) can run anywhere with the repo data

### Capability Grants (Security)

Each WASI tool component gets minimal capabilities:

| Component | Capabilities Granted |
|-----------|---------------------|
| tool-fs | `wasi:filesystem` (read/write to repo dir only) |
| tool-git | `wasi:cli/run` (git binary only), `wasi:filesystem` |
| tool-test | `wasi:cli/run` (cargo/npm/go only), `wasi:filesystem` |
| tool-search | None (pure computation — tree-sitter runs in WASM) |
| tool-web | `wasi:http/outgoing-handler` (HTTP only) |

A compromised tool-web component cannot read files. A compromised tool-fs component cannot make network requests. Defense in depth.

### Migration Path from Current Architecture

1. **Phase 10a**: Keep existing `Tool` trait implementations, add NATS dispatch as an alternative execution backend. Tools can run locally OR via NATS.

```rust
enum ToolExecutor {
    Local(Box<dyn Tool>),
    Remote { nats: NatsClient, subject: String },
}
```

2. **Phase 10b**: Compile tool implementations to WASI components. Deploy on wasmCloud alongside existing web-ui component.

3. **Phase 10c**: Remove local tool execution from daemon. All tools go through NATS → WASI workers. Daemon becomes a pure orchestration loop.

### Hot Deployment

New tools can be deployed without restarting anything:
- Write a new WASI component implementing `swarm:tools/tool-executor`
- Deploy via `wash` to the lattice
- Agent loop discovers new tools via NATS subscription
- Model's tool prompt auto-updates with new tool descriptions

### Implementation Order

1. Define `swarm:tools` WIT interface
2. Implement `tool-search` WASI component (tree-sitter is pure computation, easiest to port)
3. Implement `tool-web` WASI component (HTTP only, no filesystem)
4. Add NATS tool dispatch to ToolRegistry (parallel to local execution)
5. Implement `tool-fs` WASI component with sandboxed filesystem
6. Implement `tool-git` and `tool-test` with sandboxed CLI
7. Remove local tool execution, all tools go through WASI workers
8. Hot-deployment: auto-register new tools from lattice

### Dependencies

```toml
# Each tool component
[dependencies]
wit-bindgen = "0.36"
serde_json = "1"
```

Tree-sitter compiles to WASM (it's C code with WASM support). The `tree-sitter-rust` grammar also compiles to WASM.

### Milestone: "Distributed Tool Execution"
Tools run as sandboxed WASI components across the wasmCloud lattice. Each machine contributes its capabilities (filesystem, network, CPU) to the swarm. New tools deployed without restart. Security via capability grants — no tool can access more than it needs.

---

## Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| WasmCloud provider development is complex/underdocumented | HIGH | Phase 0-1 work without WasmCloud; can always run agents as plain Rust binaries |
| Small models produce bad diffs | MEDIUM | Retry-with-escalation, structured output format, quality gate catches failures |
| SurrealDB vector search quality | LOW | Well-documented, fallback to pgvector if needed |
| NATS cluster ops overhead | LOW | Single-machine mode works through Phase 3; NATS is optional |
| Ollama OOM on 72B model | MEDIUM | Concurrency semaphore limits parallel agents; timeout kills hung requests |
| Tree-sitter grammar coverage | LOW | Start with Rust only; TypeScript/Go/Python grammars are well-maintained |
| LSP startup latency | MEDIUM | Keep server alive across tool calls; cache per-project |
| Concurrent daemon task claiming | HIGH | Atomic conditional update in SurrealDB (Phase 9) |
| WASI component cold start | LOW | wasmCloud pre-warms components; sub-10ms startup for most tools |
| Tree-sitter in WASM performance | LOW | C code compiles to WASM efficiently; parsing is CPU-bound, not I/O-bound |
| NATS tool dispatch latency | LOW | Sub-1ms for local NATS; <5ms across Tailscale; tool execution dominates |
