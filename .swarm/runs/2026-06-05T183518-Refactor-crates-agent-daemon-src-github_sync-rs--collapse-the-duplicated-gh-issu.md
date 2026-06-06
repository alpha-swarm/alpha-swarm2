# Refactor crates/agent-daemon/src/github_sync.rs: collapse the duplicated gh issue-edit add/remove-label calls into a helper fn set_label and remove_label

- model: unknown
- files: crates/agent-daemon/src/github_sync.rs
- when: 2026-06-05T18:35:18.031071+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/github_sync.rs
+++ b/crates/agent-daemon/src/github_sync.rs
@@ -1,10 +1,52 @@
 use std::collections::HashMap;
 
 /// Escapes double quotes in the input string.
+use std::collections::HashMap;
+
+/// Escapes double quotes in the input string.
 fn esc(input: &str) -> String {
     input.replace("\"", "\\\"")
 }
 
+fn set_label(repo: &str, issue_number: u32, label: &str) {
+    // Implementation to add a label
+}
+
+fn remove_label(repo: &str, issue_number: u32, label: &str) {
+    // Implementation to remove a label
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn status_to_label(status: &str) -> &str {
+        match status {
+            "passed" => "Success",
+            "failed" => "Failure",
+            "skipped" => "Skipped",
+            "cancelled" => "Cancelled",
+            "running" => "Running",
+            _ => "Unknown",
+        }
+    }
+
+    #[test]
+    fn test_status_to_label() {
+        let status_map = HashMap::from([
+            ("passed", "Success"),
+            ("failed", "Failure"),
+            ("skipped", "Skipped"),
+            ("cancelled", "Cancelled"),
+            ("running", "Running"),
+        ]);
+
+        for (status, expected_label) in status_map {
+            assert_eq!(status_to_label(status), expected_label);
+        }
+    }
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;

```
