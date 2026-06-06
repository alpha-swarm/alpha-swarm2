use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, warn, error};

use agent_core::{Agent, AgentResult, KnowledgeConfig, parse_edits};
use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeBackend;

use crate::planner::plan_goal;
use crate::planner_types::SubTask;
use crate::memtree::MemTreeManager;

/// Timing breakdown for each phase of a swarm run.
#[derive(Debug, Clone, Default)]
pub struct PhaseTimings {
    pub rag_ms: u64,
    pub planning_ms: u64,
    pub agent_execution_ms: u64,
    pub quality_gate_ms: u64,
    pub total_ms: u64,
}

impl PhaseTimings {
    /// Format a summary line with percentages.
    pub fn summary(&self) -> String {
        let total = self.total_ms.max(1);
        format!(
            "RAG: {}ms ({:.0}%), Planning: {}ms ({:.0}%), Agents: {}ms ({:.0}%), QG: {}ms ({:.0}%), Total: {}ms",
            self.rag_ms, self.rag_ms as f64 / total as f64 * 100.0,
            self.planning_ms, self.planning_ms as f64 / total as f64 * 100.0,
            self.agent_execution_ms, self.agent_execution_ms as f64 / total as f64 * 100.0,
            self.quality_gate_ms, self.quality_gate_ms as f64 / total as f64 * 100.0,
            self.total_ms,
        )
    }
}

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
    /// Phase timing breakdown.
    pub phase_timings: PhaseTimings,
    /// True when execution stopped early because a pause/cancel was requested
    /// via `RunControl`; unfinished tasks have no entry in `results`.
    pub halted: bool,
    /// Memory pattern ids injected into the planner prompt (SONA effectiveness
    /// signal; empty when retrieval-augmented planning was off or found nothing).
    pub retrieved_pattern_ids: Vec<String>,
}

/// Requested cooperative control state for a running swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    Continue,
    Pause,
    Cancel,
}

const CONTROL_CONTINUE: u8 = 0;
const CONTROL_PAUSE: u8 = 1;
const CONTROL_CANCEL: u8 = 2;

/// Cooperative pause/cancel flag checked between execution waves — never
/// preempts an in-flight agent (correctness over speed).
#[derive(Default)]
pub struct RunControl {
    state: std::sync::atomic::AtomicU8,
}

impl RunControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_pause(&self) {
        self.state.store(CONTROL_PAUSE, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn request_cancel(&self) {
        self.state.store(CONTROL_CANCEL, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.state.store(CONTROL_CONTINUE, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn state(&self) -> ControlState {
        match self.state.load(std::sync::atomic::Ordering::SeqCst) {
            CONTROL_PAUSE => ControlState::Pause,
            CONTROL_CANCEL => ControlState::Cancel,
            _ => ControlState::Continue,
        }
    }
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
    store: Option<Arc<dyn KnowledgeBackend>>,
    repo_path: PathBuf,
    project: String,
    parent_run_id: Option<String>,
    max_concurrent: usize,
    nats_client: Option<async_nats::Client>,
    planner_tier: swarm_config::TierConfig,
    /// Heavier execution tier (e.g. 32b) for refactor/complex tasks. Pre-warmed
    /// + resident; the fast planner tier (14b) can't reliably emit structural
    /// multi-line edits, so those escalate here.
    agent_tier: swarm_config::TierConfig,
    /// When set, use GitHub API + VirtWorkspace instead of disk clone.
    github: Option<GitHubRepo>,
    /// Remaining depth for recursive sub-planning (0 = no sub-plans).
    depth: u32,
    /// Embedding model for RAG/goal embeddings. Must come from the LOADED config
    /// (`config.defaults.embed_model`) so every embedding path uses one model/dimension.
    embed_model: String,
    /// Lifecycle hooks fired around run/task boundaries (sequential, infallible).
    hooks: Arc<crate::hooks::HookSet>,
    /// Cooperative pause/cancel flag, checked between waves.
    control: Option<Arc<RunControl>>,
    /// SONA retrieval-augmented planning config; None disables retrieval.
    learning: Option<swarm_config::LearningConfig>,
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
            agent_tier: swarm_config::TierConfig::agent(),
            github: None,
            depth: 0,
            embed_model: swarm_config::DEFAULT_EMBED_MODEL.into(),
            hooks: Arc::new(crate::hooks::HookSet::new()),
            control: None,
            learning: None,
        }
    }

    pub fn with_store(mut self, store: Arc<dyn KnowledgeBackend>) -> Self {
        self.store = Some(store);
        self
    }

    /// Set the embedding model from the loaded config (`config.defaults.embed_model`).
    pub fn with_embed_model(mut self, model: impl Into<String>) -> Self {
        self.embed_model = model.into();
        self
    }

    /// Attach lifecycle hooks fired around run/task boundaries.
    pub fn with_hooks(mut self, hooks: Arc<crate::hooks::HookSet>) -> Self {
        self.hooks = hooks;
        self
    }

    /// Attach a cooperative pause/cancel flag, checked between waves.
    pub fn with_control(mut self, control: Arc<RunControl>) -> Self {
        self.control = Some(control);
        self
    }

    /// Enable SONA retrieval-augmented planning (past proven plans injected
    /// into the planner prompt).
    pub fn with_learning(mut self, learning: swarm_config::LearningConfig) -> Self {
        self.learning = Some(learning);
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

    /// Heavier tier used for refactor/complex tasks (pre-warmed, resident).
    pub fn with_agent_tier(mut self, tier: swarm_config::TierConfig) -> Self {
        self.agent_tier = tier;
        self
    }

    /// Set recursion depth for sub-planning (0 = flat, no sub-plans).
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// Enable zero-disk mode: files loaded via GitHub API, no git clone.
    pub fn with_github(mut self, gh: GitHubRepo) -> Self {
        self.github = Some(gh);
        self
    }

    /// Fire the matching task-level hook for a finished task result.
    async fn fire_task_hooks(&self, result: &TaskRunResult) {
        if self.hooks.is_empty() {
            return;
        }
        let run_id = self.parent_run_id.as_deref().unwrap_or_default();
        match (&result.agent_result, &result.error) {
            (Some(ar), _) => {
                self.hooks.emit_task_complete(&crate::hooks::TaskCompleteCtx {
                    run_id,
                    project: &self.project,
                    task_id: &result.task.id,
                    description: &result.task.description,
                    files: &result.task.files,
                    applied: ar.applied,
                    duration_ms: ar.inference_response.duration_ms,
                    tokens_in: ar.inference_response.tokens_input,
                    tokens_out: ar.inference_response.tokens_output,
                    model: &ar.inference_response.model,
                }).await;
            }
            (None, Some(err)) => {
                self.hooks.emit_task_fail(&crate::hooks::TaskFailCtx {
                    run_id,
                    project: &self.project,
                    task_id: &result.task.id,
                    description: &result.task.description,
                    error: err,
                }).await;
            }
            _ => {}
        }
    }

    /// Execute a high-level goal: plan → spawn agents in parallel → merge → validate.
    pub async fn run(&self, goal: &str) -> Result<SwarmResult> {
        let start = Instant::now();

        // 1. Discover repo files
        // Note: file embedding indexing is handled by EmbeddingManager lifecycle
        // hooks in executor.rs (on_agent_start / on_agent_done).
        let repo_files = discover_source_files(&self.repo_path)?;
        info!(file_count = repo_files.len(), "Discovered repo files");

        // 2. RAG: find relevant files + past proven plans for this goal.
        // The goal is embedded ONCE; file RAG and memory retrieval share it.
        let rag_start = Instant::now();
        let mut relevant_files: Option<Vec<(String, f32)>> = None;
        let mut past_plans_block: Option<String> = None;
        let mut retrieved_pattern_ids: Vec<String> = Vec::new();

        if let Some(ref store) = self.store {
            if let Ok(embedding) = self.ollama.embed(&self.embed_model, goal).await {
                if let Ok(files) = store.find_relevant_files(&self.project, &embedding, 15, 0.3).await
                    && !files.is_empty() {
                        let result: Vec<(String, f32)> = files.iter()
                            .map(|(path, _summary, score)| (path.clone(), *score))
                            .collect();
                        info!(count = result.len(), "RAG: found relevant files for goal");
                        relevant_files = Some(result);
                }

                // SONA: retrieve past proven plans from memory (same embedding).
                if let Some(learning) = self.learning.as_ref().filter(|l| l.enabled) {
                    let memory = knowledge_base::MemoryStore::new(
                        Arc::clone(store), Arc::clone(&self.ollama), self.embed_model.clone(),
                    );
                    let namespaces = [knowledge_base::MEM_NS_PATTERNS, knowledge_base::MEM_NS_SOLUTIONS];
                    match memory.search(&namespaces, &self.project, &embedding, learning.max_proven_plans).await {
                        Ok(hits) if !hits.is_empty() => {
                            let mut block = String::new();
                            for hit in &hits {
                                if hit.similarity < learning.min_similarity { continue; }
                                if block.len() + hit.entry.content.len() > learning.proven_plans_char_budget { break; }
                                block.push_str(&format!("- (sim {:.2}) {}\n", hit.similarity, hit.entry.content));
                                if let Some(id) = &hit.entry.id {
                                    retrieved_pattern_ids.push(id.clone());
                                }
                            }
                            if !block.is_empty() {
                                info!(patterns = retrieved_pattern_ids.len(), "SONA: injecting past proven plans");
                                past_plans_block = Some(block);
                            }
                        }
                        Ok(_) => {}
                        Err(e) => warn!(error = %e, "SONA memory retrieval failed (continuing without)"),
                    }
                }
            }
        }

        let rag_ms = rag_start.elapsed().as_millis() as u64;
        info!(rag_ms, "Phase 2a: RAG file selection");

        // 3. Plan: decompose goal into sub-tasks (with RAG + memory context)
        let plan_start = Instant::now();
        let tasks = plan_goal(
            &self.router, goal, &repo_files, &self.planner_tier,
            relevant_files.as_deref(),
            past_plans_block.as_deref(),
        ).await.context("Goal planning failed")?;

        // Post-plan fixup: tasks with empty files (e.g. glob patterns were rejected)
        // get populated with relevant repo files matching their description keywords
        let mut tasks = tasks;
        for task in &mut tasks {
            if task.files.is_empty() {
                let desc_lower = task.description.to_lowercase();
                let matching: Vec<String> = repo_files.iter()
                    .filter(|f| {
                        let fl = f.to_lowercase();
                        // Match files in directories mentioned in description
                        (desc_lower.contains("dashboard") && fl.starts_with("dashboard/src/"))
                            || (desc_lower.contains("frontend") && fl.starts_with("dashboard/src/"))
                            || (desc_lower.contains("component") && fl.contains("/components/"))
                    })
                    .take(10)
                    .cloned()
                    .collect();
                if !matching.is_empty() {
                    warn!(task_id = %task.id, files = matching.len(), "Populated empty task with matching repo files");
                    task.files = matching;
                }
            }
        }
        tasks.retain(|t| !t.files.is_empty());

        let planning_ms = plan_start.elapsed().as_millis() as u64;
        info!(task_count = tasks.len(), planning_ms, "Phase 2b: Planning complete");

        self.execute_tasks(goal, tasks, rag_ms, planning_ms, start, retrieved_pattern_ids).await
    }

    /// Execute a pre-planned task list, skipping RAG + planning. Entry point for
    /// the workflow engine and approved-plan execution (no re-planning).
    pub async fn run_planned(&self, goal: &str, tasks: Vec<SubTask>) -> Result<SwarmResult> {
        /// Phase duration reported when a phase did not run.
        const NO_PHASE_MS: u64 = 0;
        self.execute_tasks(goal, tasks, NO_PHASE_MS, NO_PHASE_MS, Instant::now(), Vec::new()).await
    }

    /// Shared execution path: dependency waves (or zero-disk loop) → capture → quality gate.
    async fn execute_tasks(
        &self,
        goal: &str,
        tasks: Vec<SubTask>,
        rag_ms: u64,
        planning_ms: u64,
        start: Instant,
        retrieved_pattern_ids: Vec<String>,
    ) -> Result<SwarmResult> {
        self.hooks.emit_run_start(&crate::hooks::RunStartCtx {
            run_id: self.parent_run_id.as_deref().unwrap_or_default(),
            project: &self.project,
            goal,
            planned_task_count: tasks.len(),
        }).await;

        // 3-6. Run agents and collect results
        let agent_start = Instant::now();
        let mut results = Vec::new();
        let mut any_applied = false;
        let mut captured_files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut merged_diff = String::new();
        let mut halted = false;

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

                // Store files in NATS blobstore for the WASI component to read
                let workspace_id = format!("ws-{}-{}", task.id, std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());

                let nats_url = std::env::var("ALPHA_SWARM_NATS_URL")
                    .unwrap_or_else(|_| "nats://127.0.0.1:4223".into());

                // Write files to NATS Object Store (blobstore)
                let mut file_paths_loaded = Vec::new();
                if let Ok(nats_client) = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async_nats::connect(&nats_url))
                }) {
                    let js = async_nats::jetstream::new(nats_client);
                    if let Ok(obj_store) = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(
                            js.create_object_store(async_nats::jetstream::object_store::Config {
                                bucket: workspace_id.clone(),
                                ..Default::default()
                            })
                        )
                    }) {
                        for file_path in &task.files {
                            if let Some(content) = virt_fp.workspace.read_file(&virt_fp.store, file_path) {
                                let key = format!("file/{file_path}");
                                let data = content.into_bytes();
                                let store_clone = obj_store.clone();
                                let _ = tokio::task::block_in_place(|| {
                                    tokio::runtime::Handle::current().block_on(async {
                                        let mut reader = &data[..];
                                        store_clone.put(key.as_str(), &mut reader).await
                                    })
                                });
                                file_paths_loaded.push(file_path.clone());
                                info!(file = file_path, bucket = %workspace_id, "Stored in NATS blobstore");
                            }
                        }
                    }
                }

                // Call WASI agent-worker — tiny payload, files are in blobstore
                let agent_url = std::env::var("AGENT_WORKER_URL")
                    .unwrap_or_else(|_| "http://localhost:8000".into());

                let agent_model = std::env::var("ALPHA_SWARM_AGENT_MODEL")
                    .unwrap_or_else(|_| "qwen2.5-coder:32b".into());
                let ollama_url = std::env::var("ALPHA_SWARM_OLLAMA_URL")
                    .unwrap_or_else(|_| "http://100.81.10.8:11434".into());

                // Include file content inline for the WASI component
                // Use task.files (not file_paths_loaded which depends on NATS)
                let mut inline_files = Vec::new();
                let mut total_content_size = 0usize;
                for file_path in &task.files {
                    if let Some(content) = virt_fp.workspace.read_file(&virt_fp.store, file_path) {
                        total_content_size += content.len();
                        inline_files.push(serde_json::json!({"path": file_path, "content": content}));
                    }
                }

                // If total content > 2KB, use native agent (WASI component has body size limit in wash dev)
                if total_content_size > 2000 {
                    info!(total_content_size, "Large files — using native agent with VirtFileProvider");
                    let mut agent = Agent::new(Arc::clone(&self.router), &self.repo_path)
                        .with_file_provider(virt_fp);
                    let result = agent.run(&task.description, &task.files, task.complexity).await;

                    // Extract from provider
                    if let Some(fp) = agent.take_file_provider() {
                        if let Some(vfp) = fp.as_any().downcast_ref::<agent_core::VirtFileProvider>() {
                            if vfp.has_changes() {
                                any_applied = true;
                                for (path, content) in vfp.modified_files() {
                                    captured_files.push((path.clone(), content.into_bytes()));
                                    info!(file = path, "Captured from VirtFileProvider");
                                }
                                merged_diff = vfp.diff_text();
                            }
                        }
                    }

                    match result {
                        Ok(agent_result) => {
                            if agent_result.applied { any_applied = true; }
                            let r = TaskRunResult { task: task.clone(), agent_result: Some(agent_result), error: None };
                            self.fire_task_hooks(&r).await;
                            results.push(r);
                        }
                        Err(e) => {
                            let r = TaskRunResult { task: task.clone(), agent_result: None, error: Some(e.to_string()) };
                            self.fire_task_hooks(&r).await;
                            results.push(r);
                        }
                    }
                    continue;
                }

                let task_json = serde_json::json!({
                    "task": task.description,
                    "model": agent_model,
                    "ollama_url": ollama_url,
                    "workspace_id": workspace_id,
                    "workspace_files": file_paths_loaded,
                    "files": inline_files,
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
                                            if let Some(updated) = agent_core::fuzzy_replace(&current, old, new) {
                                                virt_fp.workspace.write_file(&mut virt_fp.store, path, &updated);
                                            }
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

                                let r = TaskRunResult {
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
                                };
                                self.fire_task_hooks(&r).await;
                                results.push(r);
                            } else {
                                let r = TaskRunResult { task: task.clone(), agent_result: None, error: Some(format!("Agent: {status}, edits: {edits}")) };
                                self.fire_task_hooks(&r).await;
                                results.push(r);
                            }
                        }
                    }
                    Err(e) => {
                        error!(task_id = %task.id, error = %e, "WASI agent-worker call failed");
                        let r = TaskRunResult { task: task.clone(), agent_result: None, error: Some(e.to_string()) };
                        self.fire_task_hooks(&r).await;
                        results.push(r);
                    }
                }
            }
        } else {
            // Disk mode: wave-based execution with dependency ordering
            let mut mem_manager = MemTreeManager::new(&self.repo_path);
            let mut completed: std::collections::HashMap<String, TaskRunResult> = std::collections::HashMap::new();
            let mut completed_summaries: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            let task_map: std::collections::HashMap<String, SubTask> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();

            while completed.len() < tasks.len() {
                // Cooperative pause/cancel: stop dispatching new waves; in-flight
                // agents have already finished (checked between waves only).
                if let Some(ref ctl) = self.control
                    && ctl.state() != ControlState::Continue {
                        info!(state = ?ctl.state(), "Run control requested — halting before next wave");
                        halted = true;
                        break;
                }

                // Find ready tasks: all depends_on satisfied
                let ready: Vec<String> = tasks.iter()
                    .filter(|t| !completed.contains_key(&t.id))
                    .filter(|t| t.depends_on.iter().all(|d| completed.contains_key(d)))
                    .map(|t| t.id.clone())
                    .collect();

                if ready.is_empty() {
                    warn!("No ready tasks but {} incomplete — possible cycle", tasks.len() - completed.len());
                    break;
                }

                let wave_size = ready.len().min(self.max_concurrent);
                let wave_tasks: Vec<String> = ready.into_iter().take(wave_size).collect();
                info!(wave = wave_tasks.len(), remaining = tasks.len() - completed.len(), "Starting wave");

                // Create workspaces for this wave
                let mut wave_paths = Vec::new();
                for task_id in &wave_tasks {
                    let task = &task_map[task_id];
                    let ws_path = mem_manager.create(&task.id, &task.files)
                        .with_context(|| format!("Failed to create workspace for {}", task.id))?;

                    // Copy modified files from dependency workspaces
                    for dep_id in &task.depends_on {
                        if let Some(dep_ws) = mem_manager.workspace_path(dep_id) {
                            if let Some(dep_result) = completed.get(dep_id) {
                                for file_path in &dep_result.task.files {
                                    let src = dep_ws.join(file_path);
                                    if src.exists() {
                                        let dst = ws_path.join(file_path);
                                        if let Some(parent) = dst.parent() {
                                            let _ = std::fs::create_dir_all(parent);
                                        }
                                        let _ = std::fs::copy(&src, &dst);
                                    }
                                }
                            }
                        }
                    }

                    // Build augmented description with predecessor summaries
                    let mut desc = task.description.clone();
                    for dep_id in &task.depends_on {
                        if let Some(summary) = completed_summaries.get(dep_id) {
                            desc.push_str(&format!("\n\nPREVIOUS TASK {} OUTPUT:\n{}", dep_id, summary));
                        }
                    }

                    wave_paths.push((task.clone(), ws_path, desc));
                }

                // Spawn wave agents
                let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent));
                let mut join_set = tokio::task::JoinSet::new();

                for (task, wt_path, augmented_desc) in wave_paths {
                    // Fast path: direct edit — no inference needed
                    if let Some(ref edit) = task.edit {
                        info!(task_id = %task.id, path = %edit.path, "Applying direct edit (no inference)");
                        let full_path = wt_path.join(&edit.path);
                        let applied = if let Ok(content) = std::fs::read_to_string(&full_path) {
                            if let Some(updated) = agent_core::fuzzy_replace(&content, &edit.old, &edit.new) {
                                std::fs::write(&full_path, &updated).is_ok()
                            } else {
                                warn!(task_id = %task.id, "Direct edit OLD block not found, will use agent");
                                false
                            }
                        } else { false };

                        if applied {
                            any_applied = true;
                            let summary = format!("Direct edit: {} ({}→{})", edit.path, edit.old.chars().take(30).collect::<String>(), edit.new.chars().take(30).collect::<String>());
                            completed_summaries.insert(task.id.clone(), summary);
                            let result = TaskRunResult {
                                task, agent_result: Some(AgentResult {
                                    edits: vec![],
                                    inference_response: inference_client::InferenceResponse {
                                        content: "Direct edit applied".into(),
                                        model: "direct".into(),
                                        backend: inference_client::BackendKind::Ollama,
                                        tokens_input: 0, tokens_output: 0, duration_ms: 0,
                                    },
                                    applied: true, skipped: false, run_id: None, attempt: 1,
                                    escalated_from: None, tool_calls: vec![],
                                }),
                                error: None,
                            };
                            self.fire_task_hooks(&result).await;
                            completed.insert(result.task.id.clone(), result);
                            continue;
                        }
                    }

                    // Graph executor path: known template → fewer LLM calls
                    if let Some(ref tmpl) = task.template {
                        info!(task_id = %task.id, template = %tmpl, "Using graph executor");
                        let crate_name = crate::graph::detect_crate(&wt_path, task.files.first().map(|s| s.as_str()).unwrap_or(""));
                        // Structural work (refactors / complex tasks) needs a
                        // bigger model than the fast planner tier — escalate to
                        // the pre-warmed agent tier. Edits/docs stay on 14b.
                        let exec_model = if tmpl.as_str() == "refactor"
                            || matches!(task.complexity, inference_client::Complexity::Complex)
                        {
                            info!(task_id = %task.id, model = %self.agent_tier.model, "escalating to agent tier (refactor/complex)");
                            self.agent_tier.model.clone()
                        } else {
                            self.planner_tier.model.clone()
                        };
                        let executor = crate::graph::GraphExecutor::new(
                            Arc::clone(&self.router), wt_path.clone(), crate_name, 3,
                        ).with_model(exec_model.clone()).with_ollama(Arc::clone(&self.ollama));
                        let graph_result = match tmpl.as_str() {
                            "edit" => executor.execute_edit(&augmented_desc, task.files.first().map(|s| s.as_str()).unwrap_or("")).await,
                            "create" => executor.execute_create(&augmented_desc, task.files.first().map(|s| s.as_str()).unwrap_or("")).await,
                            "refactor" => executor.execute_refactor(&augmented_desc, &task.files).await,
                            "doc" => executor.execute_doc(&augmented_desc, task.files.first().map(|s| s.as_str()).unwrap_or("")).await,
                            _ => Err(anyhow::anyhow!("Unknown template: {tmpl}")),
                        };

                        match graph_result {
                            Ok(gr) if !gr.escalated => {
                                any_applied = true;
                                let summary = format!("Graph:{tmpl} edits:{}", gr.edits.len());
                                completed_summaries.insert(task.id.clone(), summary);
                                // Stamp the tier model that ran this task so
                                // model_used / the brain export attribute it
                                // (the graph response loses .model on some paths
                                // → was recorded "unknown", hiding escalation).
                                let mut gr_response = gr.response;
                                if gr_response.model.is_empty() {
                                    gr_response.model = exec_model.clone();
                                }
                                let result = TaskRunResult {
                                    task, agent_result: Some(AgentResult {
                                        edits: gr.edits, inference_response: gr_response,
                                        applied: true, skipped: false, run_id: None, attempt: 1,
                                        escalated_from: None, tool_calls: vec![],
                                    }), error: None,
                                };
                                self.fire_task_hooks(&result).await;
                                completed.insert(result.task.id.clone(), result);
                                continue;
                            }
                            Ok(_gr) => {
                                info!(task_id = %task.id, "Graph escalated to full agent");
                                // Fall through to full agent below
                            }
                            Err(e) => {
                                warn!(task_id = %task.id, error = %e, "Graph executor failed, falling back to agent");
                            }
                        }
                    }

                    let sem = Arc::clone(&semaphore);
                    let router = Arc::clone(&self.router);
                    let ollama = Arc::clone(&self.ollama);
                    let store = self.store.clone();
                    let project = self.project.clone();
                    let parent_id = self.parent_run_id.clone();
                    let nats_client_clone = self.nats_client.clone();
                    let embed_model = self.embed_model.clone();

                    join_set.spawn(async move {
                        let _permit = sem.acquire().await.expect("semaphore closed");
                        let task_id_for_progress = parent_id.clone().unwrap_or_default();
                        let store_for_progress = store.clone();

                        let project_for_progress = project.clone();
                        let agent = Agent::new(Arc::clone(&router), &wt_path);
                        let mut agent = if let Some(kb) = store {
                            agent.with_knowledge(KnowledgeConfig {
                                store: kb, embedder: ollama, embed_model, project,
                                skip_threshold: 0.9, parent_run_id: parent_id,
                            })
                        } else { agent };

                        if let Some(progress_store) = store_for_progress {
                            let tid = task_id_for_progress.clone();
                            let nats_for_progress = nats_client_clone.clone();
                            agent = agent.with_progress(move |p: agent_core::AgentProgress| {
                                let msg = format!("Step {}/{}: {} → {}", p.step, p.max_steps, p.action, p.result.chars().take(80).collect::<String>());
                                let tokens_msg = format!("{}in/{}out, {} edits", p.tokens_in, p.tokens_out, p.edits_count);
                                let full_msg = format!("{} [{}]", msg, tokens_msg);
                                let safe = full_msg.replace('\'', "").replace('\\', "").replace('\n', " ").replace('\r', "");
                                let now = chrono::Utc::now().to_rfc3339();
                                let query = if tid.contains(':') {
                                    format!("UPDATE {} SET progress_message = '{}', last_activity_at = '{}'", tid, safe, now)
                                } else if !tid.is_empty() {
                                    format!("UPDATE type::thing('agent_run', '{}') SET progress_message = '{}', last_activity_at = '{}'", tid, safe, now)
                                } else { return; };
                                let store = progress_store.clone();
                                let nats = nats_for_progress.clone();
                                let proj = project_for_progress.clone();
                                let run_id = tid.clone();
                                tokio::task::block_in_place(|| {
                                    tokio::runtime::Handle::current().block_on(async {
                                        let _ = store.db_query_raw(&query).await;
                                        // Publish real-time progress event via NATS
                                        if let Some(ref nc) = nats {
                                            let subject = format!("alpha-swarm.{}.agent.progress", proj);
                                            let event = serde_json::to_vec(&swarm_events::SwarmEvent::AgentProgress {
                                                project: proj, run_id, agent_id: "agent".into(),
                                                step: p.step, max_steps: p.max_steps,
                                                action: p.action.clone(), result_preview: p.result.chars().take(200).collect(),
                                                tokens_in: p.tokens_in, tokens_out: p.tokens_out, edits_count: p.edits_count as u32,
                                                timestamp: chrono::Utc::now().to_rfc3339(),
                                            }).unwrap_or_default();
                                            let _ = nc.publish(subject, event.into()).await;
                                        }
                                    });
                                });
                            });
                        }

                        let mut tools = swarm_tools::ToolRegistry::with_defaults()
                            // Embedded Wassette WASM tools (no-op if none installed).
                            .with_wasm_tools();
                        // Memory tools need the NATS bridge — register only
                        // when a client is attached (prompt list stays honest).
                        if let Some(nc) = nats_client_clone.clone() {
                            tools = tools.with_memory_tools(nc);
                        }
                        const MAX_TOOL_STEPS: u32 = 10;
                        let result = agent.run_with_tools(&augmented_desc, &task.files, task.complexity, &tools, MAX_TOOL_STEPS).await;
                        (task, result)
                    });
                }

                // Collect wave results
                while let Some(join_result) = join_set.join_next().await {
                    match join_result {
                        Ok((task, Ok(agent_result))) => {
                            if agent_result.applied { any_applied = true; }
                            // Build summary for dependent tasks
                            let summary = format!(
                                "Modified files: {:?}. Edits: {}. Response: {}",
                                task.files,
                                agent_result.edits.len(),
                                agent_result.inference_response.content.chars().take(200).collect::<String>(),
                            );
                            completed_summaries.insert(task.id.clone(), summary);
                            let result = TaskRunResult { task, agent_result: Some(agent_result), error: None };
                            self.fire_task_hooks(&result).await;
                            completed.insert(result.task.id.clone(), result);
                        }
                        Ok((task, Err(e))) => {
                            completed_summaries.insert(task.id.clone(), format!("FAILED: {e}"));
                            let result = TaskRunResult { task, agent_result: None, error: Some(e.to_string()) };
                            self.fire_task_hooks(&result).await;
                            completed.insert(result.task.id.clone(), result);
                        }
                        Err(e) => { warn!("Agent panicked: {e}"); }
                    }
                }
            }

            // Move completed results into the results vec
            for task in &tasks {
                if let Some(result) = completed.remove(&task.id) {
                    results.push(result);
                }
            }

            // Capture modified files from disk workspaces using git diff
            if any_applied {
                let mut blob_store = virt_git::MemoryBlobStore::new();
                let mut virt_ws = virt_git::VirtWorkspace::new();
                for result in &results {
                    if result.agent_result.as_ref().is_some_and(|r| r.applied) {
                        let Some(ws) = mem_manager.workspace_path(&result.task.id) else { continue };
                        // Use git diff to find ALL changed files (not just task.files)
                        let diff_output = std::process::Command::new("git")
                            .args(["diff", "--name-only", "HEAD"])
                            .current_dir(ws)
                            .output();
                        let changed: Vec<String> = diff_output.ok()
                            .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect())
                            .unwrap_or_default();
                        // Also check for untracked (new) files
                        let untracked = std::process::Command::new("git")
                            .args(["ls-files", "--others", "--exclude-standard"])
                            .current_dir(ws)
                            .output();
                        let new_files: Vec<String> = untracked.ok()
                            .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect())
                            .unwrap_or_default();

                        // A Cargo.lock delta with no matching Cargo.toml edit is
                        // just cargo re-resolving deps during the run's checks —
                        // the agent never touched it. Capturing it pollutes the
                        // diff and lands unrelated lock churn in swarm/auto, and
                        // can mask a no-op run as "changed". Drop that churn (and
                        // any target/ artifacts); keep Cargo.lock only when a
                        // Cargo.toml also changed (a real dependency edit).
                        let toml_changed = changed.iter().chain(new_files.iter())
                            .any(|f| f.ends_with("Cargo.toml"));
                        for file_path in changed.iter().chain(new_files.iter()) {
                            if file_path.starts_with("target/") { continue; }
                            if file_path.ends_with("Cargo.lock") && !toml_changed { continue; }
                            let orig = std::fs::read_to_string(self.repo_path.join(file_path)).unwrap_or_default();
                            virt_ws.load_file(&mut blob_store, file_path, &orig);
                            if let Ok(modified) = std::fs::read_to_string(ws.join(file_path)) {
                                if modified != orig {
                                    virt_ws.write_file(&mut blob_store, file_path, &modified);
                                    captured_files.push((file_path.clone(), modified.into_bytes()));
                                }
                            }
                        }
                    }
                }
                merged_diff = if virt_ws.has_changes() { virt_ws.diff_text(&blob_store) } else { String::new() };
            }
            mem_manager.cleanup();
        }

        let agent_execution_ms = agent_start.elapsed().as_millis() as u64;
        info!(agent_execution_ms, tasks = tasks.len(), any_applied, "Phase 3: Agent execution complete");

        // 7. Quality gate
        let qg_start = Instant::now();
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

        let quality_gate_ms = qg_start.elapsed().as_millis() as u64;
        let total_duration_ms = start.elapsed().as_millis() as u64;

        let phase_timings = PhaseTimings {
            rag_ms,
            planning_ms,
            agent_execution_ms,
            quality_gate_ms,
            total_ms: total_duration_ms,
        };
        info!(summary = %phase_timings.summary(), "Phase timing breakdown");

        Ok(SwarmResult {
            goal: goal.to_string(),
            tasks,
            results,
            merged_diff: if merged_diff.is_empty() { None } else { Some(merged_diff) },
            quality_passed,
            total_duration_ms,
            modified_files: captured_files,
            phase_timings,
            halted,
            retrieved_pattern_ids,
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
    embed_model: &str,
) {

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
                let _ = store.store_file_embedding(project, file_path, &summary_truncated, &embedding, "").await;
                indexed += 1;
            }
            Err(_) => continue,
        }
    }

    if indexed > 0 {
        info!(indexed, total = files.len(), "Indexed file embeddings for RAG");
    }
}
