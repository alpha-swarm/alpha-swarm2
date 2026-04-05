# Feature: Quality Gate Checks

## Scenario: Detect Rust toolchain
  Given a directory containing Cargo.toml
  When I call detect_toolchain
  Then build_cmd = "cargo check"
  And fmt_cmd = "cargo fmt -- --check"
  And lint_cmd = "cargo clippy -- -D warnings"
  And unit_test_cmd = "cargo test"

## Scenario: Detect Node.js toolchain
  Given a directory containing package.json
  When I call detect_toolchain
  Then build_cmd = "npm run build"

## Scenario: Detect Go toolchain
  Given a directory containing go.mod
  When I call detect_toolchain
  Then build_cmd = "go build ./..."

## Scenario: Unknown toolchain
  Given an empty directory
  When I call detect_toolchain
  Then all commands are None

## Scenario: Run all checks in order, stop on first failure
  Given a Rust repo where fmt passes but lint fails
  When I call run_all
  Then results has 2 entries: [fmt: passed, lint: failed]
  And build and test are NOT run

## Scenario: All checks pass
  Given a clean Rust repo
  When I call run_all
  Then all 4 checks pass: fmt, lint, build, test

## Scenario: CheckResult captures output
  When a check fails
  Then check.passed = false
  And check.exit_code != 0
  And check.stderr contains the error output
  And check.duration_ms > 0

## Scenario: Run single check by name
  When I call run_single("lint", ...)
  Then only clippy runs
  And the result has check_name = "lint"
