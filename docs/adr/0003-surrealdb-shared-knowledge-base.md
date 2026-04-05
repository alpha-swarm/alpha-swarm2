# ADR-0003: SurrealDB as Shared Knowledge Base

## Status

Proposed

## Date

2026-04-05

## Context and Problem Statement

Multiple agents working on the same codebase need to:
- Know what other agents (past and parallel) have done or are doing
- Avoid repeating the same mistakes a previous agent made
- Share conversation history, diffs, and error reports
- Perform semantic search over past work (e.g., "find similar diffs to this one")

We need a database that supports both structured document storage (tasks, agent runs, errors) and vector similarity search (semantic matching on diffs and conversations).

## Decision Drivers

- **Schemaless**: Agent output formats will evolve rapidly — rigid schemas slow iteration
- **Vector search**: Must support embedding storage and cosine/euclidean similarity queries
- **Hybrid queries**: Combine structured filters (e.g., "failed runs on file X") with vector similarity
- **Rust SDK**: First-class Rust client required — the entire stack is Rust
- **Embeddable / local-first**: Must run on a single machine without cloud infrastructure
- **Graph-like relations**: Agents, tasks, repos, and diffs have complex relationships

## Considered Alternatives

### PostgreSQL + pgvector
- Battle-tested, excellent Rust ecosystem (sqlx, diesel)
- pgvector adds vector similarity search
- Requires schema migrations — slow iteration on evolving agent output
- Heavy for a local-first system (though SQLite+pgvector alternative exists)
- No native graph queries

### Redis + RediSearch
- Fast, well-known, good Rust client
- RediSearch adds full-text and vector search
- In-memory by default — data loss risk for conversation history
- Limited query expressiveness for complex agent relationships
- Not designed for document storage

### Qdrant / Milvus (standalone vector DB)
- Best-in-class vector search performance
- No structured document storage — need a second database alongside
- Two databases to operate and keep in sync
- Overkill for the scale of a local agent system

### SurrealDB (chosen)
- Schemaless document model — store any JSON structure, evolve freely
- Built-in HNSW vector indexing — millisecond similarity search
- Hybrid search via `search::rrf()` — fuses BM25 full-text with vector similarity
- Graph relations via record links — model agent→task→diff→error relationships naturally
- Stable Rust SDK (surrealdb crate, production-ready)
- Runs embedded or as a standalone server — scales from laptop to cluster
- SurrealQL is expressive (SQL-like with graph traversal)

## Decision Outcome

**Use SurrealDB** as the central knowledge store for alpha-swarm.

### Data Model

```
task        — { id, description, repo, status, claimed_by, created_at }
agent_run   — { id, task, agent_type, model_used, started_at, finished_at, status }
conversation— { id, agent_run, messages: [...], embedding: [f32] }
diff        — { id, agent_run, file_path, unified_diff, embedding: [f32] }
error       — { id, agent_run, error_type, message, stack_trace, embedding: [f32] }
```

### Key Queries

1. **Task locking**: `UPDATE task SET claimed_by = $agent WHERE claimed_by = NONE`
2. **Find similar past diffs**: Vector search on diff embeddings with cosine similarity
3. **Find past errors on this file**: `SELECT * FROM error WHERE agent_run.task.repo = $repo AND diff.file_path = $path`
4. **What are parallel agents doing**: `SELECT * FROM agent_run WHERE status = 'running' AND task.repo = $repo`
5. **Avoid repeated mistakes**: Vector search on error embeddings similar to current task context

### Embeddings

Use `fastembed` crate (no API key needed) to generate embeddings locally for diffs, conversations, and errors. HNSW index on embedding fields for fast similarity search.

## Consequences

### Positive
- Single database for all storage needs — no sync between separate document and vector stores
- Schemaless evolution — agent output format can change without migrations
- Graph traversal for complex queries (agent→task→diff chain)
- Hybrid search combines keyword and semantic matching
- Runs locally with zero cloud dependencies

### Negative
- SurrealDB is younger than PostgreSQL — smaller community, fewer battle-tested production deployments
- Rust SDK may lag behind SurrealDB server releases
- HNSW index lives in RAM — memory pressure with large embedding collections
- Learning curve for SurrealQL (SQL-like but distinct)

### Risks
- If SurrealDB proves unstable at scale, fallback to PostgreSQL + pgvector (structured) with a separate embedding table
- fastembed model quality may not be sufficient — can swap to Ollama's `/api/embeddings` endpoint as alternative
