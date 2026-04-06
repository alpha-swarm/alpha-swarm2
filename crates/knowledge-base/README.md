# knowledge-base

SurrealDB integration for persistent storage. Stores agent runs, plans, embeddings, and metrics.

## Tables

- **agent_run** — Task execution records (status, tokens, duration, diff, attempts, prompts)
- **goal_plan** — Planning iterations with sub-tasks, reasoning, user feedback
- **project** — Repository metadata
- **resource_snapshot** — Per-host CPU/RAM/disk metrics

## Key Features

- **Vector similarity search** — find similar past tasks via cosine similarity on embeddings
- **Task deduplication** — skip tasks already completed with high confidence
- **Error learning** — retrieve past failures for similar tasks to improve retry prompts
- **Plan versioning** — store all plan iterations with user feedback history
