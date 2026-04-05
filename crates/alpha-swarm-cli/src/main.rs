mod commands;
mod setup;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use inference_client::Complexity;

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
        #[arg(short, long)]
        repo: PathBuf,
        #[arg(short, long)]
        task: String,
        #[arg(short, long)]
        files: Vec<String>,
        #[arg(short, long, default_value = "medium")]
        complexity: String,
        #[arg(long)]
        no_quality_gate: bool,
        #[arg(short, long)]
        project: Option<String>,
        /// Agent specialization: general, lint, test, refactor, feature, bug
        #[arg(short, long, default_value = "general")]
        agent_type: String,
        /// Enable retry with model escalation on quality gate failure
        #[arg(long)]
        retry: bool,
    },
    /// List available models across all backends
    Models,
    /// Check health of all backends
    Health,
    /// Run a swarm: decompose a goal into parallel agent tasks
    Swarm {
        #[arg(short, long)]
        repo: PathBuf,
        #[arg(short, long)]
        goal: String,
        #[arg(short, long, default_value = "default")]
        project: String,
    },
    /// Show past agent runs from knowledge base
    History {
        #[arg(short, long)]
        project: String,
    },
    /// Show aggregated metrics for a project
    Metrics {
        #[arg(short, long)]
        project: String,
    },
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
    let router = setup::setup_router()?;

    match cli.command {
        Commands::Run { repo, task, files, complexity, no_quality_gate, project, agent_type, retry } => {
            commands::run::execute(&router, repo, task, files, parse_complexity(&complexity), no_quality_gate, project, &agent_type, retry).await?;
        }
        Commands::Swarm { repo, goal, project } => {
            commands::swarm::execute(&router, repo, goal, project).await?;
        }
        Commands::Models => {
            let models = router.list_models().await?;
            if models.is_empty() {
                println!("No models available.");
            } else {
                println!("{:<30} {:<10} {:<15} {:<10}", "MODEL", "BACKEND", "PARAMS", "CTX");
                for m in models {
                    println!("{:<30} {:<10} {:<15} {:<10}", m.name, format!("{:?}", m.backend), m.parameter_size, m.context_window);
                }
            }
        }
        Commands::Health => {
            for (kind, healthy) in router.list_backends().await {
                println!("{:?}: {}", kind, if healthy { "healthy" } else { "unreachable" });
            }
        }
        Commands::Metrics { project } => {
            let kb = setup::get_knowledge_store().await?.context("Knowledge base not available")?;
            let runs = kb.list_runs(&project, None).await?;
            let metrics = knowledge_base::ProjectMetrics::from_runs(&runs);

            println!("\n=== Metrics: {project} ===");
            println!("Total runs:  {}", metrics.total_runs);
            println!("Passed:      {} ({:.0}%)", metrics.passed, metrics.pass_rate * 100.0);
            println!("Failed:      {}", metrics.failed);
            println!("Skipped:     {}", metrics.skipped);
            println!("Tokens:      {} in / {} out", metrics.total_tokens_input, metrics.total_tokens_output);
            println!("Avg duration: {}ms", metrics.avg_duration_ms);

            if !metrics.models_used.is_empty() {
                println!("\n--- Per Model ---");
                println!("{:<25} {:<6} {:<6} {:<6} {:<8} {:<10} {:<10}", "MODEL", "RUNS", "PASS", "FAIL", "RATE", "AVG TOK", "AVG MS");
                for m in &metrics.models_used {
                    println!("{:<25} {:<6} {:<6} {:<6} {:<8.0}% {:<10} {:<10}",
                        m.model.chars().take(24).collect::<String>(),
                        m.runs, m.passed, m.failed,
                        m.pass_rate * 100.0,
                        m.avg_tokens_output,
                        m.avg_duration_ms,
                    );
                }
            }
        }

        Commands::History { project } => {
            let kb = setup::get_knowledge_store().await?.context("Knowledge base not available")?;
            let runs = kb.list_runs(&project, None).await?;
            if runs.is_empty() {
                println!("No runs found for project '{project}'.");
            } else {
                println!("{:<12} {:<10} {:<20} {:<40}", "STATUS", "MODEL", "CREATED", "TASK");
                for run in &runs {
                    println!("{:<12} {:<10} {:<20} {:<40}",
                        format!("{:?}", run.status),
                        run.model_used.chars().take(10).collect::<String>(),
                        run.created_at.chars().take(19).collect::<String>(),
                        run.task_description.chars().take(38).collect::<String>(),
                    );
                }
                println!("\n{} total runs", runs.len());
            }
        }
    }

    Ok(())
}
