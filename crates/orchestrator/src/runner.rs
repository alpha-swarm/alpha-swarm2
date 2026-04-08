use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, warn, error};

use agent_core::{Agent, AgentResult, KnowledgeConfig};
use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeStore;

use crate::planner::{SubTask, plan_goal};
use crate::memtree::MemTreeManager;

/// Result of a full swarm run.
pub struct SwarmResult {
    pub goal: String,
    pub tasks: Vec<SubTask>,
    pub results: Vec<TaskRunResult>,
    pub merged_diff: Option<String>,
    pub quality_passed: bool,
    pub total_duration_ms: u64,
}

pub struct TaskRunResult {
    pub task: SubTask,
    pub agent_result: Option<AgentResult>,
    pub error: Option<String>,
}

/// Runs multiple agents in parallel on a goal.
pub struct SwarmRunner {
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Option<Arc<KnowledgeStore>>,
    repo_path: PathBuf,
    project: String,
    parent_run_id: Option<String>,
    max_concurrent: usize,
    nats_client: Option<async_nats::Client>,
    planner_tier: swarm_config::TierConfig,
}

/// Default concurrency when not configured
const DEFAULT_MAX_CONCURRENT: usize = 2;

impl SwarmRunner {
    pub fn new(
        router: Arc<InferenceRouter>,
        ollama: Arc<OllamaBackend>,
        repo_path: impl Into<PathBuf>,
        project: impl Into<String>,
    ) -> Self {
        Self {
            router,
            ollama,
            store: None,
            repo_path: repo_path.into(),
            project: project.into(),
            parent_run_id: None,
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            nats_client: None,
            planner_tier: swarm_config::TierConfig::orchestrator(),
        }
    }

    pub fn with_store(mut self, store: Arc<KnowledgeStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_parent_run_id(mut self, id: impl Into<String>) -> Self {
        self.parent_run_id = Some(id.into());
        self
    }

    pub fn with_max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = n.max(1);
        self
    }

    pub fn with_nats_client(mut self, client: async_nats::Client) -> Self {
        self.nats_client = Some(client);
        self
    }

    pub fn with_planner_tier(mut self, tier: swarm_config::TierConfig) -> Self {
        self.planner_tier = tier;
        self
    }

    /// Execute a high-level goal: plan → spawn agents in parallel → merge → validate.
    pub async fn run(&self, goal: &str) -> Result<SwarmResult> {
        let start = Instant::now();

        // 1. Discover repo files
        // Note: file embedding indexing is handled by EmbeddingManager lifecycle
        // hooks in executor.rs (on_agent_start / on_agent_done).
        let repo_files = discover_source_files(&self.repo_path)?;
        info!(file_count = repo_files.len(), "Discovered repo files");

        // 2. Plan: decompose goal into sub-tasks
        let tasks = plan_goal(&self.router, goal, &repo_files, &self.planner_tier).await
            .context("Goal planning failed")?;

        info!(task_count = tasks.len(), "Goal decomposed");

        // 3. Create workspaces for each agent (temp dirs with file copies)
        let mut mem_manager = MemTreeManager::new(&self.repo_path);
        let mut agent_paths = Vec::new();

        for task in &tasks {
            let ws_path = mem_manager.create(&task.id, &task.files)
                .with_context(|| format!("Failed to create workspace for {}", task.id))?;
            agent_paths.push((task.clone(), ws_path));
        }

        // 4. Run agents in PARALLEL using JoinSet (bounded by semaphore)
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));
        let mut join_set = tokio::task::JoinSet::new();

        info!(max_concurrent = self.max_concurrent, tasks = agent_paths.len(), "Spawning agents");

        for (task, wt_path) in agent_paths {
            let sem = Arc::clone(&semaphore);
            let router = Arc::clone(&self.router);
            let ollama = Arc::clone(&self.ollama);
            let store = self.store.clone();
            let project = self.project.clone();
            let parent_id = self.parent_run_id.clone();
            let _nats_client = self.nats_client.clone();
            let embed_model = std::env::var("ALPHA_SWARM_EMBED_MODEL")
                .unwrap_or_else(|_| swarm_config::DefaultsConfig::default().embed_model);

            join_set.spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");
                let agent = Agent::new(Arc::clone(&router), &wt_path);
                let agent = if let Some(kb) = store {
                    agent.with_knowledge(KnowledgeConfig {
                        store: kb,
                        embedder: ollama,
                        embed_model,
                        project,
                        skip_threshold: 0.9,
                        parent_run_id: parent_id,
                    })
                } else {
                    agent
                };

                info!(task_id = %task.id, desc = %task.description, "Running agent");

                // Tool-use loop: model makes small focused calls (read_file, grep, etc.)
                // then produces edits. Much faster than one big LLM call with all files.
                let tools = swarm_tools::ToolRegistry::with_defaults();
                const MAX_TOOL_STEPS: u32 = 10;
                let result = agent.run_with_tools(&task.description, &task.files, task.complexity, &tools, MAX_TOOL_STEPS).await;
                (task, result)
            });
        }

        // 5. Collect results as they complete
        let mut results = Vec::new();
        let mut any_applied = false;

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((task, Ok(agent_result))) => {
                    if agent_result.applied {
                        any_applied = true;
                        info!(task_id = %task.id, edits = agent_result.edits.len(), "Agent completed with edits");
                    } else if agent_result.skipped {
                        info!(task_id = %task.id, "Agent skipped (already done)");
                    }
                    results.push(TaskRunResult {
                        task,
                        agent_result: Some(agent_result),
                        error: None,
                    });
                }
                Ok((task, Err(e))) => {
                    error!(task_id = %task.id, error = %e, "Agent failed");
                    results.push(TaskRunResult {
                        task,
                        agent_result: None,
                        error: Some(e.to_string()),
                    });
                }
                Err(e) => {
                    warn!("Agent task panicked: {e}");
                }
            }
        }

        // 6. Commit changes from workspaces using git2 (in-memory tree)
        let mut merged_diff = String::new();
        if any_applied {
            for result in &results {
                if result.agent_result.as_ref().is_some_and(|r| r.applied) {
                    let msg = format!("agent({}): {}", result.task.id, result.task.description);
                    match mem_manager.commit_changes(&result.task.id, &msg) {
                        Ok(commit_result) => {
                            if commit_result.has_changes {
                                merged_diff.push_str(&commit_result.diff);
                                if let Err(e) = mem_manager.apply_to_working_dir(&commit_result) {
                                    warn!(task_id = %result.task.id, "Apply to working dir failed: {e}");
                                }
                            } else {
                                info!(task_id = %result.task.id, "No changes detected in workspace");
                            }
                        }
                        Err(e) => {
                            warn!(task_id = %result.task.id, "Commit failed: {e}");
                        }
                    }
                }
            }
        }

        // 7. Run quality gate on merged result — skip if only non-code files modified
        let code_extensions = ["rs", "ts", "js", "go", "py"];
        let modified_files: Vec<String> = results.iter()
            .filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied))
            .flat_map(|r| r.task.files.iter().cloned())
            .collect();
        let has_code_changes = modified_files.iter().any(|f| {
            code_extensions.iter().any(|ext| f.ends_with(&format!(".{ext}")))
        });

        let quality_passed = if any_applied && has_code_changes {
            // Auto-format before quality gate — models often get whitespace wrong
            if self.repo_path.join("Cargo.toml").exists() {
                let _ = std::process::Command::new("cargo")
                    .args(["fmt"])
                    .current_dir(&self.repo_path)
                    .output();
                info!("Auto-formatted with cargo fmt before quality gate");
            }

            info!("Running quality gate (code files modified)");
            let config = quality_gate_lib::detect_toolchain(&self.repo_path);
            match quality_gate_lib::run_all(&self.repo_path, &config).await {
                Ok(checks) => {
                    let passed = checks.iter().all(|c| c.passed);
                    for check in &checks {
                        let status = if check.passed { "PASS" } else { "FAIL" };
                        info!(check = %check.check_name, status, "Quality gate");
                    }
                    passed
                }
                Err(e) => {
                    warn!("Quality gate error: {e}");
                    false
                }
            }
        } else if any_applied {
            info!("Skipping quality gate (only non-code files modified: {:?})", modified_files);
            true
        } else {
            true
        };

        // 8. Cleanup workspaces
        mem_manager.cleanup();

        let total_duration_ms = start.elapsed().as_millis() as u64;

        Ok(SwarmResult {
            goal: goal.to_string(),
            tasks,
            results,
            merged_diff: if merged_diff.is_empty() { None } else { Some(merged_diff) },
            quality_passed,
            total_duration_ms,
        })
    }
}

fn discover_source_files(repo: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let extensions = ["rs", "ts", "js", "go", "py", "md", "toml", "json", "yaml", "yml"];

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
            } else if let Some(e) = path.extension().and_then(|e| e.to_str())
                && ext.contains(&e)
                && let Ok(rel) = path.strip_prefix(base) {
                    out.push(rel.to_string_lossy().to_string());
            }
        }
    }

    walk(repo, repo, &extensions, &mut files);
    files.sort();
    Ok(files)
}

/// Index file embeddings for RAG — runs in background, best-effort.
/// For each source file, creates a summary (first line + signature) and embeds it.
#[allow(dead_code)]
async fn index_file_embeddings(
    store: &knowledge_base::KnowledgeStore,
    ollama: &inference_client::OllamaBackend,
    project: &str,
    repo_path: &std::path::Path,
    files: &[String],
) {
    let embed_model = std::env::var("ALPHA_SWARM_EMBED_MODEL")
        .unwrap_or_else(|_| swarm_config::DefaultsConfig::default().embed_model);

    /// Max files to embed per run (avoid overloading Ollama).
    const MAX_FILES_TO_EMBED: usize = 50;
    /// Max chars of file content to summarize for embedding.
    const SUMMARY_CHARS: usize = 500;

    let mut indexed = 0;
    for file_path in files.iter().take(MAX_FILES_TO_EMBED) {
        let full_path = repo_path.join(file_path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Build summary: filename + first few lines + signatures
        let lines: Vec<&str> = content.lines().collect();
        let first_lines: String = lines.iter().take(5).cloned().collect::<Vec<_>>().join("\n");
        let sigs: Vec<String> = lines.iter()
            .filter(|l| {
                let t = l.trim();
                t.starts_with("pub fn ") || t.starts_with("fn ") || t.starts_with("pub struct ")
                    || t.starts_with("struct ") || t.starts_with("impl ") || t.starts_with("pub trait ")
            })
            .take(10)
            .map(|l| l.trim().to_string())
            .collect();

        let summary = format!("{file_path}\n{first_lines}\n{}", sigs.join("\n"));
        let summary_truncated: String = summary.chars().take(SUMMARY_CHARS).collect();

        // Embed the summary
        match ollama.embed(&embed_model, &summary_truncated).await {
            Ok(embedding) => {
                let _ = store.store_file_embedding(project, file_path, &summary_truncated, &embedding).await;
                indexed += 1;
            }
            Err(_) => continue,
        }
    }

    if indexed > 0 {
        info!(indexed, total = files.len(), "Indexed file embeddings for RAG");
    }
}
