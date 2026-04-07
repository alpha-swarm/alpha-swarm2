Feature: Interactive Planning
  As a user
  I want to review and refine plans before agents execute
  So that I can control what the agents will do and avoid wasted compute

  Background:
    Given a project "swarm" with repo URL "https://github.com/alpha-swarm/alpha-swarm2"
    And the daemon is running and connected to NATS

  # --- Submission ---

  Scenario: Submit a task with "Plan first" mode
    When I go to the Submit page
    And I enter task "Add error handling to the parser crate"
    And I select project "swarm"
    And "Plan first" is checked (default)
    And I click "Submit for Planning"
    Then I should be redirected to the Plan Review page
    And I should see a spinner saying "Generating plan..."
    And the task status in the database should be "planning"

  Scenario: Submit a task for immediate execution
    When I go to the Submit page
    And I uncheck "Plan first"
    And I click "Submit & Execute"
    Then I should be redirected to the project page
    And the task should start executing immediately

  # --- Plan Generation ---

  Scenario: Plan is generated successfully
    Given I submitted a task with "Plan first"
    When the daemon picks up the planning task
    Then it should call the orchestrator model to decompose the goal
    And store a GoalPlan with version 1 in SurrealDB
    And set the task status to "planned"
    And the Plan Review page should auto-refresh and show the plan

  Scenario: Plan generation fails
    Given I submitted a task with "Plan first"
    When the planner model is unavailable
    Then the daemon should try fallback models
    And if all fail, set status to "failed" with error message
    And the Plan Review page should show the error

  # --- Plan Review Page (Conversation UI) ---

  Scenario: Viewing the plan
    Given a task with status "planned" and a GoalPlan v1
    When I visit the Plan Review page
    Then I should see the goal description at the top
    And a plan message from the planner showing:
      | Field | Content |
      | Version | "Plan v1" |
      | Model | the model that generated it |
      | Duration | how long planning took |
      | Sub-tasks | table with id, description, files, complexity |
    And a feedback input area at the bottom
    And two buttons: "Refine Plan" and "Approve & Run"

  Scenario: Plan appears in kanban
    Given a task with status "planned"
    When I visit the project page
    Then the kanban should have an "Awaiting Review" column (blue)
    And the task should appear in that column
    And there should be a "Review Plan" button on the card
    When I click "Review Plan"
    Then I should navigate to the Plan Review page

  # --- Feedback Loop ---

  Scenario: Send feedback to refine the plan
    Given I am on the Plan Review page with Plan v1
    When I type "Remove task 3, focus only on error types" in the feedback box
    And I click "Refine Plan"
    Then my feedback should appear as a right-aligned chat bubble
    And the status should change to "planning"
    And a spinner should show "Planner is working..."
    And the daemon should re-plan with the full conversation:
      | Message | Role |
      | Plan v1 sub-tasks | planner |
      | "Remove task 3, focus only on error types" | user |
    And a new GoalPlan v2 should be stored
    And the status should change to "planned"
    And Plan v2 should appear as a new planner message

  Scenario: Multiple feedback iterations
    Given I have refined the plan twice (v1, v2, v3)
    Then the Plan Review page should show all three versions
    And each version should show the feedback that triggered it
    And the conversation should read top-to-bottom chronologically

  Scenario: Feedback while plan is generating
    Given the status is "planning" (planner is working)
    Then the feedback input should be disabled
    And a spinner should show "Planner is working..."
    When the plan is ready (status changes to "planned")
    Then the input should become enabled
    And the new plan should appear

  # --- Approval ---

  Scenario: Approve a plan
    Given I am on the Plan Review page with a plan
    When I click "Approve & Run"
    Then the status should change to "approved"
    And the page should show "Plan approved — agents executing"
    And a link to the project page should appear
    And the daemon should start executing agents using the approved plan

  Scenario: Approve after multiple refinements
    Given I have refined the plan 3 times
    When I click "Approve & Run"
    Then the latest plan version should be used for execution
    And the execution should use the sub-tasks from the latest version

  # --- Edge Cases ---

  Scenario: Page refresh during planning
    Given I am on the Plan Review page and status is "planning"
    When I refresh the page
    Then the page should reload and show "Planner is working..."
    And when the plan is ready, it should appear via auto-refresh

  Scenario: Navigate away and come back
    Given a task with status "planned"
    When I navigate to the project page
    Then I should see the task in "Awaiting Review" column
    When I click "Review Plan"
    Then I should see the full conversation history

  Scenario: Clear all data while planning
    Given a task with status "planning"
    When I click "Clear All Data" on the Overview page
    Then all tasks and plans should be deleted
    And the Plan Review page should show "No plan yet"
