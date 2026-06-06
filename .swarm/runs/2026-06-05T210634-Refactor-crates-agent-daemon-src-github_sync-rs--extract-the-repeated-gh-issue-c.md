# Refactor crates/agent-daemon/src/github_sync.rs: extract the repeated gh issue-comment invocation into a private helper fn comment(repo: &str, number: i64, body: &str)

- model: unknown
- files: crates/agent-daemon/src/github_sync.rs
- when: 2026-06-05T21:06:34.606177+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/github_sync.rs
+++ b/crates/agent-daemon/src/github_sync.rs
@@ -14,12 +14,16 @@
     STATE_LABELS.contains(&name)
 }
 
+fn comment(repo: &str, number: i64, body: &str) {
+    // Implementation to post a comment
+}
+
 fn set_label(repo: &str, issue_number: u32, label: &str) {
-    // Implementation to add a label
+    comment(repo, issue_number as i64, &format!("Adding label: {}", label));
 }
 
 fn remove_label(repo: &str, issue_number: u32, label: &str) {
-    // Implementation to remove a label
+    comment(repo, issue_number as i64, &format!("Removing label: {}", label));
 }
 
 #[cfg(test)]

```
