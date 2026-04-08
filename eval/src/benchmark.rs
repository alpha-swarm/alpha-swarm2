//! Goal benchmark — measures agents/tokens/time/pass-rate across runs.
//! Queries SurrealDB for completed goals and generates reports.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const SURREAL_URL: &str = "http://127.0.0.1:8001";

#[derive(Deserialize, Debug)]
struct AgentRun {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    project: String,
    #[serde(default)]
    task_description: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    model_used: String,
    #[serde(default)]
    tokens_input: u32,
    #[serde(default)]
    tokens_output: u32,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    parent_run_id: Option<String>,
    #[serde(default)]
    files_modified: Vec<String>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    quality_gate_passed: Option<bool>,
    #[serde(default)]
    attempts: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct GoalBenchmark {
    goal: String,
    project: String,
    status: String,
    total_agents: usize,
    agents_passed: usize,
    agents_failed: usize,
    agents_skipped: usize,
    total_tokens_in: u64,
    total_tokens_out: u64,
    total_duration_ms: u64,
    models_used: Vec<String>,
    files_modified: Vec<String>,
    retry_count: usize,
    quality_passed: bool,
    tokens_per_edit: f64,
    errors: Vec<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SurrealResult {
    result: Option<Vec<serde_json::Value>>,
}

pub fn run_benchmark() {
    println!("=== Goal Benchmark ===\n");

    let client = reqwest::blocking::Client::new();

    // Query all runs
    let runs = match query_runs(&client) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to query SurrealDB: {e}");
            return;
        }
    };

    if runs.is_empty() {
        println!("No runs found in SurrealDB.");
        return;
    }

    // Group by parent (orchestrator runs) and their sub-agents
    let mut orchestrator_runs: Vec<&AgentRun> = Vec::new();
    let mut sub_agents: HashMap<String, Vec<&AgentRun>> = HashMap::new();

    for run in &runs {
        if let Some(parent) = &run.parent_run_id {
            sub_agents.entry(parent.clone()).or_default().push(run);
        } else {
            orchestrator_runs.push(run);
        }
    }

    // Build benchmarks
    let mut benchmarks = Vec::new();

    for orch in &orchestrator_runs {
        let id = orch.id.as_deref().unwrap_or("unknown");
        let children = sub_agents.get(id).cloned().unwrap_or_default();

        let agents_passed = children.iter().filter(|r| r.status == "passed").count();
        let agents_failed = children.iter().filter(|r| r.status == "failed").count();
        let agents_skipped = children.iter().filter(|r| r.status == "skipped").count();

        let total_tokens_in: u64 = std::iter::once(orch.tokens_input as u64)
            .chain(children.iter().map(|r| r.tokens_input as u64))
            .sum();
        let total_tokens_out: u64 = std::iter::once(orch.tokens_output as u64)
            .chain(children.iter().map(|r| r.tokens_output as u64))
            .sum();

        let total_duration_ms = orch.duration_ms;

        let mut models: Vec<String> = std::iter::once(orch.model_used.clone())
            .chain(children.iter().map(|r| r.model_used.clone()))
            .filter(|m| !m.is_empty() && m != "auto" && m != "pending")
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        models.sort();

        let mut files: Vec<String> = orch.files_modified.clone();
        for child in &children {
            for f in &child.files_modified {
                if !files.contains(f) { files.push(f.clone()); }
            }
        }

        let errors: Vec<String> = std::iter::once(orch.error_message.clone())
            .chain(children.iter().map(|r| r.error_message.clone()))
            .flatten()
            .filter(|e| !e.is_empty())
            .collect();

        let edits_count = agents_passed.max(1) as f64;
        let tokens_per_edit = if agents_passed > 0 { total_tokens_out as f64 / edits_count } else { 0.0 };

        let retry_count = orch.attempts.len().max(1) - 1;

        benchmarks.push(GoalBenchmark {
            goal: orch.task_description.clone(),
            project: orch.project.clone(),
            status: orch.status.clone(),
            total_agents: children.len(),
            agents_passed,
            agents_failed,
            agents_skipped,
            total_tokens_in,
            total_tokens_out,
            total_duration_ms,
            models_used: models,
            files_modified: files,
            retry_count,
            quality_passed: orch.quality_gate_passed.unwrap_or(false),
            tokens_per_edit,
            errors,
        });
    }

    // Print report
    println!("| Goal | Status | Agents (P/F/S) | Tokens (in/out) | Duration | Models | Retries | Files |");
    println!("|------|--------|----------------|-----------------|----------|--------|---------|-------|");

    for b in &benchmarks {
        let goal_short: String = b.goal.chars().take(40).collect();
        let dur = format_duration(b.total_duration_ms);
        println!(
            "| {} | {} | {}/{}/{} ({}) | {}/{} | {} | {} | {} | {} |",
            goal_short, b.status,
            b.agents_passed, b.agents_failed, b.agents_skipped, b.total_agents,
            b.total_tokens_in, b.total_tokens_out,
            dur,
            b.models_used.join(", "),
            b.retry_count,
            b.files_modified.len(),
        );
    }

    // Summary stats
    let total_goals = benchmarks.len();
    let passed_goals = benchmarks.iter().filter(|b| b.status == "passed").count();
    let failed_goals = benchmarks.iter().filter(|b| b.status == "failed").count();
    let total_tokens: u64 = benchmarks.iter().map(|b| b.total_tokens_in + b.total_tokens_out).sum();
    let total_time: u64 = benchmarks.iter().map(|b| b.total_duration_ms).sum();
    let total_agents: usize = benchmarks.iter().map(|b| b.total_agents).sum();
    let avg_tokens_per_goal = if total_goals > 0 { total_tokens / total_goals as u64 } else { 0 };
    let avg_time_per_goal = if total_goals > 0 { total_time / total_goals as u64 } else { 0 };

    println!("\n## Summary");
    println!("- Goals: {total_goals} (passed: {passed_goals}, failed: {failed_goals})");
    println!("- Pass rate: {:.0}%", if total_goals > 0 { passed_goals as f64 / total_goals as f64 * 100.0 } else { 0.0 });
    println!("- Total agents spawned: {total_agents}");
    println!("- Total tokens: {total_tokens} (avg {avg_tokens_per_goal}/goal)");
    println!("- Total time: {} (avg {}/goal)", format_duration(total_time), format_duration(avg_time_per_goal));

    // Write to file
    let report = generate_markdown_report(&benchmarks);
    std::fs::write("eval/results/benchmark.md", &report).ok();
    println!("\nReport written to eval/results/benchmark.md");
}

fn query_runs(client: &reqwest::blocking::Client) -> Result<Vec<AgentRun>, String> {
    let resp = client.post(format!("{SURREAL_URL}/sql"))
        .basic_auth("root", Some("root"))
        .header("Accept", "application/json")
        .body("USE NS alpha_swarm DB swarm; SELECT * FROM agent_run ORDER BY created_at DESC;")
        .send()
        .map_err(|e| format!("request: {e}"))?;

    let text = resp.text().map_err(|e| format!("text: {e}"))?;
    let body: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;

    let mut runs = Vec::new();
    if let Some(arr) = body.as_array() {
        for item in arr {
            if let Some(results) = item.get("result").and_then(|r| r.as_array()) {
                for v in results {
                    if let Ok(run) = serde_json::from_value::<AgentRun>(v.clone()) {
                        runs.push(run);
                    }
                }
            }
        }
    }

    Ok(runs)
}

fn format_duration(ms: u64) -> String {
    match ms {
        0 => "—".into(),
        1..=999 => format!("{}ms", ms),
        1000..=59999 => format!("{:.1}s", ms as f64 / 1000.0),
        60000..=3599999 => {
            let m = ms / 60000;
            let s = (ms % 60000) / 1000;
            if s == 0 { format!("{}m", m) } else { format!("{}m {}s", m, s) }
        }
        _ => { let h = ms / 3600000; let m = (ms % 3600000) / 60000; format!("{}h {}m", h, m) }
    }
}

fn generate_markdown_report(benchmarks: &[GoalBenchmark]) -> String {
    let mut report = String::from("# Goal Benchmark Report\n\n");
    report.push_str(&format!("Generated: {}\n\n", chrono::Utc::now().to_rfc3339()));

    report.push_str("## Goals\n\n");
    report.push_str("| Goal | Status | Agents (P/F/S) | Tokens (in/out) | Duration | Models | Files |\n");
    report.push_str("|------|--------|----------------|-----------------|----------|--------|-------|\n");

    for b in benchmarks {
        let goal_short: String = b.goal.chars().take(50).collect();
        let dur = format_duration(b.total_duration_ms);
        report.push_str(&format!(
            "| {} | {} | {}/{}/{} | {}/{} | {} | {} | {} |\n",
            goal_short, b.status,
            b.agents_passed, b.agents_failed, b.agents_skipped,
            b.total_tokens_in, b.total_tokens_out,
            dur,
            b.models_used.join(", "),
            b.files_modified.len(),
        ));
    }

    let total_goals = benchmarks.len();
    let passed = benchmarks.iter().filter(|b| b.status == "passed").count();
    let total_tokens: u64 = benchmarks.iter().map(|b| b.total_tokens_in + b.total_tokens_out).sum();
    let total_time: u64 = benchmarks.iter().map(|b| b.total_duration_ms).sum();
    let total_agents: usize = benchmarks.iter().map(|b| b.total_agents).sum();

    report.push_str("\n## Summary\n\n");
    report.push_str(&format!("- **Goals**: {} (passed: {}, failed: {})\n", total_goals, passed, total_goals - passed));
    report.push_str(&format!("- **Pass rate**: {:.0}%\n", if total_goals > 0 { passed as f64 / total_goals as f64 * 100.0 } else { 0.0 }));
    report.push_str(&format!("- **Total agents**: {}\n", total_agents));
    report.push_str(&format!("- **Total tokens**: {} ({}/goal avg)\n", total_tokens, if total_goals > 0 { total_tokens / total_goals as u64 } else { 0 }));
    report.push_str(&format!("- **Total time**: {} ({}/goal avg)\n", format_duration(total_time), format_duration(if total_goals > 0 { total_time / total_goals as u64 } else { 0 })));

    if !benchmarks.iter().all(|b| b.errors.is_empty()) {
        report.push_str("\n## Errors\n\n");
        for b in benchmarks {
            if !b.errors.is_empty() {
                let goal_short: String = b.goal.chars().take(40).collect();
                report.push_str(&format!("### {}\n", goal_short));
                for e in &b.errors {
                    let e_short: String = e.chars().take(200).collect();
                    report.push_str(&format!("- {}\n", e_short));
                }
                report.push('\n');
            }
        }
    }

    report
}
