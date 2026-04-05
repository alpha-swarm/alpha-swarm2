# ADR-0004: Native Capability Providers for OS-Level Operations

## Status

Proposed

## Date

2026-04-05

## Context and Problem Statement

WasmCloud's security model prevents WASI components from:
- Spawning subprocesses (cannot call `ollama`, `git`, `cargo` directly)
- Accessing the host filesystem without virtualization
- Making arbitrary system calls

Yet alpha-swarm agents need to: call Ollama for inference, read/write repository files, run build/test/lint toolchain, and query SurrealDB. We need an architecture that bridges WASI component sandboxing with real OS-level operations.

## Decision Drivers

- **Security preservation**: Do not bypass WasmCloud's sandboxing — it's the reason we chose it
- **Clean separation**: Agent logic (what to change) must be separated from system integration (how to execute)
- **Testability**: Providers should be independently testable without running full WasmCloud
- **Reusability**: Providers should be generic enough for different agent types
- **Rust native**: Providers run as native Rust binaries on the host

## Considered Alternatives

### Bypass sandboxing (WASIX or custom runtime)
- WASIX supports subprocess spawning but WasmCloud is not a WASIX runtime
- Modifying the runtime defeats the security model
- Non-portable — breaks WasmCloud's capability-based architecture

### Sidecar services (plain HTTP microservices)
- Agents call sidecars via HTTP for OS operations
- Works but loses WasmCloud's capability linking, observability, and lifecycle management
- Must manage sidecar deployment separately from WasmCloud

### WasmCloud Native Capability Providers (chosen)
- Rust binaries that run on the host alongside WasmCloud
- Expose operations to WASI components via wRPC (WasmCloud's RPC protocol)
- Components explicitly link to providers — capability-based security maintained
- Managed by WasmCloud lifecycle (start, stop, scale, health checks)
- Full access to host OS — can spawn processes, read filesystem, open sockets

## Decision Outcome

**Build four native capability providers** in Rust, each wrapping a specific OS integration:

### 1. Ollama Provider
- Wraps Ollama HTTP API (localhost:11434)
- Exports: `list_models()`, `show_model(name)`, `generate(model, prompt, options)`, `chat(model, messages, options)`, `running_models()`
- WIT interface: `alpha-swarm:ollama/inference`

### 2. VirtFS Provider
- Wraps git operations and filesystem access
- Exports: `clone_repo(url)`, `create_worktree(repo, branch)`, `read_file(path)`, `write_file(path, content)`, `apply_diff(path, diff)`, `list_files(path, glob)`
- Manages git worktrees for agent isolation
- WIT interface: `alpha-swarm:vfs/repository`

### 3. QualityGate Provider
- Wraps build toolchain execution
- Exports: `run_check(repo_path, toolchain)`, `run_tests(repo_path, test_tier)`, `run_fmt(repo_path)`, `run_lint(repo_path)`
- Supports toolchain config per repo (Cargo, npm, go, make)
- Test tiers: unit, integration, e2e
- WIT interface: `alpha-swarm:quality/gate`

### 4. KnowledgeBase Provider
- Wraps SurrealDB client
- Exports: `store_run(agent_run)`, `query_similar(embedding, threshold)`, `claim_task(task_id)`, `get_parallel_runs(repo)`, `store_error(error)`
- WIT interface: `alpha-swarm:knowledge/base`

### Provider Development Strategy

Each provider is developed and tested as a standalone Rust binary first (Phase 0-1), then wrapped as a WasmCloud provider (Phase 2). This ensures:
- Core logic works before WasmCloud integration
- Integration tests run without WasmCloud
- Fallback path: run providers as plain services if WasmCloud integration proves too complex

## Consequences

### Positive
- Clean security boundary — agents never touch the OS directly
- Each provider is independently testable and deployable
- Providers can be updated without recompiling agents
- WasmCloud manages provider lifecycle (health checks, restarts, scaling)
- Same providers work across machines in the lattice

### Negative
- Four providers to build and maintain — significant upfront engineering
- wRPC serialization overhead for every OS call (negligible vs. inference time)
- WasmCloud provider SDK has a learning curve
- WIT interface design requires upfront thought — changes require component recompilation

### Risks
- Provider SDK documentation may be insufficient — mitigate with WasmCloud community engagement
- If provider development takes too long, agents can call the underlying services directly as plain Rust binaries (bypass WasmCloud for that phase)
