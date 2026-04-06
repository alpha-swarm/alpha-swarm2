# agent-daemon

Distributed task executor. Watches for pending tasks via NATS KV, claims them atomically, executes agent swarms, and stores results.

## Task Flow

```
pending → planning → planned → approved → running → passed/failed
```

- **pending**: submitted via web UI, immediate execution
- **planning**: 72B model generates plan, stores GoalPlan, waits for approval
- **approved**: user approved plan, daemon executes with approved sub-tasks
- **running**: agents executing in parallel

## Coordination

- **NATS KV** (primary): `swarm-tasks` bucket for task queue, `swarm-leases` for atomic claiming (TTL=10min), `swarm-resources` for host snapshots
- **SurrealDB** (fallback): polls every 5s if NATS unavailable
- **Lease heartbeat**: every 2min, prevents expiry during long tasks
- **Zombie recovery**: tasks with no activity for 10min auto-fail

## Provider Integration

Git and test operations go through NATS services:
- `swarm.git.*` → git-provider (clone, worktree, PR creation)
- `swarm.test.*` → test-provider (cargo test, npm test)
- Falls back to local execution if providers unavailable
