# Integration Tests

Tests that exercise the full pipeline. Require running infrastructure.

## Prerequisites

- NATS on :4223
- SurrealDB on :8001
- Ollama (local or csatapaci)

## Unit tests (no infra needed)

```bash
cargo test --lib                           # All 61+ unit tests
cd dashboard && pnpm run test              # Dashboard unit tests
```

## Integration tests

```bash
cargo test -p swarm-orchestrator --test planner_integration  # Planning only
cargo test -p agent-core --test agent_integration           # Agent with tools
```
