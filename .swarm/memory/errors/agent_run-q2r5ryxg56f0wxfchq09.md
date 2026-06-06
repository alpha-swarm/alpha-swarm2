---
key: agent_run:q2r5ryxg56f0wxfchq09
project: alpha-swarm2
namespace: errors
use_count: 1
---

GOAL: Refactor crates/events/src/scheduler.rs: extract the repeated lease key expression format!("lease.{}", sanitize_key(run_id)) into a private helper fn lease_key(run_id: &str) -> String and use it in try_claim, release_lease and renew_lease
FAILED PLAN:
[passed] task-1: Extract the repeated lease key expression into a private helper function and use it in try_claim, release_lease, and renew_lease.
