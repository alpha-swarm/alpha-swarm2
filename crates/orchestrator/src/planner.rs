use anyhow::{Context, Result};
use tracing::{info, warn};

use inference_client::{ChatMessage, Complexity, InferenceOptions, InferenceRouter};
use crate::planner_types::{SubTask, PLANNER_SYSTEM, parse_plan};

/// Decompose a high-level goal into sub-tasks via LLM inference.
pub async fn plan_goal(
    router: &InferenceRouter,
    goal: &str,
    repo_files: &[String],
    tier: &swarm_config::TierConfig,
) -> Result<Vec<SubTask>> {
    info!(goal, file_count = repo_files.len(), model = %tier.model, "Planning goal decomposition");

    // Limit file list to avoid exceeding context window
    // Prioritize files mentioned in the goal, then source files
    let max_files = tier.max_context_files.min(100);
    let limited_files: Vec<&str> = repo_files.iter()
        .filter(|f| !f.contains("/target/") && !f.starts_with("target/"))
        .take(max_files)
        .map(|s| s.as_str())
        .collect();
    let file_list = limited_files.join("\n");
    info!(total = repo_files.len(), sent = limited_files.len(), "Planner file list");
    let user_msg = format!("GOAL: {goal}\n\nREPOSITORY FILES:\n{file_list}");

    let messages = vec![
        ChatMessage::system(PLANNER_SYSTEM),
        ChatMessage::user(user_msg),
    ];

    let options = InferenceOptions {
        max_tokens: Some(tier.context_window),
        preferred_model: Some(tier.model.clone()),
        preferred_backend: Some(inference_client::BackendKind::Ollama),
        ..Default::default()
    };

    let response = router.chat(&messages, Complexity::Complex, &options).await
        .context("Planning inference failed")?;

    info!(model = %response.model, tokens = response.tokens_output, "Plan generated");

    // Extract JSON array from response
    let content = response.content.trim();
    let json_str = if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') {
            &content[start..=end]
        } else {
            content
        }
    } else {
        content
    };

    let tasks = parse_plan(json_str, repo_files)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Log file overlaps
    let mut seen_files = std::collections::HashSet::new();
    for task in &tasks {
        for file in &task.files {
            if !seen_files.insert(file.clone()) {
                warn!(file, task = %task.id, "File overlap detected");
            }
        }
    }

    info!(task_count = tasks.len(), "Goal decomposed into sub-tasks");
    for task in &tasks {
        info!(id = %task.id, description = %task.description, files = ?task.files, complexity = ?task.complexity, "Sub-task");
    }

    Ok(tasks)
}
