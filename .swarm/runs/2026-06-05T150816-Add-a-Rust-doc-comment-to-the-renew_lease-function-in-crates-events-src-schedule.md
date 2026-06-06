# Add a Rust doc comment to the renew_lease function in crates/events/src/scheduler.rs

- model: unknown
- files: crates/events/src/scheduler.rs
- when: 2026-06-05T15:08:16.325796+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/events/src/scheduler.rs
+++ b/crates/events/src/scheduler.rs
@@ -132,7 +132,15 @@
     }
 
     /// Renew a lease (heartbeat — prevents expiry during long tasks).
-    pub async fn renew_lease(&self, run_id: &str) -> Result<()> {
+    /// Renews the lease for a given run ID.
+///
+/// This function updates the lease entry in the KV store to extend its expiration time,
+/// ensuring that the daemon continues to have exclusive access to the task.
+/// Renews the lease for a given run ID.
+///
+/// This function updates the lease entry in the KV store to extend its expiration time,
+/// ensuring that the daemon continues to have exclusive access to the task.
+pub async fn renew_lease(&self, run_id: &str) -> Result<()> {
         let key = format!("lease.{}", sanitize_key(run_id));
         let lease = LeaseEntry {
             daemon_id: self.daemon_id.clone(),

```
