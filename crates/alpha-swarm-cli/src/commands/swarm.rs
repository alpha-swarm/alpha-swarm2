use std::path::PathBuf;

use anyhow::{Context, Result};
use inference_client::InferenceRouter;
use tracing::info;

use swarm_config::SwarmConfig;
use crate::setup;

pub async fn execute(
    router: &InferenceRouter,
    config: &SwarmConfig,
    repo: PathBuf,
    goal: String,
    project: String,
) -> Result<()> {
    let repo = repo.canonicalize().context("Repository path does not exist")?;
    info!(repo = %repo.display(), goal = %goal, project = %project, "Starting swarm");

    let ollama = setup::get_ollama(config);
    let kb = setup::get_knowledge_store(config).await?;

    let mut runner = swarm_orchestrator::SwarmRunner::new(router, &ollama, &repo, &project);
    if let Some(store) = &kb {
        runner = runner.with_store(store);
    }

    let result = runner.run(&goal).await?;

    println!("\n=== Swarm Result ===");
    println!("Goal:     {}", result.goal);
    println!("Tasks:    {}", result.tasks.len());
    println!("Duration: {}ms", result.total_duration_ms);
    println!("Quality:  {}", if result.quality_passed { "PASSED" } else { "FAILED" });

    println!("\n--- Sub-tasks ---");
    for tr in &result.results {
        let status = if let Some(ref r) = tr.agent_result {
            if r.skipped { "SKIP" } else if r.applied { "DONE" } else { "NOOP" }
        } else { "ERR" };
        let edits = tr.agent_result.as_ref().map(|r| r.edits.len()).unwrap_or(0);
        println!("  [{status}] {} — {} (edits: {}, files: {:?})", tr.task.id, tr.task.description, edits, tr.task.files);
        if let Some(err) = &tr.error { println!("         error: {err}"); }
    }

    if let Some(diff) = &result.merged_diff {
        if !diff.is_empty() {
            println!("\n--- Merged Diff ---");
            for line in diff.lines().take(50) { println!("{line}"); }
            if diff.lines().count() > 50 {
                println!("... ({} more lines)", diff.lines().count() - 50);
            }
        }
    }

    if !result.quality_passed { std::process::exit(1); }
    Ok(())
}
