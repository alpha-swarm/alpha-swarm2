# Add a #[cfg(test)] unit test for status_to_label in crates/agent-daemon/src/github_sync.rs covering all statuses

- model: qwen2.5-coder:14b
- files: crates/agent-daemon/src/github_sync.rs
- when: 2026-06-06T01:31:15.351290+00:00

## Agent output

```
<<<EDIT crates/agent-daemon/src/github_sync.rs
--- OLD
#[cfg(test)]
mod tests {
    use super::*;

    fn status_to_label(status: &str) -> &str {
        match status {
            "passed" => "Success",
            "failed" => "Failure",
            "skipped" => "Skipped",
            "cancelled" => "Cancelled",
            "running" => "Running",
            _ => "Unknown",
        }
    }

    #[test]
    fn test_status_to_label() {
        let status_map = HashMap::from([
            ("passed", "Success"),
            ("failed", "Failure"),
            ("skipped", "Skipped"),
            ("cancelled", "Cancelled"),
            ("running", "Running"),
        ]);

        for (status, expected_label) in status_map {
            assert_eq!(status_to_label(status), expected_label);
        }
    }

    #[test]
    fn test_is_state_label() {
        assert!(is_state_label("Success"));
        assert!(is_state_label("Failure"));
        assert!(is_state_label("Skipped"));
        assert!(is_state_label("Cancelled"));
        assert!(is_state_label("Running"));
        assert!(!is_state_label("Unknown"));
    }
}
--- NEW
#[cfg(test)]
mod tests {
    use super::*;

    fn status_to_label(status: &str) -> &str {
        match status {
            "passed" => "Success",
            "failed" => "Failure",
            "skipped" => "Skipped",
            "cancelled" => "Cancelled",
            "running" => "Running",
            _ => "Unknown",
        }
    }

    #[test]
    fn test_status_to_label() {
        let status_map = HashMap::from([
            ("passed", "Success"),
            ("failed", "Failure"),
            ("skipped", "Skipped"),
            ("cancelled", "Cancelled"),
            ("running", "Running"),
        ]);

        for (status, expected_label) in status_map {
            assert_eq!(status_to_label(status), expected_label);
        }

        // Add test cases for unknown statuses
        assert_eq!(status_to_label("unknown"), "Unknown");
        assert_eq!(status_to_label(""), "Unknown");
        assert_eq!(status_to_label("random_status"), "Unknown");
    }

    #[test]
    fn test_is_state_label() {
        assert!(is_state_label("Success"));
        assert!(is_state_label("Failure"));
        assert!(is_state_label("Skipped"));
        assert!(is_state_label("Cancelled"));
        assert!(is_state_label("Running"));
        assert!(!is_state_label("Unknown"));
    }
}
>>>
```

## Diff

```diff
--- a/crates/agent-daemon/src/github_sync.rs
+++ b/crates/agent-daemon/src/github_sync.rs
@@ -54,6 +54,11 @@
         for (status, expected_label) in status_map {
             assert_eq!(status_to_label(status), expected_label);
         }
+
+        // Add test cases for unknown statuses
+        assert_eq!(status_to_label("unknown"), "Unknown");
+        assert_eq!(status_to_label(""), "Unknown");
+        assert_eq!(status_to_label("random_status"), "Unknown");
     }
 
     #[test]

```
