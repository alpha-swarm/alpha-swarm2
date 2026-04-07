Feature: Distributed Coordination
  As the swarm system
  I want tasks distributed across multiple machines
  So that work is parallelized and resources are utilized efficiently

  Scenario: NATS KV task claiming across machines
    Given daemons running on picur, csatapaci, and malna
    And all connected to the same NATS cluster
    When a new task is submitted
    Then the first daemon to call KV.create() claims the task
    And other daemons skip it (KV.create fails for existing key)
    And the claiming daemon executes the task

  Scenario: Lease-based heartbeat prevents zombie tasks
    Given a daemon has claimed a task with a NATS KV lease (TTL=10min)
    When the daemon sends heartbeats every 2 minutes
    Then the lease stays active
    When the daemon crashes
    Then the lease expires after 10 minutes
    And the task becomes available for another daemon

  Scenario: Resource-aware scheduling
    Given daemon on picur with CPU at 85%
    And daemon on csatapaci with CPU at 20%
    When a new task is submitted
    Then picur daemon should defer (CPU above 80% threshold)
    And csatapaci daemon should claim it

  Scenario: 3-node NATS cluster quorum
    Given NATS nodes on picur (4248), csatapaci (6222), malna (6222)
    When one node goes down
    Then the remaining 2 nodes maintain quorum
    And JetStream KV operations continue working
    And task coordination is uninterrupted

  Scenario: Git provider via NATS
    Given git-provider running on picur (swarm.git.*)
    When a daemon calls ensure_repo("project", "https://github.com/...")
    Then the request goes via NATS to the git-provider
    And the git-provider clones/updates the repo locally
    And returns the repo path

  Scenario: Tool dispatch to WASI workers
    Given tool-search running on malna
    When an agent calls ts_find("symbol", "content")
    Then the request goes via NATS to malna
    And tool-search executes tree-sitter AST query
    And returns the result to the agent
