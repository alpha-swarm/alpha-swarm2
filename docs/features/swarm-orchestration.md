# Feature: Swarm Orchestration

## Scenario: Goal decomposes into non-overlapping sub-tasks
  Given a repo with src/math.rs and src/strings.rs
  When the planner decomposes "add functions to both modules"
  Then it creates 2+ SubTasks
  And no two tasks share the same file in their files list

## Scenario: Agents run on isolated git worktrees
  When the swarm creates worktrees for 2 agents
  Then each worktree is at /tmp/alpha-swarm/worktrees/{agent-id}
  And each has its own git branch agent/{agent-id}

## Scenario: Diffs from worktrees merge cleanly
  Given 2 agents modifying different files in separate worktrees
  When both complete successfully
  Then apply_diff_to_main succeeds for both
  And the main repo contains changes from both agents

## Scenario: Quality gate runs on merged result
  When the swarm completes all agents
  Then the quality gate runs on the main repo
  And SwarmResult.quality_passed reflects the gate outcome

## Scenario: Worktree cleanup on completion
  When the swarm finishes (pass or fail)
  Then all worktrees are removed from /tmp/alpha-swarm/worktrees/
  And all agent branches are deleted

## Scenario: Worktree cleanup on drop
  When a WorktreeManager is dropped
  Then its destructor removes all worktrees

## Scenario: Agent failure does not stop others
  Given a swarm with 3 tasks
  And agent 1 fails with an inference error
  When agents 2 and 3 succeed
  Then SwarmResult.results has 3 entries
  And result[0] has error, results[1] and [2] have agent_result

## Scenario: File discovery
  Given a repo with: src/main.rs, src/lib.rs, target/debug/foo, .git/config
  When discover_source_files runs
  Then it includes src/main.rs and src/lib.rs
  And excludes target/ and .git/
