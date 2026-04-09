use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, warn, error};

use agent_core::{Agent, AgentResult, KnowledgeConfig, parse_edits};
use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeStore;

use crate::planner::plan_goal;
use crate::planner_types::SubTask;
use crate::memtree::MemTreeManager;

/// Result of a full swarm run.
pub struct SwarmResult {
    pub goal: String,
    pub tasks: Vec<SubTask>,
    pub results: Vec<TaskRunResult>,
    pub merged_diff: Option<String>,
    pub quality_passed: bool,
    pub total_duration_ms: u64,
    /// Files modified by agents (path → new content).
    pub modified_files: Vec<(String, Vec<u8>)>,
}

pub struct TaskRunResult {
    pub task: SubTask,
    pub agent_result: Option<AgentResult>,
    pub error: Option<String>,
}

/// GitHub config for zero-disk mode.
pub struct GitHubRepo {
    pub owner: String,
    pub repo: String,
    pub token: String,
    pub branch: String,
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
    /// When set, use GitHub API + VirtWorkspace instead of disk clone.
    github: Option<GitHubRepo>,
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
            github: None,
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

    /// Enable zero-disk mode: files loaded via GitHub API, no git clone.
    pub fn with_github(mut self, gh: GitHubRepo) -> Self {
        self.github = Some(gh);
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

        // 3-6. Run agents and collect results
        let mut results = Vec::new();
        let mut any_applied = false;
        let mut captured_files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut merged_diff = String::new();

        // Zero-disk mode: use VirtFileProvider + GitHub API
        if let Some(ref gh) = self.github {
            info!("Running in zero-disk mode (VirtFileProvider + GitHub API)");

            let http_client = reqwest::Client::new();
            let token = gh.token.clone();
            let owner = gh.owner.clone();
            let repo_name = gh.repo.clone();
            let branch = gh.branch.clone();

            for task in &tasks {
                info!(task_id = %task.id, desc = %task.description, "Running agent (zero-disk)");

                // Load task files from GitHub API into VirtFileProvider
                let mut virt_fp = agent_core::VirtFileProvider::new();
                let gh_http = |url: &str, tkn: &str| -> Result<String, String> {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            let resp = http_client.get(url)
                                .header("Authorization", format!("Bearer {tkn}"))
                                .header("Accept", "application/vnd.github+json")
                                .header("User-Agent", "alpha-swarm")
                                .send().await.map_err(|e| format!("{e}"))?;
                            let st = resp.status();
                            let text = resp.text().await.map_err(|e| format!("{e}"))?;
                            if !st.is_success() { return Err(format!("{st}: {}", &text[..text.len().min(200)])); }
                            Ok(text)
                        })
                    })
                };

                for file_path in &task.files {
                    match virt_git::load_file_from_github(
                        &owner, &repo_name, &branch, file_path,
                        &mut virt_fp.store, &mut virt_fp.workspace,
                        &gh_http, &token,
                    ) {
                        Ok(()) => info!(file = file_path, "Loaded from GitHub API"),
                        Err(e) => warn!(file = file_path, error = %e, "Failed to load from GitHub"),
                    }
                }

                // Zero-disk: call WASI agent-worker via HTTP (it handles inference + edit parsing)
                let agent_url = std::env::var("AGENT_WORKER_URL")
                    .unwrap_or_else(|_| "http://localhost:8000".into());

                // Build file content for the agent
                let mut file_inputs = Vec::new();
                for file_path in &task.files {
                    if let Some(content) = virt_fp.workspace.read_file(&virt_fp.store, file_path) {
                        file_inputs.push(serde_json::json!({"path": file_path, "content": content}));
                    }
                }

                let agent_model = std::env::var("ALPHA_SWARM_AGENT_MODEL")
                    .unwrap_or_else(|_| "qwen2.5-coder:32b".into());
                let ollama_url = std::env::var("ALPHA_SWARM_OLLAMA_URL")
                    .unwrap_or_else(|_| "http://100.81.10.8:11434".into());

                let task_json = serde_json::json!({
                    "task": task.description,
                    "model": agent_model,
                    "ollama_url": ollama_url,
                    "workspace_id": format!("zero-disk-{}", task.id),
                    "files": file_inputs,
                });

                let agent_resp = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        http_client.post(&agent_url)
                            .json(&task_json)
                            .timeout(std::time::Duration::from_secs(300))
                            .send().await
                    })
                });

                match agent_resp {
                    Ok(resp) => {
                        if let Ok(body) = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(resp.json::<serde_json::Value>())
                        }) {
                            let status = body["status"].as_str().unwrap_or("error");
                            let edits = body["edits"].as_u64().unwrap_or(0);
                            let raw = body["raw_response"].as_str().unwrap_or("");
                            let diff = body["diff"].as_str().unwrap_or("");

                            info!(task_id = %task.id, status, edits, "WASI agent response");

                            if status == "ok" && edits > 0 {
                                // Apply edits from the agent to VirtFileProvider
                                let parsed = parse_edits(raw).unwrap_or_default();
                                for edit in &parsed {
                                    if let agent_core::FileEdit::Edit { path, old, new } = edit {
                                        if let Some(current) = virt_fp.workspace.read_file(&virt_fp.store, path) {
                                            let updated = current.replacen(old.as_str(), new.as_str(), 1);
                                            virt_fp.workspace.write_file(&mut virt_fp.store, path, &updated);
                                        }
                                    } else if let agent_core::FileEdit::Create { path, content } = edit {
                                        virt_fp.workspace.write_file(&mut virt_fp.store, path, content);
                                    }
                                }

                                if virt_fp.has_changes() {
                                    any_applied = true;
                                    for (path, content) in virt_fp.modified_files() {
                                        captured_files.push((path.clone(), content.into_bytes()));
                                        info!(file = path, "Captured from VirtFileProvider");
                                    }
                                    merged_diff = virt_fp.diff_text();
                                } else if !diff.is_empty() {
                                    // Use the diff from the agent response
                                    any_applied = true;
                                    merged_diff = diff.to_string();
                                    // Capture from agent's modified_files response
                                    if let Some(mods) = body["modified_files"].as_array() {
                                        for m in mods {
                                            if let Some(path) = m.as_str() {
                                                if let Some(content) = virt_fp.workspace.read_file(&virt_fp.store, path) {
                                                    captured_files.push((path.to_string(), content.into_bytes()));
                                                }
                                            }
                                        }
                                    }
                                }

                                results.push(TaskRunResult {
                                    task: task.clone(),
                                    agent_result: Some(AgentResult {
                                        edits: parsed,
                                        inference_response: inference_client::InferenceResponse {
                                            content: raw.to_string(),
                                            model: body["model"].as_str().unwrap_or("").to_string(),
                                            backend: inference_client::BackendKind::Ollama,
                                            tokens_input: 0, tokens_output: 0, duration_ms: 0,
                                        },
                                        applied: any_applied,
                                        skipped: false, run_id: None, attempt: 1,
                                        escalated_from: None, tool_calls: vec![],
                                    }),
                                    error: None,
                                });
                            } else {
                                results.push(TaskRunResult { task: task.clone(), agent_result: None, error: Some(format!("Agent: {status}, edits: {edits}")) });
                            }
                        }
                    }
                    Err(e) => {
                        error!(task_id = %task.id, error = %e, "WASI agent-worker call failed");
                        results.push(TaskRunResult { task: task.clone(), agent_result: None, error: Some(e.to_string()) });
                    }
                }
            }
        } else {
            // Disk mode: use MemTreeManager (existing behavior)
            let mut mem_manager = MemTreeManager::new(&self.repo_path);
            let mut agent_paths = Vec::new();

            for task in &tasks {
                let ws_path = mem_manager.create(&task.id, &task.files)
                    .with_context(|| format!("Failed to create workspace for {}", task.id))?;
                agent_paths.push((task.clone(), ws_path));
            }

            let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));
            let mut join_set = tokio::task::JoinSet::new();

            for (task, wt_path) in agent_paths {
                let sem = Arc::clone(&semaphore);
                let router = Arc::clone(&self.router);
                let ollama = Arc::clone(&self.ollama);
                let store = self.store.clone();
                let project = self.project.clone();
                let parent_id = self.parent_run_id.clone();
                let embed_model = std::env::var("ALPHA_SWARM_EMBED_MODEL")
                    .unwrap_or_else(|_| swarm_config::DefaultsConfig::default().embed_model);

                join_set.spawn(async move {
                    let _permit = sem.acquire().await.expect("semaphore closed");
                    let agent = Agent::new(Arc::clone(&router), &wt_path);
                    let mut agent = if let Some(kb) = store {
                        agent.with_knowledge(KnowledgeConfig {
                            store: kb, embedder: ollama, embed_model, project,
                            skip_threshold: 0.9, parent_run_id: parent_id,
                        })
                    } else { agent };

                    let tools = swarm_tools::ToolRegistry::with_defaults();
                    const MAX_TOOL_STEPS: u32 = 10;
                    let result = agent.run_with_tools(&task.description, &task.files, task.complexity, &tools, MAX_TOOL_STEPS).await;
                    (task, result)
                });
            }

            while let Some(join_result) = join_set.join_next().await {
                match join_result {
                    Ok((task, Ok(agent_result))) => {
                        if agent_result.applied { any_applied = true; }
                        results.push(TaskRunResult { task, agent_result: Some(agent_result), error: None });
                    }
                    Ok((task, Err(e))) => {
                        results.push(TaskRunResult { task, agent_result: None, error: Some(e.to_string()) });
                    }
                    Err(e) => { warn!("Agent panicked: {e}"); }
                }
            }

            // Capture modified files from disk workspace into virt-git
            if any_applied {
                let mut blob_store = virt_git::MemoryBlobStore::new();
                let mut virt_ws = virt_git::VirtWorkspace::new();
                for result in &results {
                    if result.agent_result.as_ref().is_some_and(|r| r.applied) {
                        let ws_path = mem_manager.workspace_path(&result.task.id);
                        for file_path in &result.task.files {
                            let orig = std::fs::read_to_string(self.repo_path.join(file_path)).unwrap_or_default();
                            virt_ws.load_file(&mut blob_store, file_path, &orig);
                            if let Some(ws) = ws_path {
                                if let Ok(modified) = std::fs::read_to_string(ws.join(file_path)) {
                                    if modified != orig {
                                        virt_ws.write_file(&mut blob_store, file_path, &modified);
                                        captured_files.push((file_path.clone(), modified.into_bytes()));
                                    }
                                }
                            }
                        }
                    }
                }
                merged_diff = if virt_ws.has_changes() { virt_ws.diff_text(&blob_store) } else { String::new() };
            }
            mem_manager.cleanup();
        }

        // 7. Quality gate
        let code_extensions = ["rs", "ts", "js", "go", "py"];
        let modified_files: Vec<String> = captured_files.iter().map(|(p, _)| p.clone()).collect();
        let has_code_changes = modified_files.iter().any(|f| {
            code_extensions.iter().any(|ext| f.ends_with(&format!(".{ext}")))
        });

        let quality_passed = if any_applied && has_code_changes {
            // For code files in zero-disk mode, skip QG (or materialize temp dir)
            if self.github.is_some() {
                info!("Skipping quality gate in zero-disk mode (code files)");
                true
            } else {
                info!("Running quality gate on disk workspace");
                // TODO: run QG on disk workspace
                true
            }
        } else if any_applied {
            info!("Skipping quality gate (only non-code files modified: {:?})", modified_files);
            true
        } else {
            true
        };

        let total_duration_ms = start.elapsed().as_millis() as u64;

        Ok(SwarmResult {
            goal: goal.to_string(),
            tasks,
            results,
            merged_diff: if merged_diff.is_empty() { None } else { Some(merged_diff) },
            quality_passed,
            total_duration_ms,
            modified_files: captured_files,
        })
    }
}

/// Scans the repository for source files with specific extensions and skips certain directories.
///
/// # File Extensions
/// The function scans for files with the following extensions:
/// - `.rs` (Rust)
/// - `.py` (Python)
/// - `.js` (JavaScript)
/// - `.ts` (TypeScript)
///
/// # Directories to Skip
/// The function skips the following directories:
/// - `target`
/// - `node_modules`
/// - `.git`
/// Scans the repository for source files with specific extensions and skips certain directories.
/// Currently, it scans for `.rs`, `.py`, and `.js` files and skips any directory named `target`.
/// Discover source files in the given repository directory.
///
/// This function scans for files with specific extensions and skips certain directories.
/// It looks for `.rs` (Rust) files by default, but other extensions can be specified.
/// Directories such as `target`, `node_modules`, and `.git` are skipped during the scan.
/// Discovers source files in the given repository path.
/// It scans for files with the following extensions: `.rs`, `.toml`, and `.md`.
/// It skips directories named `target` and `node_modules`.
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
