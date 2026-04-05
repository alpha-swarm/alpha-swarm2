# alpha-swarm Infrastructure

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          CLIENT BOUNDARY                                │
│                                                                         │
│   CLI ─────┐                                                            │
│   Dashboard ┼──► gateway ──► swarm/control ──► project/manager          │
│   MCP Server┘       │                              │                    │
│                     │                              │                    │
│              events/bus ◄──────────────────────────┘                    │
└──────────────────────┬──────────────────────────────────────────────────┘
                       │
┌──────────────────────┼──────────────────────────────────────────────────┐
│                 ORCHESTRATION BOUNDARY                                   │
│                      │                                                  │
│              ┌───────▼────────┐                                         │
│              │  orchestrator  │                                         │
│              │  (goal → tasks)│                                         │
│              └──┬──┬──┬──┬───┘                                         │
│                 │  │  │  │                                              │
│    ┌────────────┘  │  │  └────────────┐                                │
│    ▼               ▼  ▼               ▼                                │
│ ┌──────┐  ┌──────┐ ┌──────┐  ┌──────┐                                 │
│ │agent │  │agent │ │agent │  │agent │   (N one-shot workers)           │
│ │  #1  │  │  #2  │ │  #3  │  │  #N  │                                 │
│ └──┬───┘  └──┬───┘ └──┬───┘  └──┬───┘                                 │
└────┼─────────┼────────┼─────────┼──────────────────────────────────────┘
     │         │        │         │
┌────┼─────────┼────────┼─────────┼──────────────────────────────────────┐
│    │    INFERENCE BOUNDARY      │                                      │
│    │         │        │         │                                      │
│    └─────────┴────┬───┴─────────┘                                      │
│                   ▼                                                    │
│         ┌──────────────────┐                                           │
│         │ inference-router │   routes by complexity + availability     │
│         └────┬─────────┬───┘                                           │
│              │         │                                               │
│         ┌────▼───┐ ┌───▼────┐                                          │
│         │ claude │ │ ollama │   ... future: openai, local llama.cpp    │
│         │ prov.  │ │ prov.  │                                          │
│         └────┬───┘ └───┬────┘                                          │
│              │         │                                               │
└──────────────┼─────────┼──────────────────────────────────────────────┘
               │         │
┌──────────────┼─────────┼──────────────────────────────────────────────┐
│         PROVIDER BOUNDARY    (native Rust, host OS access)             │
│              │         │                                               │
│         ┌────▼───┐ ┌───▼────┐ ┌──────────┐ ┌──────────┐              │
│         │ virtfs │ │quality │ │knowledge │ │ events   │              │
│         │ prov.  │ │gate p. │ │base prov.│ │ bus prov.│              │
│         └────┬───┘ └───┬────┘ └────┬─────┘ └────┬─────┘              │
└──────────────┼─────────┼──────────┼────────────┼──────────────────────┘
               │         │          │            │
┌──────────────┼─────────┼──────────┼────────────┼──────────────────────┐
│         EXTERNAL SERVICE BOUNDARY │            │                       │
│              ▼         ▼          ▼            ▼                       │
│         ┌──────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐              │
│         │ git  │ │ cargo/   │ │ SurrealDB│ │   NATS   │              │
│         │ repos│ │ npm/go   │ │ :8000    │ │  :4222   │              │
│         └──────┘ └──────────┘ └──────────┘ └──────────┘              │
│                                                                        │
│    ┌──────────┐  ┌──────────┐                                          │
│    │ Anthropic│  │  Ollama  │                                          │
│    │ API      │  │  :11434  │                                          │
│    └──────────┘  └──────────┘                                          │
└────────────────────────────────────────────────────────────────────────┘
```

## Physical Deployment (Current Setup)

```
┌──────────────────────────┐          ┌──────────────────────────┐
│  LOCAL MACHINE            │          │  CSATAPACI                │
│                           │          │  M2 Max · 96GB · 3.6TB   │
│  Tailscale: 100.79.38.122│◄────────►│  Tailscale: 100.81.10.8  │
│                           │   NATS   │                           │
│  ┌─────────────────────┐  │  cluster │  ┌─────────────────────┐  │
│  │ NATS node 1         │  │◄────────►│  │ NATS node 2         │  │
│  │ :4222 :6222 :8222   │  │          │  │ :4222 :6222 :8222   │  │
│  └─────────────────────┘  │          │  └─────────────────────┘  │
│                           │          │                           │
│  ┌─────────────────────┐  │          │  ┌─────────────────────┐  │
│  │ SurrealDB :8000     │  │          │  │ Ollama :11434       │  │
│  │ (primary data store)│  │          │  │ · qwen2.5-coder:7b  │  │
│  └─────────────────────┘  │          │  │ · deepseek-coder:33b│  │
│                           │          │  │ · codellama:34b     │  │
│  ┌─────────────────────┐  │          │  └─────────────────────┘  │
│  │ WasmCloud Host      │  │          │                           │
│  │ role: orchestrator   │  │          │  ┌─────────────────────┐  │
│  │                      │  │          │  │ WasmCloud Host      │  │
│  │ Components:          │  │          │  │ role: inference      │  │
│  │  · gateway           │  │          │  │ ram: 96gb            │  │
│  │  · orchestrator      │  │          │  │                      │  │
│  │  · inference-router  │  │          │  │ Components:          │  │
│  │                      │  │          │  │  · agent-worker (N)  │  │
│  │ Providers:           │  │          │  │                      │  │
│  │  · claude-provider   │  │          │  │ Providers:           │  │
│  │  · knowledge-base    │  │          │  │  · ollama-provider   │  │
│  │  · virtfs            │  │          │  │  · virtfs            │  │
│  │  · quality-gate      │  │          │  │  · quality-gate      │  │
│  └─────────────────────┘  │          │  └─────────────────────┘  │
└──────────────────────────┘          └──────────────────────────┘

┌──────────────────────────┐
│  malna (RPi)              │
│  8GB RAM · 512GB SSD      │
│  Tailscale: TBD           │
│                           │
│  ┌─────────────────────┐  │
│  │ NATS node 3         │  │   ← 3rd quorum node
│  │ :4222 :6222         │  │
│  └─────────────────────┘  │
│                           │
│  ┌─────────────────────┐  │
│  │ SurrealDB (replica) │  │   ← persistent store
│  │ :8000               │  │
│  └─────────────────────┘  │
│                           │
│  ┌─────────────────────┐  │
│  │ WasmCloud Host      │  │
│  │ role: infra+quality  │  │
│  │                      │  │
│  │ Components:          │  │
│  │  · quality-gate      │  │   ← runs build/test
│  └─────────────────────┘  │
└──────────────────────────┘
```

## Error Boundaries

### Boundary 1: External Services

```
┌──────────────────────────────────────────────────────────────┐
│  EXTERNAL SERVICE BOUNDARY                                    │
│                                                               │
│  Ollama (:11434)                                              │
│  ├─ DOWN  → inference-router marks backend unavailable,      │
│  │          falls back to Claude API                          │
│  ├─ SLOW  → per-request timeout (configurable, default 120s) │
│  ├─ MODEL MISSING → swarm-error::not-found                   │
│  └─ OOM   → inference-failed, router escalates to smaller    │
│             model or different backend                        │
│                                                               │
│  Claude API                                                   │
│  ├─ 401/403   → swarm-error::inference-failed (bad API key)  │
│  ├─ 429       → rate limited, backoff + retry                │
│  ├─ 500/529   → overloaded, fallback to Ollama if available  │
│  └─ TIMEOUT   → inference-failed                             │
│                                                               │
│  SurrealDB (:8000)                                            │
│  ├─ DOWN → knowledge-base-provider returns io-error           │
│  │         system enters DEGRADED mode:                       │
│  │         agents skip knowledge queries, still functional    │
│  ├─ QUERY TIMEOUT → io-error with context                    │
│  └─ NAMESPACE MISSING → not-found                            │
│                                                               │
│  NATS (:4222)                                                 │
│  ├─ DOWN → lattice partitions, no new scheduling             │
│  │         in-flight agents finish locally                     │
│  ├─ PARTITION → 2-node cluster has no quorum                 │
│  │   each side operates independently until reconnect         │
│  └─ RECONNECT → automatic, NATS client handles               │
│                                                               │
│  Git repos                                                    │
│  ├─ LOCK CONTENTION → virtfs retries with backoff            │
│  ├─ CORRUPT REPO → io-error, project marked as error         │
│  └─ DISK FULL → io-error, checked before worktree creation   │
└──────────────────────────────────────────────────────────────┘
```

### Boundary 2: Inference Router

```
┌──────────────────────────────────────────────────────────────┐
│  INFERENCE BOUNDARY                                           │
│                                                               │
│  The router is the single point where backend failures are    │
│  absorbed. Agents never see backend-specific errors.          │
│                                                               │
│  Routing strategy:                                            │
│    simple task  → prefer Ollama (qwen2.5-coder:7b)           │
│    medium task  → prefer Ollama (deepseek-coder:33b)         │
│    complex task → prefer Claude API                           │
│                                                               │
│  Fallback chain:                                              │
│    preferred backend fails                                    │
│         │                                                     │
│         ▼                                                     │
│    try next backend with equivalent model                     │
│         │                                                     │
│         ▼                                                     │
│    try any backend with any model                             │
│         │                                                     │
│         ▼                                                     │
│    return swarm-error::inference-failed                       │
│                                                               │
│  Circuit breaker per backend:                                 │
│    5 consecutive failures → open circuit for 30s              │
│    during open → fast-fail, skip to next backend              │
│    after 30s → half-open, probe with one request              │
│    probe succeeds → close circuit, resume normal              │
│                                                               │
│  Health events emitted:                                       │
│    backend.claude.up / backend.claude.down                    │
│    backend.ollama.up / backend.ollama.down                    │
│    inference.fallback (when preferred backend unavailable)    │
└──────────────────────────────────────────────────────────────┘
```

### Boundary 3: Providers

```
┌──────────────────────────────────────────────────────────────┐
│  PROVIDER BOUNDARY                                            │
│                                                               │
│  Each provider:                                               │
│  ├─ ISOLATES external service failures                        │
│  │   Never panics — all errors become result<_, swarm-error> │
│  ├─ RETRIES internally (3x with exponential backoff)         │
│  │   before returning error to component                      │
│  ├─ HEALTH CHECK via WasmCloud lifecycle                      │
│  │   unhealthy provider → WasmCloud restarts it               │
│  └─ Emits events on state changes                             │
│                                                               │
│  virtfs-provider:                                             │
│  ├─ Git lock contention → retry with backoff                  │
│  ├─ Worktree create fails → io-error                          │
│  ├─ Disk full → io-error (checked before worktree creation)  │
│  └─ Concurrent repo access → mutex per repo                   │
│                                                               │
│  quality-gate-provider:                                       │
│  ├─ Build tool not found → quality-gate-failed                │
│  ├─ Process timeout (300s) → kill + quality-gate-failed       │
│  ├─ Non-zero exit → structured check-result with              │
│  │   stdout/stderr captured (not a swarm-error)               │
│  └─ OOM during build → quality-gate-failed                    │
│                                                               │
│  knowledge-base-provider:                                     │
│  ├─ SurrealDB down → reconnect loop (5s interval)            │
│  ├─ After 30s unreachable → emit degraded event               │
│  │   Components skip knowledge queries, still work            │
│  └─ Query timeout (10s) → io-error                            │
└──────────────────────────────────────────────────────────────┘
```

### Boundary 4: Orchestration

```
┌──────────────────────────────────────────────────────────────┐
│  ORCHESTRATION BOUNDARY                                       │
│                                                               │
│  orchestrator:                                                │
│  ├─ PLANNING FAILS (LLM error) →                             │
│  │   goal-status = failed, event emitted                      │
│  ├─ AGENT FAILS (task-status = failed) →                      │
│  │   retry strategy:                                          │
│  │   1. Retry same model (transient?)        max 1 retry     │
│  │   2. Retry with larger model              max 1 retry     │
│  │   3. Mark task failed, continue others                     │
│  │   4. If >50% tasks fail → fail the goal                   │
│  ├─ MERGE CONFLICT →                                          │
│  │   re-run conflicting agent with updated base               │
│  │   max 2 retries, then fail that task                       │
│  └─ AGENT TIMEOUT (300s) →                                    │
│     task unclaimed in SurrealDB after TTL                      │
│     another agent can pick it up                               │
│                                                               │
│  agent-worker:                                                │
│  ├─ WASM TRAP (panic, OOM) →                                  │
│  │   WasmCloud catches it, task stays unclaimed                │
│  │   orchestrator re-dispatches after timeout                  │
│  ├─ INFERENCE BAD OUTPUT (unparseable) →                       │
│  │   task-status = failed with error message                   │
│  │   stored in knowledge-base for future avoidance             │
│  └─ QUALITY GATE FAILS →                                      │
│     diff discarded, error stored in knowledge-base             │
│     task retried with error context fed to next attempt        │
└──────────────────────────────────────────────────────────────┘
```

### Boundary 5: Client

```
┌──────────────────────────────────────────────────────────────┐
│  CLIENT BOUNDARY                                              │
│                                                               │
│  gateway:                                                     │
│  ├─ All swarm-errors → structured response to client          │
│  ├─ Event bus down → client gets stale status                 │
│  │   (poll-based, client handles staleness)                   │
│  └─ Invalid request → swarm-error::internal                   │
│                                                               │
│  CLI:                                                         │
│  ├─ NATS unreachable →                                        │
│  │   "Cannot reach swarm. Is NATS running?"                   │
│  ├─ Timeout waiting for goal → show partial status            │
│  └─ Ctrl-C → goal keeps running (async by design)            │
│     "Goal abc123 still running. Check with:                   │
│      alpha-swarm status abc123"                                │
└──────────────────────────────────────────────────────────────┘
```

## Error Propagation Flow

```
                    HAPPY PATH
                    ──────────
submit-goal ──► orchestrator.plan ──► agents[N] ──► merge ──► done
                                         │
                    ERROR PATHS           │
                    ───────────           │
                                         ▼
                              ┌─────────────────────┐
                              │ Agent fails?         │
                              └──────┬──────────────┘
                                     │
                    ┌────────────────┬┴───────────────┐
                    ▼                ▼                 ▼
               inference         quality           wasm trap
               failed            failed            (panic/OOM)
                    │                │                 │
                    ▼                ▼                 ▼
               ┌────────────────────────────────────────┐
               │ Store error in knowledge base          │
               │ (embedding for future avoidance)       │
               └───────────────┬────────────────────────┘
                               │
                      ┌────────▼────────┐
                      │ Retry logic:    │
                      │ 1. Same model   │
                      │ 2. Bigger model │
                      │ 3. Give up task │
                      └────────┬────────┘
                               │
                  ┌────────────┼────────────┐
                  ▼            ▼            ▼
               retried      task failed   >50% failed
               (loop back)  (continue     (fail entire
                             others)       goal)
```

## Graceful Degradation Matrix

```
┌──────────────┬───────────┬──────────┬────────┬──────────────────┐
│ What's Down  │ Ollama    │ Claude   │SurrealDB│ NATS             │
│              │           │ API      │         │                  │
├──────────────┼───────────┼──────────┼─────────┼──────────────────┤
│ Can plan?    │ YES (1)   │ YES (2)  │ YES     │ NO               │
│ Can run      │ YES (1)   │ YES (2)  │ YES (3) │ NO               │
│  agents?     │           │          │         │                  │
│ Can query    │ YES       │ YES      │ NO      │ YES              │
│  knowledge?  │           │          │         │                  │
│ Can emit     │ YES       │ YES      │ YES     │ NO               │
│  events?     │           │          │         │                  │
│ Can run      │ YES       │ YES      │ YES     │ NO               │
│  quality     │           │          │         │                  │
│  checks?     │           │          │         │                  │
├──────────────┼───────────┼──────────┼─────────┼──────────────────┤
│ System state │ DEGRADED  │ DEGRADED │DEGRADED │ PARTITIONED      │
│              │ fallback  │ fallback │ no      │ in-flight agents │
│              │ to Claude │ to Ollama│ knowledge│ finish locally   │
└──────────────┴───────────┴──────────┴─────────┴──────────────────┘

(1) Falls back to Claude API via inference-router
(2) Falls back to Ollama via inference-router
(3) Agents work but skip knowledge queries (no past context, no dedup)
```

## Startup & Shutdown

### Startup Order

```
1. NATS             ← must be first (lattice backbone)
   ├─ verify: nats server check
   │
2. SurrealDB        ← can start in parallel with step 1
   ├─ verify: surreal isready
   │
3. Ollama           ← can start in parallel with step 1
   ├─ verify: curl http://localhost:11434/
   │
4. WasmCloud Host   ← requires NATS
   ├─ verify: wash get hosts
   │
5. wash app deploy wadm.yaml
   ├─ verify: wash get inventory
   └─ verify: wash get links
```

### Shutdown Order (Reverse)

```
1. wash app undeploy alpha-swarm     ← drain agents (wait 60s max)
2. Stop WasmCloud Host
3. Stop Ollama                       ← or leave running
4. Stop SurrealDB
5. Stop NATS                         ← last
```

### Scripts

```bash
# Local machine
./scripts/infra-up-local.sh          # Start NATS + SurrealDB + WasmCloud
./scripts/infra-down-local.sh        # Graceful shutdown

# csatapaci (runs commands via SSH)
./scripts/infra-up-csatapaci.sh      # Start NATS + SurrealDB + Ollama + WasmCloud
./scripts/infra-down-csatapaci.sh    # Graceful shutdown

# Both machines
./scripts/infra-status.sh            # Check all services + lattice
```

## Configuration

### Environment Variables

```bash
# NATS
ALPHA_SWARM_NATS_URL=nats://127.0.0.1:4222

# SurrealDB
ALPHA_SWARM_SURREALDB_URL=ws://127.0.0.1:8000
ALPHA_SWARM_SURREALDB_NS=alpha_swarm
ALPHA_SWARM_SURREALDB_DB=swarm
ALPHA_SWARM_SURREALDB_USER=root
ALPHA_SWARM_SURREALDB_PASS=root

# Ollama
ALPHA_SWARM_OLLAMA_URL=http://csatapaci:11434
ALPHA_SWARM_OLLAMA_TIMEOUT_SECS=120

# Claude API
ALPHA_SWARM_CLAUDE_API_KEY=sk-ant-...
ALPHA_SWARM_CLAUDE_MODEL=claude-sonnet-4-20250514

# Provider behavior
ALPHA_SWARM_RETRY_MAX=3
ALPHA_SWARM_RETRY_BACKOFF_MS=1000
ALPHA_SWARM_CIRCUIT_BREAKER_THRESHOLD=5
ALPHA_SWARM_CIRCUIT_BREAKER_RESET_SECS=30

# Agent behavior
ALPHA_SWARM_TASK_TIMEOUT_SECS=300
ALPHA_SWARM_GOAL_FAIL_THRESHOLD=0.5
```

### WasmCloud Host Labels

```toml
# Local machine
[labels]
alpha-swarm-role = "orchestrator"
alpha-swarm-gpu = "false"

# csatapaci
[labels]
alpha-swarm-role = "inference"
alpha-swarm-gpu = "false"
alpha-swarm-ram-gb = "96"
alpha-swarm-models = "deepseek-coder:33b,codellama:34b,qwen2.5-coder:7b"
```

### NATS Cluster

```
Node 1 (local):     100.79.38.122:4222 (client) / :6222 (cluster)
Node 2 (csatapaci): 100.81.10.8:4222   (client) / :6222 (cluster)
Cluster name:       alpha-swarm
JetStream:          enabled on both nodes
```
