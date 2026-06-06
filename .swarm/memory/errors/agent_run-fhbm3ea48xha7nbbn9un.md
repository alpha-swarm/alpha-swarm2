---
key: agent_run:fhbm3ea48xha7nbbn9un
project: alpha-swarm2
namespace: errors
use_count: 0
---

GOAL: Refactor crates/events/src/scheduler.rs: extract the repeated lease key expression format!("lease.{}", sanitize_key(run_id)) into a private helper fn lease_key(run_id: &str) -> String and use it in try_claim, release_lease and renew_lease
FAILED PLAN:
[passed] task-1: Extract repeated lease key expression into a private helper function.
