# ADR-0005: VirtFS Repo Access via WasmCloud Provider and Git Worktrees

## Status

Proposed

## Date

2026-04-05

## Context and Problem Statement

Multiple agents work on the same repository simultaneously. Each agent needs:
- Read access to the full repo state
- Write access to make changes without conflicting with other agents
- Isolation — one agent's failed changes must not corrupt another's working copy
- The ability to produce clean diffs against the base branch

We need a strategy for providing file-level access to repositories from within sandboxed WASI components.

## Decision Drivers

- **Isolation**: Each agent must have its own working copy
- **Efficiency**: Full git clones per agent are wasteful — repos can be large
- **Diff production**: Agents must produce clean, mergeable diffs
- **WASI compatibility**: File access must work through wasi:filesystem interface
- **Conflict prevention**: Orchestrator should assign non-overlapping file sets when possible

## Considered Alternatives

### Shared NFS/CIFS Mount
- Simple, well-understood
- No isolation — agents can overwrite each other's work
- File locking is unreliable across machines
- No built-in diff generation

### Full Git Clone Per Agent
- Perfect isolation
- Expensive: large repos take significant disk and time to clone
- Network-dependent if cloning from remote
- Wasteful when agents only modify a few files

### Object Storage (S3-like)
- Good for distributed access
- No git integration — must build diff/merge logic from scratch
- High latency for file-level operations
- Overkill for local-first system

### Git Worktrees (chosen)
- Lightweight: shares .git directory, only checks out working tree
- Perfect isolation: each worktree is an independent working copy on its own branch
- Built-in diff: `git diff` against base branch produces clean unified diffs
- Fast creation: near-instant for repos already cloned
- Standard git merge workflow for combining agent outputs

## Decision Outcome

**Use git worktrees** managed by the VirtFS capability provider.

### Workflow

1. **Repo registration**: Orchestrator tells VirtFS provider about a repo (local path or remote URL)
2. **Worktree creation**: For each agent task, provider creates a git worktree:
   ```
   git worktree add /tmp/alpha-swarm/worktrees/{agent-id} -b agent/{agent-id}
   ```
3. **File access**: Agent component accesses files through wasi:filesystem, mediated by the provider
4. **Diff extraction**: When agent completes, provider runs `git diff main..agent/{agent-id}` to extract changes
5. **Cleanup**: Provider removes worktree after diff is captured:
   ```
   git worktree remove /tmp/alpha-swarm/worktrees/{agent-id}
   ```

### Directory Layout

```
/tmp/alpha-swarm/
  repos/
    {repo-hash}/              # Main clone (shared .git)
  worktrees/
    {agent-id-1}/             # Agent 1's isolated working copy
    {agent-id-2}/             # Agent 2's isolated working copy
```

### Conflict Prevention

The orchestrator assigns file sets to agents when decomposing tasks. The VirtFS provider enforces this:
- Agent can read any file in the worktree
- Agent can only write to files in its assigned set
- If an agent attempts to write outside its set, the provider returns an error

### Merge Strategy

The orchestrator merges agent diffs sequentially:
1. Apply agent 1's diff to main
2. Run quality gate
3. If pass, apply agent 2's diff
4. If conflict, re-run agent 2 with updated base (retry)

## Consequences

### Positive
- Near-zero overhead per agent (worktrees share object store)
- Clean git diffs — standard tooling for review and merge
- Natural conflict detection via git merge
- Agents work on real file trees — no custom file format or API

### Negative
- Worktree creation requires the repo to be cloned locally first
- Large repos with many worktrees consume disk (working tree files are not shared)
- Git operations are synchronous — provider must serialize concurrent worktree operations on the same repo
- File-level write restrictions add complexity to the provider

### Risks
- Very large repos (monorepos) may strain disk with many concurrent worktrees — mitigate with sparse checkout
- Git worktree bugs with concurrent operations — mitigate with mutex in the provider
