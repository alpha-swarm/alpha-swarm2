# Feature: Retry with Model Escalation

## Scenario: First attempt passes quality gate
  Given an agent that produces valid edits
  And the quality gate passes (fmt + lint + build + test)
  When I run_with_retry with max_attempts=3
  Then it returns on attempt=1

## Scenario: Retry same model on first failure
  Given an agent whose edits fail cargo clippy
  When attempt 1 fails quality gate
  Then attempt 2 runs with the same model
  And the prompt includes the clippy error output
  And the prompt says "Fix the issues from the previous attempt"

## Scenario: Escalate to larger model on third attempt
  Given attempts 1 and 2 failed with qwen2.5-coder:7b
  When attempt 3 starts
  Then it calls escalate_model to get a larger model
  And the inference options specify the escalated model
  And AgentResult.escalated_from contains "qwen2.5-coder:7b"

## Scenario: All attempts exhausted
  Given max_attempts=3 and all 3 fail quality gate
  When run_with_retry completes
  Then it returns the last AgentResult with attempt=3
  And applied=true (edits were applied even though quality failed)

## Scenario: Skipped task does not retry
  Given the knowledge base says this task is already done
  When I run_with_retry
  Then it returns skipped=true with attempt=1
  And no quality gate is run
