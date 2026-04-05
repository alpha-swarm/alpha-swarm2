# ADR-0001: Use WasmCloud as the Orchestration Layer

## Status

Proposed

## Date

2026-04-05

## Context and Problem Statement

alpha-swarm needs an orchestration layer that can:
- Run sandboxed agent workloads across multiple machines
- Provide capability-based security (agents should not have unrestricted OS access)
- Support hot-swapping and scaling agents without downtime
- Enable a self-forming mesh network that agents join and leave dynamically

We need to choose between container orchestration (Kubernetes), job schedulers (Nomad), or WebAssembly-native runtimes.

## Decision Drivers

- **Security by default**: Agents execute LLM-generated code — they must be sandboxed
- **Lightweight**: Agents are short-lived one-shot tasks, not long-running services — container overhead is wasteful
- **Distributed-first**: Must work across heterogeneous machines (laptops, servers, edge) without central control plane
- **Rust ecosystem**: The entire stack is Rust — native WASI component support is critical
- **Cold start**: Agents spin up frequently — sub-second start times required

## Considered Alternatives

### Kubernetes + Custom Controllers
- Mature, well-understood, massive ecosystem
- Heavy: etcd, API server, kubelet per node — overkill for agent workloads
- Container cold start (seconds) too slow for one-shot agents
- Designed for long-running services, not ephemeral tasks

### HashiCorp Nomad
- Lighter than K8s, supports batch jobs well
- Has WASM task driver (experimental)
- Still requires server/client architecture with leader election
- Limited WASI 0.2 support

### Bare NATS Pub/Sub + Custom Runtime
- Maximum flexibility, minimal dependencies
- Must build everything: scheduling, health checks, capability management
- No component model — agents would be plain binaries with no sandboxing

### WasmCloud (chosen)
- Built on NATS lattice — self-forming mesh, no central control plane
- WASI 0.2 component model with deny-by-default capability security
- Sub-millisecond cold start for WASM components
- Native Rust toolchain (wasm32-wasip2 is Tier 2 stable)
- Capability providers handle OS-level operations (filesystem, networking, external tools)

## Decision Outcome

**Use WasmCloud** as the orchestration layer for alpha-swarm.

Agents compile to WASI components and run on WasmCloud hosts connected via a NATS lattice. All OS-level operations (Ollama inference, git operations, build toolchain) are exposed through native capability providers, not directly by agents.

The lattice provides:
- Automatic service discovery and load balancing via NATS queue subscriptions
- Cross-machine agent scheduling without a central scheduler
- Self-healing — if a host dies, pending work redistributes to surviving hosts
- Resource tagging — hosts declare capabilities (GPU, models, memory) for scheduling

## Consequences

### Positive
- Agents are sandboxed by default — LLM-generated code cannot escape the WASM sandbox
- Near-zero cold start for agent components
- Distributed execution is built-in, not bolted on
- Provider model cleanly separates agent logic from OS integration

### Negative
- WasmCloud provider development has a learning curve and less documentation than K8s operators
- Components cannot spawn subprocesses — all external tool access requires building native providers
- WASI filesystem access is virtualized — need custom provider for real file I/O
- Smaller community than Kubernetes — fewer off-the-shelf solutions

### Risks
- WasmCloud v2 runtime changes (Q3 2025 roadmap) may require migration
- If provider development proves too complex, fallback is running agents as plain Rust binaries orchestrated by NATS directly (Phase 0-1 of the plan work without WasmCloud)
