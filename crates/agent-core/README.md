# agent-core

One-shot code modification agent. Reads source files, calls an LLM, parses structured edits, and applies them to the repository.

## Agent Modes

- **`run()`** — Single LLM call. Sends files + task → receives `<<<EDIT>>>` / `<<<CREATE>>>` / `<<<DELETE>>>` blocks → applies edits.
- **`run_with_tools()`** — Iterative tool-use loop. Model calls tools (read_file, grep, run_tests, etc.) between inference steps. Uses Ollama native tool calling API with automatic fallback to `run()` for models that don't support tools.
- **`run_with_retry()`** — Retry with quality gate. On failure: retry same model, then escalate to larger model.

## Agent Specializations

`AgentType` enum customizes the system prompt:
- General, LintFixer, TestWriter, Refactorer, FeatureAdder, BugFixer

## Knowledge Integration

With `KnowledgeConfig`, the agent:
- Checks SurrealDB for similar past tasks (skip if already done)
- Retrieves past error context for retry prompts
- Stores prompt, response, attempts, and embeddings
- Links sub-agent runs to parent via `parent_run_id`

## Edit Format

```
<<<EDIT path/to/file.rs
--- OLD
exact lines to replace
--- NEW
replacement lines
>>>

<<<CREATE path/to/new_file.rs
file contents
>>>

<<<DELETE path/to/file.rs
>>>
```
