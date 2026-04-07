Feature: Web Dashboard
  As a user
  I want to monitor and control the agent swarm via a web UI
  So that I can see what's happening and intervene when needed

  Scenario: View system overview
    Given the dashboard is running at http://localhost:3000
    When I visit the Overview page
    Then I see system status (Online/Offline)
    And the number of active agents
    And the number of available models
    And recent activity feed

  Scenario: Create a project
    Given I'm on the Projects page
    When I click "+ New Project"
    And fill in name, repo URL, branch, and description
    And click "Create Project"
    Then the project appears in the project list

  Scenario: Submit a task with plan-first mode
    Given I'm on the Submit page
    When I enter a task description
    And select a project
    And check "Plan first"
    And click "Submit for Planning"
    Then the task appears in the project's kanban as "Planning"

  Scenario: Review and approve a plan
    Given a task with status "planned"
    When I click "Review Plan" on the kanban card
    Then I see the plan review page with sub-tasks table
    And the planner's reasoning
    And the context files analyzed
    When I click "Approve & Run"
    Then agents start executing

  Scenario: View agent details
    Given a running or completed agent
    When I click on the agent row
    Then I see the Run Detail panel with:
      | Field | Description |
      | Status badge | passed/failed/running |
      | Model | which model was used |
      | Tokens | input/output count |
      | Duration | human-readable (e.g., "2m 34s") |
      | Started | relative time (e.g., "3m ago") |
      | Last Active | with zombie indicator (green/yellow/red) |
      | Attempts | per-attempt timeline with model, tokens, pass/fail |
      | Prompt | collapsible, full prompt sent |
      | Response | collapsible, full LLM response |
      | Diff | collapsible, unified diff output |

  Scenario: Clear all data
    Given noisy data from testing
    When I click "Clear All Data" on the Overview page
    And confirm the dialog
    Then all agent_run and project records are deleted
    And the dashboard shows empty state

  Scenario: Kanban board shows task progress
    Given tasks in various states
    Then the kanban shows columns:
      | Column | Status |
      | In Progress | running, planning, partial |
      | Completed | passed |
      | Failed | failed |
    And each card shows agent count, progress message, and expand for sub-agent table
