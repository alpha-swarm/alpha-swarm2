Feature: Tool System
  As an AI agent
  I want to use deterministic tools for mechanical operations
  So that I save LLM tokens for creative reasoning

  Scenario: Read a file via tool
    Given the tool registry with "read_file" registered
    When the model calls read_file(path="src/main.rs")
    Then the tool reads the file from the worktree
    And returns the content (max 100KB, truncated if larger)

  Scenario: Search with grep
    Given the tool registry with "grep" registered
    When the model calls grep(pattern="fn main", glob="*.rs")
    Then the tool runs ripgrep (or grep fallback)
    And returns matching lines with file:line format

  Scenario: Tree-sitter AST rename
    Given the tool registry with "ts_rename" registered
    When the model calls ts_rename(path="src/lib.rs", old_name="Foo", new_name="Bar")
    Then tree-sitter parses the Rust AST
    And all identifier/type_identifier nodes matching "Foo" are renamed to "Bar"
    And the file is written back

  Scenario: Run tests
    Given the tool registry with "run_tests" registered
    And a Rust project with Cargo.toml
    When the model calls run_tests()
    Then the tool runs "cargo test -- --nocapture"
    And returns PASSED/FAILED with stdout/stderr (max 10KB)

  Scenario: Web search
    Given the tool registry with "web_search" registered
    When the model calls web_search(query="rust error E0308")
    Then the tool queries DuckDuckGo HTML lite
    And returns top 5 results with title, URL, snippet

  Scenario: NATS remote dispatch with local fallback
    Given a tool registry with NATS dispatcher configured
    When a tool call is made
    Then the registry tries NATS dispatch first (swarm.tools.{name})
    And if NATS fails, falls back to local execution
    And the tool result is returned either way

  Scenario: Allowlisted shell commands
    Given the tool registry with "run_command" registered
    When the model calls run_command(command="cargo", args=["check"])
    Then the tool executes "cargo check"
    When the model calls run_command(command="rm", args=["-rf", "/"])
    Then the tool rejects it ("rm" not in allowlist)
