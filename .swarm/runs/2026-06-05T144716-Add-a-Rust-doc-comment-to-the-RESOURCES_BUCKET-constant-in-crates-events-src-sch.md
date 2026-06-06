# Add a Rust doc comment to the RESOURCES_BUCKET constant in crates/events/src/scheduler.rs

- model: unknown
- files: crates/events/src/scheduler.rs
- when: 2026-06-05T14:47:16.437696+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/events/src/scheduler.rs
+++ b/crates/events/src/scheduler.rs
@@ -13,6 +13,7 @@
 /// KV bucket names
 const TASKS_BUCKET: &str = "swarm-tasks";
 const LEASES_BUCKET: &str = "swarm-leases";
+/// The bucket name for storing resource snapshots.
 const RESOURCES_BUCKET: &str = "swarm-resources";
 /// Key for the global execution lock (only 1 goal runs across all daemons).
 const EXECUTION_LOCK_KEY: &str = "goal-execution-lock";

```
