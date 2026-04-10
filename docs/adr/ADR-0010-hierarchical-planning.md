# ADR-0010: Hierarchical Planning and Task Composability

## Status

Proposed

## Context

The current planner makes a single LLM call to decompose a goal into 1-5 flat tasks. This works for small goals ("add a doc comment", "update README") but fails for complex goals that span multiple crates, require new files, or have task dependencies.

Users want to submit goals like:
- "Add OAuth2 to the web-ui"
- "Port the inference client to use wasi:http instead of reqwest"
- "Create a new WASI component for event streaming"

These require multi-step reasoning, code understanding across files, and ordered execution.

## Decision Drivers

- Agent reliability decreases with task complexity
- LLM context windows limit how much code can be reasoned about at once
- Parallel execution is only safe when tasks are independent
- Current 14b/32b local models have weaker reasoning than cloud models
- The system should work with any model, not just the best ones

## Options Considered

### Option A: Hierarchical Sub-Planning (Tree Decomposition)

```
Goal → Level 1 planner → [Sub-goals]
  Each sub-goal → Level 2 planner → [Tasks]
    Each task → Agent → Edits
```

**Pros:**
- Handles arbitrarily complex goals
- Each planning level has focused context (not 164 files)
- Natural dependency ordering (parent completes before children)
- Sub-goals can be reviewed/approved independently
- Enables "plan first, execute later" workflow
- Aligns with how humans decompose work

**Cons:**
- More LLM calls = more latency (each level adds 30-60s)
- Planning errors compound — bad L1 plan → all L2 plans wrong
- Model swap between planner and agent at each level
- Harder to debug — which level failed?
- Risk of over-decomposition (10 sub-goals × 5 tasks = 50 agent runs)
- Requires dependency graph + topological sort
- State management: sub-goals need to see results of prior sub-goals
- **The models we have (14b/32b) aren't good enough for reliable multi-level planning**

### Option B: Flat Planning with Larger Context

Keep single-level planning but improve it:
- Send file content excerpts (not just names) to the planner
- Use RAG (embeddings) to select relevant files for the goal
- Increase context window (32K+ with qwen2.5-coder:32b)
- Better prompt engineering for the planner

**Pros:**
- Simpler — no tree structure, no dependency management
- Fewer LLM calls — faster total execution
- Easier to debug (single plan, single execution)
- Works well with current model quality
- Proven: 8 PRs merged with flat planning

**Cons:**
- Limited to ~5 tasks per goal
- Can't handle goals spanning 10+ files
- No dependency ordering between tasks
- Context window limits file understanding

### Option C: Composable Task Pipelines

Instead of hierarchical planning, define reusable task templates:

```toml
[[pipeline.add_wasi_component]]
steps = [
  { task = "create_cargo_toml", template = "component" },
  { task = "create_wit_world", depends_on = "create_cargo_toml" },
  { task = "implement_handler", depends_on = "create_wit_world" },
  { task = "add_to_workspace", depends_on = "create_cargo_toml" },
  { task = "update_wadm", depends_on = "implement_handler" },
]
```

**Pros:**
- Deterministic — no LLM planning failures
- Reusable across projects
- Clear dependency graph
- Fast (no planning LLM calls)
- Easy to test and debug
- Works with any model quality

**Cons:**
- Must define templates manually for each workflow
- Not flexible — can't handle novel goals
- Doesn't leverage LLM reasoning
- Maintenance burden for template library
- Mixing templates with LLM-generated tasks is complex

### Option D: Agent-Driven Iteration (No Explicit Planning)

Skip planning entirely. Single agent reads code, makes changes, runs tests, iterates:

```
Goal → Agent reads relevant files → Makes edit → Runs tests
  → If tests fail → Agent reads error → Fixes → Repeats
  → If tests pass → PR
```

**Pros:**
- Simplest architecture — no planner, no decomposition
- Agent has full context of what it changed
- Natural error correction loop
- Works well for small-medium tasks
- This is how Claude Code / Cursor / Copilot agents work

**Cons:**
- Single agent bottleneck (no parallelism)
- Context window fills up with iteration history
- Can't handle truly large changes
- No plan review/approval step
- Harder to track progress

## Recommendation

**Option B (improved flat planning) for now, with Option D (agent iteration) as the execution model.**

Rationale:
1. **Our models aren't reliable enough for hierarchical planning.** gemma4:26b can't even produce valid JSON. qwen2.5-coder:32b produces good plans but takes 45s per call. Multi-level planning would take minutes of just planning.

2. **The biggest wins come from better execution, not better planning.** The agent's edit application (fuzzy matching, format compliance) is the bottleneck, not the plan quality.

3. **Flat planning + iteration is the proven path.** 8 PRs merged with single-level planning. The failures were all in execution (model output parsing, body size limits), not planning.

4. **Sub-planning can be added later** when models improve. The architecture supports it — `SubTask` already has `complexity` which could trigger sub-planning for `Complex` tasks.

## Future: When to Add Hierarchical Planning

Add sub-planning when ALL of these are true:
- [ ] Local model reliably produces valid JSON plans (>95% success rate)
- [ ] Model inference is fast enough (<10s per planning call)
- [ ] A goal requires >5 coordinated file changes
- [ ] Tasks have real dependencies (not just parallel edits)

## Composability: What We Should Do Instead

Instead of sub-planning, invest in:

1. **Better file selection** — Use embeddings/RAG to pick the 5-10 most relevant files for a goal, not all 164. The planner sees focused context.

2. **Agent iteration** — Let the agent make multiple edit rounds on the same files. Read → edit → check → fix → done. Like how a human developer works.

3. **Task templates** — For common operations (add component, add crate, add test), provide structured templates that don't need LLM planning.

4. **Result chaining** — After a task completes, feed its diff to the next task as context. "The previous agent added X, now add Y that uses it."

## Consequences

- Keep single-level planning (proven, reliable)
- Improve planner context (RAG-based file selection)
- Add agent iteration loop (edit → test → fix)
- Define task templates for common workflows
- Revisit hierarchical planning when local models reach GPT-4 quality
