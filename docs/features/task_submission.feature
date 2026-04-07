Feature: Task Submission
  As a user
  I want to submit coding tasks to the swarm
  So that AI agents can work on my repository

  Scenario: Submit a task for immediate execution
    Given a project "my-project" with repo URL "https://github.com/user/repo"
    When I submit a task "Add error handling to auth module" to project "my-project"
    Then the task status should be "pending"
    And the daemon should claim the task via NATS KV
    And the task status should change to "running"
    And the orchestrator should decompose the goal into sub-tasks
    And agents should execute in parallel

  Scenario: Submit a task with plan-first mode
    Given a project "my-project" with repo URL "https://github.com/user/repo"
    When I submit a planning task "Refactor the auth module" to project "my-project"
    Then the task status should be "planning"
    And the orchestrator should generate a GoalPlan with sub-tasks
    And the task status should change to "planned"
    And the kanban card should show "Review Plan" button
    When I approve the plan
    Then the task status should change to "approved"
    And agents should execute using the approved plan

  Scenario: Submit a task with feedback loop
    Given a planned task with 5 sub-tasks
    When I send feedback "Remove the frontend tasks, focus on backend only"
    Then the task status should change to "planning"
    And the orchestrator should re-plan with the feedback context
    And a new GoalPlan version should be created

  Scenario: Task claiming prevents duplicate execution
    Given two daemons watching NATS KV
    When a task becomes "pending"
    Then only one daemon should claim it via NATS KV create
    And the other daemon should skip it
