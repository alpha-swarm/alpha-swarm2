use std::collections::HashMap;

/// Escapes double quotes in the input string.
use std::collections::HashMap;

/// Escapes double quotes in the input string.
fn esc(input: &str) -> String {
    input.replace("\"", "\\\"")
}

const STATE_LABELS: [&str; 5] = ["Success", "Failure", "Skipped", "Cancelled", "Running"];

fn is_state_label(name: &str) -> bool {
    STATE_LABELS.contains(&name)
}

fn comment(repo: &str, number: i64, body: &str) {
    // Implementation to post a comment
}

fn set_label(repo: &str, issue_number: u32, label: &str) {
    comment(repo, issue_number as i64, &format!("Adding label: {}", label));
}

fn remove_label(repo: &str, issue_number: u32, label: &str) {
    comment(repo, issue_number as i64, &format!("Removing label: {}", label));
}

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