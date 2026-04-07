use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use inference_client::{Complexity, InferenceRouter};
use tracing::info;

use agent_core::{Agent, KnowledgeConfig};
use swarm_config::SwarmConfig;
use crate::setup;

pub async fn execute(
    router: Arc<InferenceRouter>,
    config: &SwarmConfig,
    repo: PathBuf,
    task: String,
    files: Vec<String>,
    complexity: Complexity,
    no_quality_gate: bool,
    project: Option<String>,
    agent_type: &str,
    retry: bool,
) -> Result<()> {
    let _agent_type = agent_core::AgentType::from_str(agent_type);
    let repo = repo.canonicalize().context("Repository path does not exist")?;
    info!(repo = %repo.display(), task = %task, "Starting agent run");

    let files = if files.is_empty() { setup::discover_files(&repo)? } else { files };

    let kb = if project.is_some() { setup::get_knowledge_store(config).await? } else { None };
    let ollama = Arc::new(setup::get_ollama(config));
    let events = setup::get_event_publisher(config).await?;
    let mut agent = Agent::new(Arc::clone(&router), &repo);
    if let Some(pub_) = events {
        agent = agent.with_events(Arc::new(pub_));
    }
    let kb_arc = kb.map(Arc::new);
    if let (Some(proj), Some(store)) = (&project, &kb_arc) {
        let embed_model = config.defaults.embed_model.clone();
        agent = agent.with_knowledge(KnowledgeConfig {
            store: Arc::clone(store),
            embedder: Arc::clone(&ollama),
            embed_model,
            project: proj.clone(),
            skip_threshold: 0.9,
            parent_run_id: None,
        });
        info!(project = %proj, "Knowledge base enabled");
    }

    let result = agent.run(&task, &files, complexity).await?;

    println!("\n=== Agent Result ===");
    if result.skipped {
        println!("SKIPPED: {}", result.inference_response.content);
        return Ok(());
    }

    println!("Model:    {} ({:?})", result.inference_response.model, result.inference_response.backend);
    println!("Tokens:   {} in / {} out", result.inference_response.tokens_input, result.inference_response.tokens_output);
    println!("Duration: {}ms", result.inference_response.duration_ms);
    println!("Edits:    {}", result.edits.len());
    println!("Applied:  {}", result.applied);
    if let Some(id) = &result.run_id {
        println!("Run ID:   {id}");
    }

    for edit in &result.edits {
        match edit {
            agent_core::FileEdit::Edit { path, .. } => println!("  EDIT   {path}"),
            agent_core::FileEdit::Create { path, .. } => println!("  CREATE {path}"),
            agent_core::FileEdit::Delete { path } => println!("  DELETE {path}"),
        }
    }

    if !no_quality_gate && result.applied {
        println!("\n=== Quality Gate ===");
        let config = quality_gate_lib::detect_toolchain(&repo);
        let checks = quality_gate_lib::run_all(&repo, &config).await?;
        let all_passed = checks.iter().all(|c| c.passed);

        if let (Some(store), Some(id)) = (&kb_arc, &result.run_id) {
            let mut run = knowledge_base::AgentRun::new(
                project.as_deref().unwrap_or(""), &task, "", &result.inference_response.model,
            );
            run.quality_gate_passed = Some(all_passed);
            run.status = if all_passed { knowledge_base::RunStatus::Passed } else { knowledge_base::RunStatus::Failed };
            if !all_passed {
                run.error_message = Some(checks.iter()
                    .filter(|c| !c.passed)
                    .map(|c| format!("{}: {}", c.check_name, c.stderr.chars().take(200).collect::<String>()))
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
            let _ = store.update_run(id, &run).await;
        }

        for check in &checks {
            let status = if check.passed { "PASS" } else { "FAIL" };
            println!("  [{status}] {} ({}ms)", check.check_name, check.duration_ms);
            if !check.passed && !check.stderr.is_empty() {
                for line in check.stderr.lines().take(20) { println!("    {line}"); }
            }
        }

        if all_passed { println!("\nAll checks passed."); }
        else { println!("\nSome checks failed."); std::process::exit(1); }
    }

    Ok(())
}
