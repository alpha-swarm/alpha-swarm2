# Feature: Agent Specialization via System Prompts

## Scenario: from_str parses all variants
  Then "lint" -> LintFixer
  And "lint-fixer" -> LintFixer
  And "linter" -> LintFixer
  And "test" -> TestWriter
  And "test-writer" -> TestWriter
  And "refactor" -> Refactorer
  And "feature" -> FeatureAdder
  And "bug" -> BugFixer
  And "bugfix" -> BugFixer
  And "general" -> General
  And "unknown" -> General
  And "" -> General

## Scenario: Lint-fixer prompt is constrained
  When I build a prompt with AgentType::LintFixer
  Then the system prompt contains "fix ONLY the reported lint issues"
  And the system prompt contains "Do not refactor or change logic"

## Scenario: Test-writer preserves existing code
  When I build a prompt with AgentType::TestWriter
  Then the system prompt contains "Do not modify the code under test"
  And mentions "edge cases"

## Scenario: Refactorer preserves behavior
  When I build a prompt with AgentType::Refactorer
  Then the system prompt contains "Do NOT change external behavior"

## Scenario: All prompts include edit format
  For each AgentType variant:
  When I build a prompt
  Then the system prompt contains "<<<EDIT" and "--- OLD" and "--- NEW"

## Scenario: User message includes all files
  Given files: [("src/a.rs", "fn a(){}"), ("src/b.rs", "fn b(){}")]
  When I build a prompt for task "fix both"
  Then the user message contains "=== src/a.rs ===" and "=== src/b.rs ==="
  And contains both function bodies
