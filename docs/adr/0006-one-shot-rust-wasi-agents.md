# ADR-0006: One-Shot Agents as Rust WASI Components

## Status

Proposed

## Date

2026-04-05

## Context and Problem Statement

alpha-swarm needs worker agents that:
- Receive a task + context, produce a code diff + report
- Are stateless — all state lives in SurrealDB
- Run sandboxed inside WasmCloud
- Start fast and finish fast (one-shot, not long-running)
- Are small enough to distribute across machines efficiently

We need to choose the agent implementation language and execution model.

## Decision Drivers

- **WASI 0.2 support**: Must compile to wasm32-wasip2 with full networking and file I/O
- **Binary size**: Agents are distributed across the lattice — smaller is better
- **Cold start**: Agents spin up per-task — must be sub-second
- **Predictable performance**: No GC pauses, deterministic memory usage
- **Ecosystem**: Must integrate with WasmCloud provider model via WIT interfaces
- **Team expertise**: Entire stack is Rust

## Considered Alternatives

### Python Agents
- Fastest development speed, richest AI/ML ecosystem
- No wasm32-wasip2 support (componentize-py exists but limited)
- Large runtime, slow cold start in WASM
- GC unpredictability
- Would require running as native processes, not WASI components

### TypeScript/Deno Agents
- Good developer experience, async-first
- ComponentizeJS exists but adds JS runtime overhead (~5MB+ per component)
- GC pauses affect latency
- Poor fit for the all-Rust architecture

### Go Agents (TinyGo)
- TinyGo compiles to WASI but with significant standard library gaps
- Binary size larger than Rust WASI components
- Limited async support in TinyGo
- Separate toolchain from the rest of the project

### Rust WASI Components (chosen)
- wasm32-wasip2 is Tier 2 stable (since Rust 1.82)
- Smallest binary size (~1-3MB typical for an agent component)
- Sub-millisecond cold start in Wasmtime
- No GC — deterministic memory, predictable performance
- Same language as providers — shared types, single toolchain
- `wstd` crate provides async Rust standard library for WASM

## Decision Outcome

**Build all agents as Rust WASI components** targeting wasm32-wasip2.

### Agent Architecture

```
┌─────────────────────────────────────┐
│           Agent Component           │
│         (wasm32-wasip2)             │
│                                     │
│  1. Receive task + context          │
│  2. Query knowledge base            │
│     (similar past work, errors)     │
│  3. Build prompt with context       │
│  4. Call Ollama via provider         │
│  5. Parse response → file edits     │
│  6. Apply edits via VirtFS          │
│  7. Request quality gate check      │
│  8. Report result to knowledge base │
│                                     │
│  Imports:                           │
│    alpha-swarm:ollama/inference     │
│    alpha-swarm:vfs/repository       │
│    alpha-swarm:quality/gate         │
│    alpha-swarm:knowledge/base       │
└─────────────────────────────────────┘
```

### Agent Types (same binary, different system prompts)

| Agent Type | System Prompt Focus | Typical Model |
|---|---|---|
| lint-fixer | Fix specific lint/clippy warnings | qwen2.5-coder:7b |
| test-writer | Write tests for untested functions | qwen2.5-coder:7b |
| refactorer | Refactor function/module by instruction | deepseek-coder-v2:16b |
| feature-adder | Implement a described feature | deepseek-coder-v2:16b |
| bug-fixer | Fix a described bug with test | Claude API / large local |

### Agent Lifecycle

1. **Spawned** by orchestrator with task ID
2. **Claims** task in SurrealDB (atomic — prevents double-work)
3. **Queries** knowledge base for relevant context
4. **Executes** single inference call (one-shot, no multi-turn)
5. **Applies** changes to worktree
6. **Requests** quality gate validation
7. **Reports** result (diff, pass/fail, errors) to knowledge base
8. **Exits** — component is unloaded

Total lifetime: seconds to low minutes, depending on inference time.

## Consequences

### Positive
- Tiny components (~1-3MB) — fast to distribute across lattice
- Sub-millisecond instantiation — no cold start penalty
- Deterministic resource usage — no GC surprises
- Single toolchain for entire project (Rust)
- Type-safe WIT interface contracts between agents and providers
- Agents are truly stateless — crash recovery is just re-spawning

### Negative
- Rust development is slower than Python/TypeScript for prototyping
- Limited runtime reflection — agent behavior configured via system prompts, not dynamic code
- wasm32-wasip2 ecosystem is still maturing — some crates may not compile
- Debugging WASM components is harder than native binaries

### Risks
- If a critical crate doesn't compile to wasm32-wasip2, may need to move that logic to a provider
- One-shot model may be insufficient for complex tasks — mitigate with retry-with-escalation (small model fails → larger model retries)
