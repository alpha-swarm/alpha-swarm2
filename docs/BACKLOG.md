# Backlog — toward "usable & valuable"

Two lists. **Loop-eligible** = small, single-file, edit-shaped tasks the local-model
swarm does reliably; these are seeded into the `autopilot_goal` queue and drained
continuously by the autopilot loop (`[autopilot] continuous = true`), each
quality-gated (`cargo fmt/check/test`) so only passing changes count.
**Human / architect** = new files, cross-crate refactors, or design — out of the
loop (local-model reliability ceiling: edits well, struggles to create files).

## Loop-eligible (seeded into the queue)

- [ ] `rvindex`: remove the unnecessary `mut` on the `flush` closure binding
      (`crates/knowledge-base/src/rvindex.rs` ~L32) — clears the `unused_mut` warning.
- [ ] `config`: add a `#[cfg(test)] mod tests` asserting the defaults
      (`AutopilotConfig`/`WassetteConfig` `enabled == false`, `OllamaConfig.keep_alive == "-1"`).
- [ ] `config`: add `WassetteConfig::is_enabled(&self) -> bool` accessor + doc comment.
- [ ] Doc-comment pass: add `///` docs to undocumented `pub` items in a chosen small crate.

## Human / architect (NOT loop-eligible)

- [ ] Continuous-loop **self-refill**: a guarded gap-goal that generates the next
      small tasks (needs a task-generation path + runaway guards).
- [ ] Parallel runs: per-run workspace isolation (the `task-1` dir collision,
      `memtree.rs`) + multi-slot/per-project execution lock.
- [ ] Run quality gate on the disk workspace (`runner.rs` ~L963 TODO).
- [ ] Wire the Wassette fs/fetch tools into the agent `ToolRegistry` by default
      (currently config-gated, off).
- [ ] A real front-end to submit + watch goals (TUI/CLI/dashboard polish).
- [ ] `OLLAMA_MAX_LOADED_MODELS=4` on csatapaci for sustained 4-model warmth.

## How the loop runs

1. `[autopilot] enabled = true`, `continuous = true`, `auto_approve = true`.
2. Seed: `nats req swarm.db.autopilot.queue '{"project":"alpha-swarm2","goal":"<task>"}'`.
3. The daemon drains back-to-back; each run plans (70b) → edits (qwen2.5-coder:32b)
   → quality gates → `passed` merges its diff, failures requeue/abort (repo untouched).
