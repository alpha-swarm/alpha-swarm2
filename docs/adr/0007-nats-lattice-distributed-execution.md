# ADR-0007: NATS Lattice for Distributed Multi-Machine Execution

## Status

Proposed

## Date

2026-04-05

## Context and Problem Statement

alpha-swarm must run across multiple machines to:
- Leverage GPUs on different hosts for different models
- Distribute agent workloads based on available resources
- Scale horizontally — add machines to increase throughput
- Tolerate host failures without losing in-flight work

We need a distributed execution strategy that enables agents and providers to communicate transparently across machine boundaries.

## Decision Drivers

- **Self-forming**: New machines should join the system with minimal configuration
- **Resource-aware**: Scheduling must account for GPU availability, loaded Ollama models, and memory
- **No single point of failure**: The system must survive individual machine failures
- **Transparent to agents**: Agent code should not know or care which machine it runs on
- **Operational simplicity**: Must be manageable by a small team, not a dedicated platform team

## Considered Alternatives

### Manual SSH Orchestration
- Simple, no infrastructure dependencies
- No automatic failover or load balancing
- Requires custom scripting for every operational task
- Does not scale beyond a handful of machines

### Kubernetes Cluster
- Industry standard for distributed workloads
- Heavy operational burden: etcd, API server, CNI, storage drivers
- Requires dedicated infrastructure or cloud provider
- Overkill for a system that already uses WasmCloud

### Consul + Nomad
- Lighter than Kubernetes, good service discovery
- Separate system from WasmCloud — double the operational complexity
- Must bridge Nomad scheduling with WasmCloud component model
- Additional dependency to maintain

### Custom Gossip Protocol
- Maximum flexibility
- Years of engineering to make production-reliable
- Must solve: membership, failure detection, state replication
- Reinventing what NATS already provides

### WasmCloud Lattice over NATS (chosen)
- Built into WasmCloud — no additional system to operate
- NATS provides: pub/sub, request/reply, queue groups, JetStream persistence
- Lattice is self-forming: hosts discover each other via NATS subscriptions
- Queue subscriptions enable automatic load balancing across hosts
- If a host dies, NATS re-routes pending requests to other hosts
- Public key cryptography for multi-tenant security on shared NATS

## Decision Outcome

**Use WasmCloud's built-in lattice** with a multi-node NATS cluster for distributed execution.

### Network Topology

```
Machine A (GPU)              Machine B (CPU)              Machine C (CPU)
┌──────────────────┐        ┌──────────────────┐        ┌──────────────────┐
│ WasmCloud Host   │        │ WasmCloud Host   │        │ WasmCloud Host   │
│                  │        │                  │        │                  │
│ Providers:       │        │ Providers:       │        │ Providers:       │
│  - Ollama (GPU)  │        │  - Ollama (CPU)  │        │  - Ollama (CPU)  │
│  - VirtFS        │        │  - VirtFS        │        │  - VirtFS        │
│  - QualityGate   │        │  - QualityGate   │        │  - KnowledgeBase │
│                  │        │                  │        │                  │
│ Agents: N        │        │ Agents: M        │        │ Agents: K        │
└───────┬──────────┘        └───────┬──────────┘        └───────┬──────────┘
        │                           │                           │
        └───────────────────────────┴───────────────────────────┘
                              NATS Cluster
                         (3-node, JetStream)
```

### NATS Cluster Setup

- **Minimum 3 nodes** for quorum (can co-locate with WasmCloud hosts)
- **JetStream enabled** — required for lattice metadata persistence
- **Ports**: 4222 (client), 6222 (cluster), 8222 (monitoring)
- **Storage**: Local SSD per NATS node (never NAS/NFS)
- **Replication**: R3 for lattice KV buckets

### Resource-Aware Scheduling

Hosts declare capabilities via labels:

```toml
# Machine A (GPU host)
[labels]
gpu = "true"
gpu_model = "rtx4090"
vram_gb = "24"
ollama_models = "codellama:34b,qwen2.5-coder:32b"

# Machine B (CPU only)
[labels]
gpu = "false"
ram_gb = "64"
ollama_models = "qwen2.5-coder:7b,deepseek-coder-v2:16b"
```

The orchestrator uses these labels to schedule agents:
- Tasks requiring large models → hosts with those models loaded
- GPU-intensive inference → GPU hosts
- Simple tasks → any available host

### Failure Handling

1. **Host dies**: NATS detects via heartbeat timeout, re-routes pending wRPC calls to other hosts running the same provider
2. **Agent crashes**: Orchestrator detects via task timeout in SurrealDB, re-spawns on any available host
3. **NATS node dies**: Cluster continues with 2/3 nodes (quorum maintained)
4. **SurrealDB unreachable**: Agents degrade to stateless mode (no knowledge base queries, still functional)

### Deployment Commands

```bash
# Start NATS cluster (each machine)
nats-server -c nats-cluster.conf

# Start WasmCloud host (each machine)
wasmcloud --nats-host nats://cluster-addr:4222 --label gpu=true

# Deploy application
wash app deploy wadm.yaml

# Verify lattice
wash get inventory
wash get links
```

## Consequences

### Positive
- Zero additional infrastructure — NATS is already required by WasmCloud
- Self-healing — host failures handled automatically by the lattice
- Transparent to agents — same code runs on one machine or twenty
- Horizontal scaling — add a machine, start a host, it joins automatically
- Resource-aware scheduling via host labels
- Built-in observability via OpenTelemetry

### Negative
- NATS cluster requires 3 nodes minimum for production resilience
- JetStream storage adds disk requirements per node
- Network partitions can cause split-brain — mitigated by NATS quorum
- Debugging distributed issues harder than single-machine

### Risks
- WasmCloud Q3 2025 scheduling changes may alter how host labels work — monitor roadmap
- NATS cluster operations (upgrades, certificate rotation) add operational burden
- Cross-machine latency affects agent performance if providers are on different machines than agents — mitigate by co-locating providers with their agents via scheduling constraints
