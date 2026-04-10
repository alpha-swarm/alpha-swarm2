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
- ONLY list files that ACTUALLY EXIST in the repository file list below.
- Never invent or guess file paths. If a file isn't in the list, don't reference it.
- Each task lists the specific files it will read or modify.
- Classify complexity: simple (1-2 files, small change), medium (2-4 files), complex (4+ files).
- Maximum 5 tasks. Most goals need 1-3.

OUTPUT FORMAT (JSON array only, no other text):
[{"id":"task-1","description":"what to do","files":["existing/file.rs"],"complexity":"simple"}]"#;

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
        // Pick the most likely target files from repo_files
        let likely_files: Vec<String> = repo_files.iter()
            .filter(|f| !f.starts_with("target/") && !f.starts_with(".git/"))
            .take(5)
            .cloned()
            .collect();

        return Ok(vec![SubTask {
            id: "task-1".into(),
            description: json_str.chars().take(200).collect(),
            files: likely_files,
            complexity: inference_client::Complexity::Medium,
        }]);
    }

    // Cap tasks
    if tasks.len() > MAX_TASKS {
        tasks.truncate(MAX_TASKS);
    }

    // Filter out tasks referencing non-existent files
    let tasks: Vec<SubTask> = tasks.into_iter().filter(|task| {
        task.files.iter().all(|f| repo_files.iter().any(|rf| rf == f))
    }).collect();

    if tasks.is_empty() {
        return Err("All planned tasks reference non-existent files".into());
    }

    Ok(tasks)
}
