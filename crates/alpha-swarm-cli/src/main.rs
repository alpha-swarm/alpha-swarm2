use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use agent_core::Agent;
use inference_client::{ClaudeBackend, Complexity, InferenceRouter, OllamaBackend};

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

        /// Run quality gate after applying changes
        #[arg(long, default_value_t = true)]
        quality_gate: bool,
    },

    /// List available models across all backends
    Models,

    /// Check health of all backends
    Health,
}

fn setup_router() -> Result<InferenceRouter> {
    let mut router = InferenceRouter::new();

    // Claude backend (if API key is set)
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        let model = std::env::var("ALPHA_SWARM_CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".into());
        info!("Claude backend configured (model: {model})");
        router = router.add_backend(ClaudeBackend::new(api_key).with_model(model));
    }

    // Ollama backend
    let ollama_url = std::env::var("ALPHA_SWARM_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".into());
    info!("Ollama backend configured ({ollama_url})");
    router = router.add_backend(OllamaBackend::new(ollama_url));

    Ok(router)
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
                .add_directive("quality_gate_lib=info".parse().unwrap()),
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
            quality_gate,
        } => {
            let repo = repo.canonicalize()
                .context("Repository path does not exist")?;
            let complexity = parse_complexity(&complexity);

            info!(repo = %repo.display(), task = %task, "Starting agent run");

            // Discover files if none specified
            let files = if files.is_empty() {
                discover_files(&repo)?
            } else {
                files
            };

            // Run agent
            let agent = Agent::new(&router, &repo);
            let result = agent.run(&task, &files, complexity).await?;

            // Print results
            println!("\n=== Agent Result ===");
            println!("Model:    {} ({:?})", result.inference_response.model, result.inference_response.backend);
            println!("Tokens:   {} in / {} out", result.inference_response.tokens_input, result.inference_response.tokens_output);
            println!("Duration: {}ms", result.inference_response.duration_ms);
            println!("Edits:    {}", result.edits.len());
            println!("Applied:  {}", result.applied);

            for edit in &result.edits {
                match edit {
                    agent_core::FileEdit::Edit { path, .. } => println!("  EDIT   {path}"),
                    agent_core::FileEdit::Create { path, .. } => println!("  CREATE {path}"),
                    agent_core::FileEdit::Delete { path } => println!("  DELETE {path}"),
                }
            }

            // Quality gate
            if quality_gate && result.applied {
                println!("\n=== Quality Gate ===");
                let config = quality_gate_lib::detect_toolchain(&repo);
                let checks = quality_gate_lib::run_all(&repo, &config).await?;

                let all_passed = checks.iter().all(|c| c.passed);
                for check in &checks {
                    let status = if check.passed { "PASS" } else { "FAIL" };
                    println!("  [{status}] {} ({}ms)", check.check_name, check.duration_ms);
                    if !check.passed {
                        if !check.stderr.is_empty() {
                            for line in check.stderr.lines().take(20) {
                                println!("    {line}");
                            }
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
    }

    Ok(())
}

/// Discover source files in the repo (simple heuristic for now).
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

    // Limit to avoid blowing up context
    if files.len() > 20 {
        info!("Found {} files, taking first 20", files.len());
        files.truncate(20);
    }

    Ok(files)
}
