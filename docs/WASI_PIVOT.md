# WASI-Native Architecture Pivot

## Vision

Everything except inference calls runs as WASI components. Inference stays local (Ollama) with an abstraction layer that *could* support cloud providers later — but only local for now to avoid runaway costs.

## Current vs Target

| Component | Currently | Target |
|-----------|-----------|--------|
| Agent core (edit/plan) | Native Rust binary | WASI component |
| Tool registry | Native Rust, disk I/O | WASI + wasi:filesystem |
| Orchestrator/planner | Native Rust binary | WASI component |
| Quality gate | Native Rust, shell commands | WASI + wasi:cli |
| Knowledge store | SurrealDB (native client) | WASI + wasi:http (REST) |
| Event bus | async-nats (native) | WASI + wasi:messaging |
| File workspace | git2 + local clone | WASI + wasi:blobstore (NATS) |
| Inference client | reqwest to Ollama | **Abstraction trait** via wasi:http |
| Web UI | WASI (done) | Keep |
| Frontend | Leptos WASM (done) | Keep |

## Inference Abstraction

### Trait

```rust
trait InferenceProvider: Send + Sync {
    async fn chat(&self, messages: &[Message], options: &Options) -> Result<Response>;
    async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>>;
    fn name(&self) -> &str;
    fn supports_tools(&self) -> bool;
}
```

### Local Providers Only (for now)

| Provider | Backend | Use case |
|----------|---------|----------|
| `OllamaProvider` | Ollama HTTP API | Primary — runs on csatapaci (96GB) |
| `LlamaCppProvider` | llama.cpp server | Alternative local — lighter weight |

The trait is designed so cloud providers (Claude, OpenAI) can be added later with cost controls, but they're **not enabled by default**.

### Router

```rust
InferenceRouter {
    providers: Vec<Box<dyn InferenceProvider>>,
    
    // Pick by: model availability, VRAM, queue depth
    fn select(&self, complexity: Complexity) -> &dyn InferenceProvider;
    
    // Fallback: preferred Ollama host → other Ollama hosts → error
    fn chat_with_fallback(&self, ...) -> Result<Response>;
}
```

### Config

```toml
# Local providers only — no cloud API keys needed
[[providers]]
type = "ollama"
url = "http://100.81.10.8:11434"  # csatapaci M2 Max 96GB
models = ["qwen2.5-coder:32b", "qwen2.5-coder:14b"]
priority = 1

[[providers]]
type = "ollama"
url = "http://localhost:11434"     # local machine
models = ["qwen2.5-coder:7b"]
priority = 2  # Fallback
```

## WASI Component Architecture

### WIT Interfaces

```wit
// alpha-swarm:inference/provider
interface provider {
    record message { role: string, content: string }
    record options { model: string, max-tokens: u32, temperature: option<f32> }
    record response { content: string, model: string, tokens-in: u32, tokens-out: u32 }
    
    chat: func(messages: list<message>, opts: options) -> result<response, string>;
    embed: func(model: string, text: string) -> result<list<f32>, string>;
}

// alpha-swarm:workspace/files  
interface files {
    read-file: func(path: string) -> result<string, string>;
    write-file: func(path: string, content: string) -> result<_, string>;
    list-files: func(pattern: string) -> result<list<string>, string>;
    file-exists: func(path: string) -> bool;
    extract-diff: func() -> result<string, string>;
    commit: func(message: string) -> result<string, string>;
}

// alpha-swarm:agent/core
interface agent {
    record task { description: string, files: list<string>, complexity: string }
    record edit { path: string, old-content: string, new-content: string }
    record agent-result { edits: list<edit>, applied: bool, diff: string }
    
    run: func(task: task) -> result<agent-result, string>;
}
```

### Component Graph

```
[Web UI]  ─── wasi:http ──→  SurrealDB
    │
    v
[Orchestrator]  ─── alpha-swarm:inference/provider ──→  [Inference Provider]
    │                                                        │
    │                                                    Ollama HTTP
    v
[Agent Worker]  ─── alpha-swarm:workspace/files ──→  [Workspace Provider]
    │                                                     │
    │                                                 NATS Object Store
    ├── alpha-swarm:inference/provider ──→  [Inference Provider]
    │
    v
[Tool Worker]  ─── wasi:filesystem (sandboxed, read-only repo)
               ─── wasi:cli (cargo, test commands)
```

### wasmCloud Capability Providers

| Provider | Interface | Backend |
|----------|-----------|---------|
| `inference-provider` | `alpha-swarm:inference/provider` | Ollama HTTP (local only) |
| `workspace-provider` | `alpha-swarm:workspace/files` | NATS object store + git2 |
| `nats-messaging` | `wasi:messaging` | NATS (built-in) |
| `http-client` | `wasi:http/outgoing-handler` | HTTP (built-in) |
| `blobstore-nats` | `wasi:blobstore` | NATS object store |

## Migration Path

### Phase 1: Inference Abstraction (native Rust)
- Extract `InferenceProvider` trait
- Support multiple Ollama hosts (csatapaci + local)
- Model routing by VRAM/queue depth
- Config-driven provider list

### Phase 2: Workspace via NATS blobstore
- Replace local git clone with NATS object store
- Read-through cache: NATS → git HEAD
- Diff extraction from NATS keys vs git tree
- Agent works distributed — no local filesystem needed

### Phase 3: Agent Worker as WASI component
- Compile agent-core to `wasm32-wasip2`
- Import `alpha-swarm:inference/provider` + `alpha-swarm:workspace/files`
- Tool calls via WASI interfaces
- Runs on any wasmCloud host

### Phase 4: Full WASI stack
- Orchestrator as WASI component
- Quality gate via `wasi:cli` provider
- Event bus via `wasi:messaging`
- Everything sandboxed except Ollama/SurrealDB/NATS servers

## What Stays Native (servers)

- **Ollama** — GPU inference server, HTTP API
- **SurrealDB** — database, HTTP/WS API
- **NATS** — messaging, native protocol
- **cargo/git** — dev tools, invoked via `wasi:cli`
