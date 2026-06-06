---
key: agent_run:spb1e2ayoebqw8j88j5r
project: alpha-swarm2
namespace: errors
use_count: 6
---

GOAL: Refactor crates/events/src/scheduler.rs: extract the repeated lease key expression format!("lease.{}", sanitize_key(run_id)) into a private helper fn lease_key(run_id: &str) -> String and use it in try_claim, release_lease and renew_lease
FAILED PLAN:
[passed] task-1: Extract repeated lease key expression into a private helper function and use it in try_claim, release_lease, and renew_lease.
