# Refactor crates/agent-daemon/src/github_sync.rs: replace the STATE_LABELS membership checks with a small helper fn is_state_label(name: &str) -> bool

- model: unknown
- files: crates/agent-daemon/src/github_sync.rs
- when: 2026-06-05T20:42:30.564456+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/github_sync.rs
+++ b/crates/agent-daemon/src/github_sync.rs
@@ -8,6 +8,12 @@
     input.replace("\"", "\\\"")
 }
 
+const STATE_LABELS: [&str; 5] = ["Success", "Failure", "Skipped", "Cancelled", "Running"];
+
+fn is_state_label(name: &str) -> bool {
+    STATE_LABELS.contains(&name)
+}
+
 fn set_label(repo: &str, issue_number: u32, label: &str) {
     // Implementation to add a label
 }
@@ -45,35 +51,14 @@
             assert_eq!(status_to_label(status), expected_label);
         }
     }
-}
-
-#[cfg(test)]
-mod tests {
-    use super::*;
 
-    fn status_to_label(status: &str) -> &str {
-        match status {
-            "passed" => "Success",
-            "failed" => "Failure",
-            "skipped" => "Skipped",
-            "cancelled" => "Cancelled",
-            "running" => "Running",
-            _ => "Unknown",
-        }
-    }
-
     #[test]
-    fn test_status_to_label() {
-        let status_map = HashMap::from([
-            ("passed", "Success"),
-            ("failed", "Failure"),
-            ("skipped", "Skipped"),
-            ("cancelled", "Cancelled"),
-            ("running", "Running"),
-        ]);
-
-        for (status, expected_label) in status_map {
-            assert_eq!(status_to_label(status), expected_label);
-        }
+    fn test_is_state_label() {
+        assert!(is_state_label("Success"));
+        assert!(is_state_label("Failure"));
+        assert!(is_state_label("Skipped"));
+        assert!(is_state_label("Cancelled"));
+        assert!(is_state_label("Running"));
+        assert!(!is_state_label("Unknown"));
     }
 }
\ No newline at end of file

```
