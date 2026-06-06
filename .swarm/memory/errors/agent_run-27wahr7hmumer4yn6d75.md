---
key: agent_run:27wahr7hmumer4yn6d75
project: alpha-swarm2
namespace: errors
use_count: 3
---

GOAL: Refactor crates/agent-daemon/src/github_sync.rs: extract the repeated gh issue-comment invocation into a private helper fn comment(repo: &str, number: i64, body: &str) and call it from ingest and reconcile
FAILED PLAN:
[passed] task-1: Extract repeated GitHub issue-comment invocation into a private helper function `comment` and call it from `ingest` and `reconcile` in `github_sync.rs`.
