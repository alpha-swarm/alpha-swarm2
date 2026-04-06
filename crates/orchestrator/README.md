# swarm-orchestrator

Decomposes high-level goals into parallel sub-tasks, runs agents in isolated git worktrees, merges results, and validates with quality gates.

## Flow

1. **Plan**: LLM decomposes goal into `Vec<SubTask>` (each with description, files, complexity)
2. **Worktrees**: creates isolated git worktree per agent (`/tmp/alpha-swarm/worktrees/{agent-id}`)
3. **Execute**: agents run in parallel via `tokio::task::JoinSet` (bounded by semaphore)
4. **Merge**: diffs applied back to main repo sequentially (rollback on conflict)
5. **Quality gate**: cargo fmt/clippy/build/test on merged result

## Concurrency

`max_concurrent_agents` (default: 2) limits parallel agents to avoid OOM when running large models. Controlled via `alpha-swarm.toml` `[resources]` section.

## NATS Tool Dispatch

Optional `async_nats::Client` enables remote tool execution. Tools dispatched to WASI workers via `swarm.tools.{name}` subjects.
