# Add a Rust /// doc comment to the esc function in crates/agent-daemon/src/github_sync.rs

- model: unknown
- files: crates/agent-daemon/src/github_sync.rs
- when: 2026-06-05T17:30:25.240957+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/github_sync.rs
+++ b/crates/agent-daemon/src/github_sync.rs
@@ -0,0 +1,4 @@
+/// Escapes double quotes in the input string.
+fn esc(input: &str) -> String {
+    input.replace("\"", "\\\"")
+}
\ No newline at end of file

```
