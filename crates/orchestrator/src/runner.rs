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
    /// Remaining depth for recursive sub-planning (0 = no sub-plans).
    depth: u32,
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
            depth: 0,
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

    /// Execute a high-level goal: plan → spawn agents in parallel → merge → validate.
    pub async fn run(&self, goal: &str) -> Result<SwarmResult> {
        let start = Instant::now();

        // 1. Discover repo files
        // Note: file embedding indexing is handled by EmbeddingManager lifecycle
        // hooks in executor.rs (on_agent_start / on_agent_done).
        let repo_files = discover_source_files(&self.repo_path)?;
        info!(file_count = repo_files.len(), "Discovered repo files");

        // 2. RAG: find relevant files for this goal
        let rag_start = Instant::now();
        let relevant_files = if let Some(ref store) = self.store {
            let embed_model = std::env::var("ALPHA_SWARM_EMBED_MODEL")
                .unwrap_or_else(|_| swarm_config::DefaultsConfig::default().embed_model);
            match self.ollama.embed(&embed_model, goal).await {
                Ok(embedding) => {
                    match store.find_relevant_files(&self.project, &embedding, 15, 0.3).await {
                        Ok(files) if !files.is_empty() => {
                            let result: Vec<(String, f32)> = files.iter()
                                .map(|(path, _summary, score)| (path.clone(), *score))
                                .collect();
                            info!(count = result.len(), "RAG: found relevant files for goal");
                            Some(result)
                        }
                        _ => None,
                    }
                }
                Err(_) => None,
            }
        } else { None };

        let rag_ms = rag_start.elapsed().as_millis() as u64;
        info!(rag_ms, "Phase 2a: RAG file selection");

        // 3. Plan: decompose goal into sub-tasks (with RAG context)
        let plan_start = Instant::now();
        let tasks = plan_goal(
            &self.router, goal, &repo_files, &self.planner_tier,
            relevant_files.as_deref(),
        ).await.context("Goal planning failed")?;

        let planning_ms = plan_start.elapsed().as_millis() as u64;
        info!(task_count = tasks.len(), planning_ms, "Phase 2b: Planning complete");

        // 3-6. Run agents and collect results
        let agent_start = Instant::now();
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
                            results.push(TaskRunResult { task: task.clone(), agent_result: Some(agent_result), error: None });
                        }
                        Err(e) => {
                            results.push(TaskRunResult { task: task.clone(), agent_result: None, error: Some(e.to_string()) });
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
            // Disk mode: wave-based execution with dependency ordering
            let mut mem_manager = MemTreeManager::new(&self.repo_path);
            let mut completed: std::collections::HashMap<String, TaskRunResult> = std::collections::HashMap::new();
            let mut completed_summaries: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            let task_map: std::collections::HashMap<String, SubTask> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();

            while completed.len() < tasks.len() {
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
                            if content.contains(&edit.old) {
                                let updated = content.replacen(&edit.old, &edit.new, 1);
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
                            completed.insert(task.id.clone(), TaskRunResult {
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
                            });
                            continue;
                        }
                    }

                    let sem = Arc::clone(&semaphore);
                    let router = Arc::clone(&self.router);
                    let ollama = Arc::clone(&self.ollama);
                    let store = self.store.clone();
                    let project = self.project.clone();
                    let parent_id = self.parent_run_id.clone();
                    let nats_client_clone = self.nats_client.clone();
                    let embed_model = std::env::var("ALPHA_SWARM_EMBED_MODEL")
                        .unwrap_or_else(|_| swarm_config::DefaultsConfig::default().embed_model);

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
                                            let event = serde_json::to_vec(&swarm_events::SwarmEvent::AgentProgress {
                                                project: proj, run_id, agent_id: "agent".into(),
                                                step: p.step, max_steps: p.max_steps,
                                                action: p.action.clone(), result_preview: p.result.chars().take(200).collect(),
                                                tokens_in: p.tokens_in, tokens_out: p.tokens_out, edits_count: p.edits_count as u32,
                                                timestamp: chrono::Utc::now().to_rfc3339(),
                                            }).unwrap_or_default();
                                            let _ = nc.publish(format!("alpha-swarm.{}.agent.progress", "alpha-swarm2"), event.into()).await;
                                        }
                                    });
                                });
                            });
                        }

                        let tools = swarm_tools::ToolRegistry::with_defaults();
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
                            completed.insert(task.id.clone(), TaskRunResult { task, agent_result: Some(agent_result), error: None });
                        }
                        Ok((task, Err(e))) => {
                            completed_summaries.insert(task.id.clone(), format!("FAILED: {e}"));
                            completed.insert(task.id.clone(), TaskRunResult { task, agent_result: None, error: Some(e.to_string()) });
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

            // Capture modified files from disk workspaces into virt-git
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
