//! WASI-portable planner types and constants.

use serde::{Deserialize, Serialize};
use inference_client::Complexity;

/// A sub-task decomposed from a high-level goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub complexity: Complexity,
}

/// System prompt for the planner LLM.
pub const PLANNER_SYSTEM: &str = r#"You decompose a goal into the MINIMUM number of tasks. Correctness matters more than speed.

RULES:
- Create as FEW tasks as possible. If the goal can be done in 1 task, output 1 task.
- For EDITING existing files: list files from the repository file list below.
- For CREATING new files: you MAY list file paths that don't exist yet. The agent will use <<<CREATE>>> to make them.
- Each task lists the specific files it will read, modify, or create.
- Classify complexity: simple (1-2 files, small change), medium (2-4 files), complex (4+ files).
- Maximum 5 tasks. Most goals need 1-3.

OUTPUT FORMAT (JSON array only, no other text):
[{"id":"task-1","description":"what to do","files":["existing/file.rs","new/file/to/create.ts"],"complexity":"simple"}]"#;

/// Maximum number of sub-tasks the planner can create.
pub const MAX_TASKS: usize = 5;

/// Parse a planner response into sub-tasks, validating against known repo files.
/// Tolerant: tries JSON directly, then extracts from markdown fences, then falls back to single task.
pub fn parse_plan(json_str: &str, repo_files: &[String]) -> Result<Vec<SubTask>, String> {
    let input = json_str.trim();

    // Try 1: direct JSON parse
    let mut tasks: Vec<SubTask> = match serde_json::from_str(input) {
        Ok(t) => t,
        Err(_) => {
            // Try 2: extract from markdown code fence ```json ... ```
            let extracted = if let Some(start) = input.find("```json") {
                let after = &input[start + 7..];
                if let Some(end) = after.find("```") {
                    after[..end].trim()
                } else { "" }
            } else if let Some(start) = input.find("```") {
                let after = &input[start + 3..];
                if let Some(end) = after.find("```") {
                    after[..end].trim()
                } else { "" }
            } else { "" };

            if !extracted.is_empty() {
                serde_json::from_str(extracted).unwrap_or_default()
            } else {
                Vec::new()
            }
        }
    };

    // Try 3: if still empty, find JSON array in the text
    if tasks.is_empty() {
        if let Some(start) = input.find('[') {
            if let Some(end) = input.rfind(']') {
                let _ = serde_json::from_str::<Vec<SubTask>>(&input[start..=end])
                    .map(|t| tasks = t);
            }
        }
    }

    // Fallback: create single task from the goal (model didn't produce JSON)
    if tasks.is_empty() {
        // Extract file paths mentioned in the goal text (look for path-like strings)
        let mut files: Vec<String> = extract_file_paths_from_text(json_str);

        // Also include existing repo files that are referenced in the goal
        for repo_file in repo_files {
            if json_str.contains(repo_file.as_str()) && !files.contains(repo_file) {
                files.push(repo_file.clone());
            }
        }

        // If still empty, pick first 5 repo files as fallback
        if files.is_empty() {
            files = repo_files.iter()
                .filter(|f| !f.starts_with("target/") && !f.starts_with(".git/"))
                .take(5)
                .cloned()
                .collect();
        }

        return Ok(vec![SubTask {
            id: "task-1".into(),
            description: json_str.chars().take(500).collect(),
            files,
            complexity: inference_client::Complexity::Medium,
        }]);
    }

    // Cap tasks
    if tasks.len() > MAX_TASKS {
        tasks.truncate(MAX_TASKS);
    }

    // Validate files: existing files must match repo, new files are allowed (agent will CREATE them)
    for task in &mut tasks {
        let (existing, new): (Vec<_>, Vec<_>) = task.files.iter()
            .partition(|f| repo_files.iter().any(|rf| rf == *f));

        // Keep all files — existing ones for EDIT, new ones for CREATE
        // But if a task has ONLY non-existent files and none look like valid paths, flag it
        if existing.is_empty() && !new.is_empty() {
            // Ensure new file paths look reasonable (have an extension, no spaces)
            task.files.retain(|f| f.contains('.') && !f.contains(' '));
        }
    }

    // Remove tasks with no files left
    tasks.retain(|t| !t.files.is_empty());

    if tasks.is_empty() {
        return Err("All planned tasks have no valid files".into());
    }

    Ok(tasks)
}

/// Extract file paths from free text (e.g. goal descriptions).
/// Looks for strings that look like file paths: contain '/' and end with a known extension.
fn extract_file_paths_from_text(text: &str) -> Vec<String> {
    let extensions = [".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".toml", ".json", ".yaml", ".yml", ".md", ".css", ".html"];
    let mut paths = Vec::new();

    for word in text.split_whitespace() {
        // Strip punctuation from edges
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-');
        if clean.contains('/') && extensions.iter().any(|ext| clean.ends_with(ext)) {
            if !paths.contains(&clean.to_string()) {
                paths.push(clean.to_string());
            }
        }
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plan_allows_new_files() {
        let repo_files = vec!["src/lib.rs".into(), "Cargo.toml".into()];
        let json = r#"[{"id":"task-1","description":"Create types","files":["src/lib.rs","dashboard/src/types/swarm.ts"],"complexity":"simple"}]"#;
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].files.len(), 2);
        assert!(tasks[0].files.contains(&"dashboard/src/types/swarm.ts".to_string()));
    }

    #[test]
    fn parse_plan_rejects_invalid_new_paths() {
        let repo_files = vec!["src/lib.rs".into()];
        let json = r#"[{"id":"task-1","description":"Bad","files":["no extension"],"complexity":"simple"}]"#;
        let result = parse_plan(json, &repo_files);
        assert!(result.is_err());
    }

    #[test]
    fn parse_plan_keeps_existing_files() {
        let repo_files = vec!["src/lib.rs".into(), "src/main.rs".into()];
        let json = r#"[{"id":"task-1","description":"Edit","files":["src/lib.rs"],"complexity":"simple"}]"#;
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].files, vec!["src/lib.rs"]);
    }

    #[test]
    fn extract_paths_from_goal_text() {
        let text = "Read crates/knowledge-base/src/schema.rs then CREATE dashboard/src/types/swarm.ts";
        let paths = extract_file_paths_from_text(text);
        assert!(paths.contains(&"crates/knowledge-base/src/schema.rs".to_string()));
        assert!(paths.contains(&"dashboard/src/types/swarm.ts".to_string()));
    }

    #[test]
    fn fallback_uses_goal_paths() {
        let repo_files = vec!["crates/knowledge-base/src/schema.rs".into(), "src/lib.rs".into()];
        // This text won't parse as JSON, triggering fallback
        let text = "Read crates/knowledge-base/src/schema.rs and create dashboard/src/types/swarm.ts";
        let tasks = parse_plan(text, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].files.contains(&"crates/knowledge-base/src/schema.rs".to_string()));
        assert!(tasks[0].files.contains(&"dashboard/src/types/swarm.ts".to_string()));
    }

    #[test]
    fn parse_plan_only_new_files() {
        let repo_files = vec!["src/lib.rs".into()];
        let json = r#"[{"id":"task-1","description":"Create","files":["new/file.ts"],"complexity":"simple"}]"#;
        let tasks = parse_plan(json, &repo_files).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].files, vec!["new/file.ts"]);
    }
}
