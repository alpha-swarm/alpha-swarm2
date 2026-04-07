Feature: Agent Execution
  As the swarm orchestrator
  I want agents to modify code correctly
  So that tasks are completed and PRs can be created

  Scenario: Agent creates a new file
    Given a task "Create README.md" with file "README.md" assigned
    When the agent runs with the LLM
    Then the LLM should output a <<<CREATE README.md ... >>> block
    And the edit parser should extract the file creation
    And the file should be created in the worktree

  Scenario: Agent edits an existing file
    Given a task "Fix the parse function" with file "src/parser.rs" assigned
    When the agent runs with the LLM
    Then the LLM should output a <<<EDIT src/parser.rs ... >>> block
    And the edit parser should extract the old and new text
    And the file should be modified in the worktree

  Scenario: Agent uses tool calling when supported
    Given a model that supports Ollama native tool calling (qwen2.5 family)
    When the agent runs with tools
    Then the model should receive tool definitions as JSON schema
    And the model can call tools like read_file, grep, run_tests
    And tool results are fed back as role="tool" messages

  Scenario: Agent falls back to standard mode for incompatible models
    Given a model that does not support tool calling (deepseek-coder)
    When the agent tries run_with_tools()
    Then chat_with_tools() should return an error
    And the agent should fall back to standard run()
    And the agent should use <<<EDIT>>>/<<<CREATE>>> format

  Scenario: Non-existent files are handled gracefully
    Given a task with file "new_file.rs" that doesn't exist yet
    When the agent reads files
    Then the file should be reported as "[NEW FILE — use <<<CREATE>>> to create it]"
    And the agent should proceed without error

  Scenario: Quality gate validation
    Given an agent that has applied edits
    When the quality gate runs (cargo fmt, clippy, build, test)
    And all checks pass
    Then the run status should be "passed"
    And a PR should be created

  Scenario: Quality gate failure triggers retry
    Given an agent that applied edits
    When the quality gate fails on "cargo clippy"
    Then the orchestrator should retry with error context
    And the retry prompt should include the clippy errors
    And the model should attempt to fix the issues
