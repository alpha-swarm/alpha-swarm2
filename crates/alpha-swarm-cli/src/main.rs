use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use agent_core::{Agent, KnowledgeConfig};
use inference_client::{ClaudeBackend, Complexity, InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeStore;

#[derive(Parser)]
#[command(name = "alpha-swarm", about = "Distributed agent orchestration system")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a one-shot agent task on a repository
    Run {
        /// Path to the repository
        #[arg(short, long)]
        repo: PathBuf,

        /// Task description
        #[arg(short, long)]
        task: String,

        /// Files to include as context (glob or explicit paths)
        #[arg(short, long)]
        files: Vec<String>,

        /// Complexity tier: simple, medium, complex
        #[arg(short, long, default_value = "medium")]
        complexity: String,

        /// Skip quality gate
        #[arg(long)]
        no_quality_gate: bool,

        /// Project name for knowledge base (enables knowledge features)
        #[arg(short, long)]
        project: Option<String>,
    },

    /// List available models across all backends
    Models,

    /// Check health of all backends
    Health,

    /// Run a swarm: decompose a goal into parallel agent tasks
    Swarm {
        /// Path to the repository
        #[arg(short, long)]
        repo: PathBuf,

        /// High-level goal description
        #[arg(short, long)]
        goal: String,

        /// Project name for knowledge base
        #[arg(short, long, default_value = "default")]
        project: String,
    },

    /// Show past agent runs from knowledge base
    History {
        /// Project name
        #[arg(short, long)]
        project: String,
    },
}

fn setup_router() -> Result<InferenceRouter> {
    let mut router = InferenceRouter::new();

    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        let model = std::env::var("ALPHA_SWARM_CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".into());
        info!("Claude backend configured (model: {model})");
        router = router.add_backend(ClaudeBackend::new(api_key).with_model(model));
    }

    let ollama_url = std::env::var("ALPHA_SWARM_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".into());
    info!("Ollama backend configured ({ollama_url})");
    router = router.add_backend(OllamaBackend::new(&ollama_url));

    Ok(router)
}

fn get_ollama() -> OllamaBackend {
    let ollama_url = std::env::var("ALPHA_SWARM_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".into());
    OllamaBackend::new(ollama_url)
}

async fn get_knowledge_store() -> Result<Option<KnowledgeStore>> {
    let url = std::env::var("ALPHA_SWARM_SURREALDB_URL")
        .unwrap_or_else(|_| "127.0.0.1:8000".into());
    let ns = std::env::var("ALPHA_SWARM_SURREALDB_NS")
        .unwrap_or_else(|_| "alpha_swarm".into());
    let db = std::env::var("ALPHA_SWARM_SURREALDB_DB")
        .unwrap_or_else(|_| "swarm".into());

    match KnowledgeStore::connect(&url, &ns, &db).await {
        Ok(store) => Ok(Some(store)),
        Err(e) => {
            tracing::warn!("Knowledge base unavailable: {e}");
            Ok(None)
        }
    }
}

fn parse_complexity(s: &str) -> Complexity {
    match s.to_lowercase().as_str() {
        "simple" => Complexity::Simple,
        "complex" => Complexity::Complex,
        _ => Complexity::Medium,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("alpha_swarm=info".parse().unwrap())
                .add_directive("agent_core=info".parse().unwrap())
                .add_directive("inference_client=info".parse().unwrap())
                .add_directive("quality_gate_lib=info".parse().unwrap())
                .add_directive("knowledge_base=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    let router = setup_router()?;

    match cli.command {
        Commands::Run {
            repo,
            task,
            files,
            complexity,
            no_quality_gate,
            project,
        } => {
            let repo = repo.canonicalize()
                .context("Repository path does not exist")?;
            let complexity = parse_complexity(&complexity);

            info!(repo = %repo.display(), task = %task, "Starting agent run");

            let files = if files.is_empty() {
                discover_files(&repo)?
            } else {
                files
            };

            // Set up knowledge base if project is specified
            let kb = if project.is_some() {
                get_knowledge_store().await?
            } else {
                None
            };
            let ollama = get_ollama();

            let mut agent = Agent::new(&router, &repo);

            if let (Some(proj), Some(store)) = (&project, &kb) {
                let embed_model = std::env::var("ALPHA_SWARM_EMBED_MODEL")
                    .unwrap_or_else(|_| "qwen2.5-coder:7b".into());
                agent = agent.with_knowledge(KnowledgeConfig {
                    store,
                    embedder: &ollama,
                    embed_model,
                    project: proj.clone(),
                    skip_threshold: 0.9,
                });
                info!(project = %proj, "Knowledge base enabled");
            }

            let result = agent.run(&task, &files, complexity).await?;

            // Print results
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

            // Quality gate
            if !no_quality_gate && result.applied {
                println!("\n=== Quality Gate ===");
                let config = quality_gate_lib::detect_toolchain(&repo);
                let checks = quality_gate_lib::run_all(&repo, &config).await?;

                let all_passed = checks.iter().all(|c| c.passed);

                // Update knowledge base with quality gate result
                if let (Some(store), Some(id)) = (&kb, &result.run_id) {
                    let mut run = knowledge_base::AgentRun::new(
                        project.as_deref().unwrap_or(""),
                        &task,
                        "",
                        &result.inference_response.model,
                    );
                    run.quality_gate_passed = Some(all_passed);
                    run.status = if all_passed {
                        knowledge_base::RunStatus::Passed
                    } else {
                        knowledge_base::RunStatus::Failed
                    };
                    if !all_passed {
                        let errors: String = checks.iter()
                            .filter(|c| !c.passed)
                            .map(|c| format!("{}: {}", c.check_name, c.stderr.chars().take(200).collect::<String>()))
                            .collect::<Vec<_>>()
                            .join("\n");
                        run.error_message = Some(errors);
                    }
                    let _ = store.update_run(id, &run).await;
                }

                for check in &checks {
                    let status = if check.passed { "PASS" } else { "FAIL" };
                    println!("  [{status}] {} ({}ms)", check.check_name, check.duration_ms);
                    if !check.passed && !check.stderr.is_empty() {
                        for line in check.stderr.lines().take(20) {
                            println!("    {line}");
                        }
                    }
                }

                if all_passed {
                    println!("\nAll checks passed.");
                } else {
                    println!("\nSome checks failed.");
                    std::process::exit(1);
                }
            }
        }

        Commands::Models => {
            let models = router.list_models().await?;
            if models.is_empty() {
                println!("No models available.");
            } else {
                println!("{:<30} {:<10} {:<15} {:<10}", "MODEL", "BACKEND", "PARAMS", "CTX");
                for m in models {
                    println!(
                        "{:<30} {:<10} {:<15} {:<10}",
                        m.name,
                        format!("{:?}", m.backend),
                        m.parameter_size,
                        m.context_window,
                    );
                }
            }
        }

        Commands::Health => {
            let statuses = router.list_backends().await;
            for (kind, healthy) in statuses {
                let status = if healthy { "healthy" } else { "unreachable" };
                println!("{:?}: {status}", kind);
            }
        }

        Commands::Swarm { repo, goal, project } => {
            let repo = repo.canonicalize()
                .context("Repository path does not exist")?;

            info!(repo = %repo.display(), goal = %goal, project = %project, "Starting swarm");

            let ollama = get_ollama();
            let kb = get_knowledge_store().await?;

            let mut runner = swarm_orchestrator::SwarmRunner::new(&router, &ollama, &repo, &project);
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
                } else {
                    "ERR"
                };
                let edits = tr.agent_result.as_ref().map(|r| r.edits.len()).unwrap_or(0);
                println!(
                    "  [{status}] {} — {} (edits: {}, files: {:?})",
                    tr.task.id, tr.task.description, edits, tr.task.files,
                );
                if let Some(err) = &tr.error {
                    println!("         error: {err}");
                }
            }

            if let Some(diff) = &result.merged_diff {
                if !diff.is_empty() {
                    println!("\n--- Merged Diff ---");
                    for line in diff.lines().take(50) {
                        println!("{line}");
                    }
                    if diff.lines().count() > 50 {
                        println!("... ({} more lines)", diff.lines().count() - 50);
                    }
                }
            }

            if !result.quality_passed {
                std::process::exit(1);
            }
        }

        Commands::History { project } => {
            let kb = get_knowledge_store().await?
                .context("Knowledge base not available")?;
            let runs = kb.list_runs(&project, None).await?;
            if runs.is_empty() {
                println!("No runs found for project '{project}'.");
            } else {
                println!("{:<12} {:<10} {:<20} {:<40}", "STATUS", "MODEL", "CREATED", "TASK");
                for run in &runs {
                    let status = format!("{:?}", run.status);
                    let task_short: String = run.task_description.chars().take(38).collect();
                    println!(
                        "{:<12} {:<10} {:<20} {:<40}",
                        status,
                        run.model_used.chars().take(10).collect::<String>(),
                        run.created_at.chars().take(19).collect::<String>(),
                        task_short,
                    );
                }
                println!("\n{} total runs", runs.len());
            }
        }
    }

    Ok(())
}

fn discover_files(repo: &PathBuf) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let extensions = ["rs", "ts", "js", "go", "py"];

    fn walk(dir: &std::path::Path, base: &std::path::Path, ext: &[&str], out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                walk(&path, base, ext, out);
            } else if let Some(e) = path.extension().and_then(|e| e.to_str()) {
                if ext.contains(&e) {
                    if let Ok(rel) = path.strip_prefix(base) {
                        out.push(rel.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    walk(repo, repo, &extensions, &mut files);
    files.sort();

    if files.len() > 20 {
        info!("Found {} files, taking first 20", files.len());
        files.truncate(20);
    }

    Ok(files)
}
