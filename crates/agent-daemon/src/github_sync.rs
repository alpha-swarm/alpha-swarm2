use std::collections::HashMap;

/// Escapes double quotes in the input string.
use std::collections::HashMap;

/// Escapes double quotes in the input string.
fn esc(input: &str) -> String {
    input.replace("\"", "\\\"")
}

fn set_label(repo: &str, issue_number: u32, label: &str) {
    // Implementation to add a label
}

fn remove_label(repo: &str, issue_number: u32, label: &str) {
    // Implementation to remove a label
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
    }
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
    }
}