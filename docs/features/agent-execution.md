# Feature: One-Shot Agent Execution

## Scenario: Agent reads files, calls LLM, applies edits
  Given a repo with src/main.rs containing "fn main() {}"
  And an inference backend that returns an EDIT block
  When I run the agent with task "add a greet function"
  Then it reads src/main.rs
  And calls the inference backend with file contents in the prompt
  And parses the response into edits
  And writes the modified file
  And returns AgentResult with applied=true and edits.len() > 0

## Scenario: Agent with no edits in response
  Given an inference backend that returns text without edit blocks
  When I run the agent
  Then it returns AgentResult with applied=false and edits=[]

## Scenario: Agent skips task already done in knowledge base
  Given a knowledge base with a Passed run for "add greet function" (similarity > 0.9)
  When I run the same task with knowledge enabled
  Then the agent returns skipped=true
  And does NOT call the inference backend

## Scenario: Agent includes past errors in prompt
  Given a knowledge base with a Failed run for "add logging"
  When I run a similar task "add log statements"
  Then the user message includes "PAST ERRORS TO AVOID"
  And includes the previous error message

## Scenario: Agent detects parallel agents
  Given a Running agent on project "myapp"
  When I start a second agent on "myapp" with knowledge enabled
  Then the prompt includes "CURRENTLY RUNNING AGENTS"
  And includes the first agent's task description

## Scenario: Agent records run in knowledge base
  Given knowledge base is connected
  When the agent completes (pass or fail)
  Then a run record is stored with model, tokens, duration, status
  And an embedding is stored for future similarity search

## Scenario: File read failure
  Given a task referencing a nonexistent file "src/missing.rs"
  When I run the agent
  Then it returns an error mentioning the file path
