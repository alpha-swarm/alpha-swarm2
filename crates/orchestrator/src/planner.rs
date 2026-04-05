use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use inference_client::{ChatMessage, Complexity, InferenceOptions, InferenceRouter};

/// A sub-task decomposed from a high-level goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub complexity: Complexity,
}

const PLANNER_SYSTEM: &str = r#"You are a task decomposition agent. Given a high-level goal and a list of files in a repository, break the goal into independent sub-tasks that can be worked on in parallel by separate agents.

RULES:
- Each sub-task must list the specific files it will modify
- No two sub-tasks should modify the same file
- Each sub-task should be small enough for a single agent with limited context
- Classify each sub-task as simple, medium, or complex

OUTPUT FORMAT (JSON array):
```json
[
  {
    "id": "task-1",
    "description": "What the agent should do",
    "files": ["src/foo.rs", "src/bar.rs"],
    "complexity": "simple"
  }
]
```

Output ONLY the JSON array, no other text."#;

/// Decompose a high-level goal into parallel sub-tasks.
pub async fn plan_goal(
    router: &InferenceRouter,
    goal: &str,
    repo_files: &[String],
) -> Result<Vec<SubTask>> {
    info!(goal, file_count = repo_files.len(), "Planning goal decomposition");

    let file_list = repo_files.join("\n");
    let user_msg = format!("GOAL: {goal}\n\nREPOSITORY FILES:\n{file_list}");

    let messages = vec![
        ChatMessage::system(PLANNER_SYSTEM),
        ChatMessage::user(user_msg),
    ];

    // Use complex tier for planning — needs good reasoning
    let options = InferenceOptions {
        max_tokens: Some(4096),
        ..Default::default()
    };

    let response = router.chat(&messages, Complexity::Complex, &options).await
        .context("Planning inference failed")?;

    info!(
        model = %response.model,
        tokens = response.tokens_output,
        "Plan generated"
    );

    // Parse JSON from response
    let content = response.content.trim();

    // Try to extract JSON array from the response
    let json_str = if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') {
            &content[start..=end]
        } else {
            content
        }
    } else {
        content
    };

    let tasks: Vec<SubTask> = serde_json::from_str(json_str)
        .with_context(|| format!("Failed to parse plan JSON: {json_str}"))?;

    if tasks.is_empty() {
        bail!("Planner returned empty task list");
    }

    // Validate no file overlaps
    let mut seen_files = std::collections::HashSet::new();
    for task in &tasks {
        for file in &task.files {
            if !seen_files.insert(file.clone()) {
                warn!(file, task = %task.id, "File overlap detected — planner output may cause conflicts");
            }
        }
    }

    info!(task_count = tasks.len(), "Goal decomposed into sub-tasks");
    for task in &tasks {
        info!(
            id = %task.id,
            description = %task.description,
            files = ?task.files,
            complexity = ?task.complexity,
            "Sub-task"
        );
    }

    Ok(tasks)
}
