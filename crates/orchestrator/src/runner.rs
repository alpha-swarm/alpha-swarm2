use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, warn, error};

use agent_core::{Agent, AgentResult, KnowledgeConfig};
use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeStore;

use crate::planner::{SubTask, plan_goal};
use crate::worktree::WorktreeManager;

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
pub struct SwarmRunner<'a> {
    router: &'a InferenceRouter,
    ollama: &'a OllamaBackend,
    store: Option<&'a KnowledgeStore>,
    repo_path: PathBuf,
    project: String,
}

impl<'a> SwarmRunner<'a> {
    pub fn new(
        router: &'a InferenceRouter,
        ollama: &'a OllamaBackend,
        repo_path: impl Into<PathBuf>,
        project: impl Into<String>,
    ) -> Self {
        Self {
            router,
            ollama,
            store: None,
            repo_path: repo_path.into(),
            project: project.into(),
        }
    }

    pub fn with_store(mut self, store: &'a KnowledgeStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Execute a high-level goal: plan → spawn agents → merge → validate.
    pub async fn run(&self, goal: &str) -> Result<SwarmResult> {
        let start = Instant::now();

        // 1. Discover repo files
        let repo_files = discover_source_files(&self.repo_path)?;
        info!(file_count = repo_files.len(), "Discovered repo files");

        // 2. Plan: decompose goal into sub-tasks
        let tasks = plan_goal(self.router, goal, &repo_files).await
            .context("Goal planning failed")?;

        info!(task_count = tasks.len(), "Goal decomposed");

        // 3. Create worktrees for each agent
        let mut wt_manager = WorktreeManager::new(&self.repo_path);
        let mut agent_paths = Vec::new();

        for task in &tasks {
            let wt_path = wt_manager.create(&task.id)
                .with_context(|| format!("Failed to create worktree for {}", task.id))?;
            agent_paths.push((task.clone(), wt_path));
        }

        // 4. Run agents in parallel
        let mut handles = Vec::new();

        for (task, wt_path) in agent_paths {
            let router = self.router;
            let ollama = self.ollama;
            let store = self.store;
            let project = self.project.clone();
            let embed_model = std::env::var("ALPHA_SWARM_EMBED_MODEL")
                .unwrap_or_else(|_| swarm_config::DefaultsConfig::default().embed_model);

            // Can't move references into spawn — need to use scoped tasks
            // For now, run sequentially but with worktree isolation
            let agent = Agent::new(router, &wt_path);
            let agent = if let Some(kb) = store {
                agent.with_knowledge(KnowledgeConfig {
                    store: kb,
                    embedder: ollama,
                    embed_model: embed_model.clone(),
                    project: project.clone(),
                    skip_threshold: 0.9,
                })
            } else {
                agent
            };

            info!(task_id = %task.id, desc = %task.description, "Running agent");
            let result = agent.run(&task.description, &task.files, task.complexity).await;

            handles.push((task, result));
        }

        // 5. Collect results
        let mut results = Vec::new();
        let mut any_applied = false;

        for (task, result) in handles {
            match result {
                Ok(agent_result) => {
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
                Err(e) => {
                    error!(task_id = %task.id, error = %e, "Agent failed");
                    results.push(TaskRunResult {
                        task,
                        agent_result: None,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        // 6. Merge diffs back to main repo
        let mut merged_diff = String::new();
        if any_applied {
            for result in &results {
                if result.agent_result.as_ref().is_some_and(|r| r.applied) {
                    match wt_manager.apply_diff_to_main(&result.task.id) {
                        Ok(()) => {
                            if let Ok(diff) = wt_manager.extract_diff(&result.task.id) {
                                merged_diff.push_str(&diff);
                            }
                        }
                        Err(e) => {
                            warn!(task_id = %result.task.id, "Merge failed: {e}");
                        }
                    }
                }
            }
        }

        // 7. Run quality gate on merged result
        let quality_passed = if any_applied {
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
        } else {
            true
        };

        // 8. Cleanup worktrees
        wt_manager.cleanup();

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

fn discover_source_files(repo: &PathBuf) -> Result<Vec<String>> {
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
    Ok(files)
}
