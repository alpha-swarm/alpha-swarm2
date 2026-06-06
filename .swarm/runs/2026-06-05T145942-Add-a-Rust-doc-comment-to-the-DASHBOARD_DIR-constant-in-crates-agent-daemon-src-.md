# Add a Rust doc comment to the DASHBOARD_DIR constant in crates/agent-daemon/src/http_bridge.rs

- model: unknown
- files: crates/agent-daemon/src/http_bridge.rs
- when: 2026-06-05T14:59:42.678367+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/http_bridge.rs
+++ b/crates/agent-daemon/src/http_bridge.rs
@@ -34,6 +34,10 @@
 /// Leading verbs allowed on /sql statements (same policy as the NATS bridge).
 const ALLOWED_VERBS: &[&str] = &["SELECT", "CREATE", "UPDATE", "UPSERT", "DELETE", "RELATE", "INSERT"];
 /// Built Leptos dashboard bundle (trunk dist), relative to the daemon workdir.
+///
+/// This constant holds the directory path where the Leptos dashboard bundle is located.
+/// The dashboard is served by the HTTP shim for WASM components, allowing access to the
+/// built web application.
 const DASHBOARD_DIR: &str = "dashboard-leptos/dist";
 
 #[derive(Clone)]

```
