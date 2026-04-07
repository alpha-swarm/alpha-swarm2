Feature: System Resilience
  As the swarm infrastructure
  I want to handle failures gracefully
  So that the system recovers without data loss

  Scenario: Ollama inference timeout
    Given an inference request to Ollama
    When Ollama doesn't respond within 10 minutes
    Then the request times out with a clear error
    And the agent marks the run as failed
    And the retry loop can try again with backoff

  Scenario: Daemon crash recovery
    Given a daemon processing a task
    When the daemon crashes
    Then the NATS KV lease expires after 10 minutes
    And the periodic zombie recovery marks the task as failed
    And the task can be retried

  Scenario: SurrealDB unavailable
    Given the daemon is running
    When SurrealDB goes down
    Then the daemon continues watching NATS KV
    And operations that need SurrealDB fail gracefully
    When SurrealDB comes back
    Then the daemon reconnects on next operation

  Scenario: NATS unavailable fallback
    Given the daemon starting up
    When NATS is unreachable
    Then the daemon falls back to SurrealDB polling (every 5s)
    And task execution continues without NATS events

  Scenario: Worktree merge conflict rollback
    Given an agent applied edits in a worktree
    When merging back to main causes a conflict
    Then the merge is rolled back (git checkout .)
    And the error is reported
    And subsequent agents are not affected

  Scenario: Claude API rate limiting
    Given an inference request to Claude API
    When Claude returns 429 (rate limited)
    Then the error includes the retry-after duration
    And the retry loop waits accordingly

  Scenario: Disk space monitoring
    Given disk usage is above 90%
    When a new task is submitted
    Then the daemon defers scheduling
    And logs "Disk too full"

  Scenario: Atomic task claiming prevents duplicates
    Given a task with status "pending" in SurrealDB
    When two daemons try to claim it simultaneously
    Then only one succeeds (UPDATE ... WHERE status='pending')
    And the other daemon's update matches 0 rows
    And the task runs exactly once
