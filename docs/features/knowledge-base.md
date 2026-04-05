# Feature: Knowledge Base (SurrealDB)

## Scenario: Store and retrieve a run
  Given a connected KnowledgeStore
  When I store an AgentRun with project="test" and status=Passed
  Then store_run returns a non-empty ID
  And list_runs("test", None) includes the stored run

## Scenario: Filter runs by status
  Given runs with statuses: Passed, Failed, Running
  When I list_runs with status=Some(Failed)
  Then only the Failed run is returned

## Scenario: Update run status
  Given a stored run with status=Running
  When I update_run to status=Passed
  Then list_runs returns it with status=Passed

## Scenario: Vector similarity search
  Given a stored run with embedding [1.0, 0.0, 0.0, ...]
  When I find_similar with embedding [0.99, 0.01, 0.0, ...]
  Then the stored run is returned with similarity > 0.9

## Scenario: task_already_done returns Passed run
  Given a Passed run with high embedding similarity (>0.9)
  When I call task_already_done with threshold 0.9
  Then it returns Some(the matching run)

## Scenario: task_already_done ignores Failed runs
  Given only Failed runs with high similarity
  When I call task_already_done
  Then it returns None

## Scenario: find_past_errors returns only Failed runs
  Given Passed and Failed runs
  When I call find_past_errors
  Then only Failed runs are returned

## Scenario: Metrics aggregation
  Given 10 runs: 7 Passed, 2 Failed, 1 Skipped using 2 models
  When I compute ProjectMetrics::from_runs
  Then total_runs=10, passed=7, failed=2, skipped=1
  And pass_rate=0.7
  And models_used has 2 entries sorted by run count

## Scenario: Empty metrics
  Given 0 runs
  When I compute ProjectMetrics::from_runs
  Then pass_rate=0.0 and avg_duration_ms=0
