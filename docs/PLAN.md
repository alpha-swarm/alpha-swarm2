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

### Tasks
1. Set up 3-node NATS cluster with JetStream (can be 3 laptops, VMs, or cloud instances)
2. Start WasmCloud hosts on each node, all joining the same lattice
3. Resource-aware scheduling: tag hosts with capabilities (GPU, available models, memory)
4. Agents float to hosts that have the required Ollama model loaded
5. SurrealDB runs as a shared service accessible from all nodes
6. Build `alpha-swarm-cli` for submitting tasks and watching progress

### Checkpoint 4
```
TEST: 3 machines in the lattice. Submit a task from machine A.
      Orchestrator on machine A decomposes task.
      Some agents run on machine B (has GPU + codellama),
      others on machine C (has qwen2.5-coder).
VERIFY: wash get inventory shows hosts on all 3 machines.
VERIFY: wash get links shows cross-machine provider links.
VERIFY: Agent on machine B used a model only available on machine B.
VERIFY: SurrealDB queries from all machines return consistent results.
VERIFY: Killing one host causes agents to reschedule on remaining hosts.
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
```

Each phase is **independently useful**. You can stop at any phase and have a working system.

---

## Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| WasmCloud provider development is complex/underdocumented | HIGH | Phase 0-1 work without WasmCloud; can always run agents as plain Rust binaries |
| Small models produce bad diffs | MEDIUM | Retry-with-escalation, structured output format, quality gate catches failures |
| SurrealDB vector search quality | LOW | Well-documented, fallback to pgvector if needed |
| NATS cluster ops overhead | LOW | Single-machine mode works through Phase 3; NATS is optional |
