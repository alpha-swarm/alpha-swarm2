use anyhow::{Context, Result};
use tracing::{info, warn};

use inference_client::{ChatMessage, Complexity, InferenceOptions, InferenceRouter};
use crate::planner_types::{SubTask, PLANNER_SYSTEM, REPLANNER_SYSTEM, parse_plan};

/// Max chars of step error text included in a replan prompt.
const MAX_REPLAN_ERROR_CHARS: usize = 600;
/// Max repo files listed in a replan prompt.
const MAX_REPLAN_FILES: usize = 100;

/// Decompose a high-level goal into sub-tasks via LLM inference.
/// If `relevant_files` is provided (from RAG), those are prioritized in the file list.
/// If `past_plans` is provided (SONA retrieval), it is injected as guidance.
pub async fn plan_goal(
    router: &InferenceRouter,
    goal: &str,
    repo_files: &[String],
    tier: &swarm_config::TierConfig,
    relevant_files: Option<&[(String, f32)]>, // (path, similarity score) from RAG
    past_plans: Option<&str>, // pre-rendered "past proven plans" block from memory
) -> Result<Vec<SubTask>> {
    info!(goal, file_count = repo_files.len(), model = %tier.model, "Planning goal decomposition");

    let max_files = tier.max_context_files.min(100);

    // Build prioritized file list: RAG-relevant files first, then remaining
    let mut file_list_parts: Vec<String> = Vec::new();

    if let Some(relevant) = relevant_files {
        let relevant_count = relevant.len().min(20);
        if relevant_count > 0 {
            file_list_parts.push("MOST RELEVANT FILES (by semantic similarity to goal):".into());
            for (path, score) in relevant.iter().take(relevant_count) {
                file_list_parts.push(format!("  {path} (relevance: {score:.2})"));
            }
            file_list_parts.push(String::new());
            info!(relevant = relevant_count, "RAG: found relevant files");
        }
    }

    // Add remaining files (up to max)
    let already_listed: std::collections::HashSet<&str> = relevant_files
        .map(|r| r.iter().map(|(p, _)| p.as_str()).collect())
        .unwrap_or_default();

    let remaining: Vec<&str> = repo_files.iter()
        .filter(|f| !f.contains("/target/") && !f.starts_with("target/") && !already_listed.contains(f.as_str()))
        .take(max_files.saturating_sub(already_listed.len()))
        .map(|s| s.as_str())
        .collect();

    if !remaining.is_empty() {
        file_list_parts.push("ALL REPOSITORY FILES:".into());
        file_list_parts.push(remaining.join("\n"));
    }

    let file_list = file_list_parts.join("\n");
    let total_files = already_listed.len() + remaining.len();
    info!(total = repo_files.len(), sent = total_files, "Planner file list");

    let past_block = past_plans
        .filter(|p| !p.is_empty())
        .map(|p| format!("PAST PROVEN PLANS (guidance from similar successful goals — adapt to current files):\n{p}\n\n"))
        .unwrap_or_default();

    let user_msg = format!("GOAL: {goal}\n\n{past_block}{file_list}");

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

    let content = response.content.trim();
    let json_str = if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') { &content[start..=end] } else { content }
    } else { content };

    let tasks = parse_plan(json_str, repo_files)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

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

/// Adaptive replanning: produce the REMAINING tasks to finish `goal` from the
/// current state after `failed_step` failed. Output is validated through the
/// same `parse_plan()` as initial plans; validation failure aborts the replan.
#[allow(clippy::too_many_arguments)]
pub async fn replan_goal(
    router: &InferenceRouter,
    goal: &str,
    completed_summary: &str,
    failed_step_desc: &str,
    failed_error: &str,
    files_modified: &[String],
    repo_files: &[String],
    tier: &swarm_config::TierConfig,
) -> Result<Vec<SubTask>> {
    info!(goal, model = %tier.model, "Replanning after step failure");

    let error_trunc: String = failed_error.chars().take(MAX_REPLAN_ERROR_CHARS).collect();
    let file_list: Vec<&str> = repo_files.iter()
        .filter(|f| !f.contains("/target/") && !f.starts_with("target/"))
        .take(MAX_REPLAN_FILES)
        .map(|s| s.as_str())
        .collect();

    let user_msg = format!(
        "GOAL: {goal}\n\n\
         STEPS ALREADY DONE (do not redo):\n{completed}\n\n\
         FAILED STEP:\n{failed}\n\
         ERROR:\n{error}\n\n\
         FILES ALREADY CHANGED: {changed:?}\n\n\
         REPOSITORY FILE LIST:\n{files}",
        goal = goal,
        completed = if completed_summary.is_empty() { "(none)" } else { completed_summary },
        failed = failed_step_desc,
        error = error_trunc,
        changed = files_modified,
        files = file_list.join("\n"),
    );

    let messages = vec![
        ChatMessage::system(REPLANNER_SYSTEM),
        ChatMessage::user(user_msg),
    ];

    let options = InferenceOptions {
        max_tokens: Some(tier.context_window),
        preferred_model: Some(tier.model.clone()),
        preferred_backend: Some(inference_client::BackendKind::Ollama),
        ..Default::default()
    };

    let response = router.chat(&messages, Complexity::Complex, &options).await
        .context("Replanning inference failed")?;

    let content = response.content.trim();
    let json_str = if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') { &content[start..=end] } else { content }
    } else { content };

    // Hard rule: replan output goes through the SAME strict validator as the
    // initial plan (cycle detection, file-hallucination dropping, glob rejection).
    let tasks = parse_plan(json_str, repo_files)
        .map_err(|e| anyhow::anyhow!("Replan validation failed: {e}"))?;

    info!(task_count = tasks.len(), "Replan produced remaining tasks");
    Ok(tasks)
}
