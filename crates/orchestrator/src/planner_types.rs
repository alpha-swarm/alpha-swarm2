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
pub fn parse_plan(json_str: &str, repo_files: &[String]) -> Result<Vec<SubTask>, String> {
    let mut tasks: Vec<SubTask> = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse plan JSON: {e}"))?;

    if tasks.is_empty() {
        return Err("Planner returned empty task list".into());
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
