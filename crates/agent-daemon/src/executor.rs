use std::sync::Arc;

use tracing::{info, warn, error};

use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::{AgentRun, AttemptRecord, KnowledgeBackend, RunStatus};
use swarm_config::SwarmConfig;

/// Max chars for attempt preview fields
const ATTEMPT_PREVIEW_CHARS: usize = 500;
use swarm_events::{EventPublisher, SwarmEvent};

fn format_update(task_id: &str, set_clause: &str) -> String {
    if task_id.contains(':') {
        format!("UPDATE {} {}", task_id, set_clause)
    } else {
        format!("UPDATE type::thing('agent_run', '{}') {}", task_id, set_clause)
    }
}

fn format_update_where(task_id: &str, set_clause: &str, where_clause: &str) -> String {
    if task_id.contains(':') {
        format!("UPDATE {} {} WHERE {}", task_id, set_clause, where_clause)
    } else {
        format!("UPDATE type::thing('agent_run', '{}') {} WHERE {}", task_id, set_clause, where_clause)
    }
}

/// Land a passed run's changes onto a `swarm/auto` branch in the source repo,
/// via a throwaway git worktree so the live checkout is never touched. Commits
/// accumulate on `swarm/auto` for human review/merge. Local-path repos only
/// (remote URLs are left to PR mode). Best-effort; logs and moves on.
fn land_to_branch(repo_url: &str, run_id: &str, goal: &str, files: &[(String, Vec<u8>)]) {
    if files.is_empty() || repo_url.contains("://") {
        return;
    }
    let repo = std::path::Path::new(repo_url);
    if !repo.join(".git").exists() {
        return;
    }
    let safe_id = run_id.replace([':', '/'], "_");
    let land_dir = std::path::PathBuf::from(format!("/tmp/alpha-swarm/land/{safe_id}"));
    let ld = land_dir.to_string_lossy().to_string();
    let _ = std::fs::remove_dir_all(&land_dir);
    let git = |dir: &std::path::Path, args: &[&str]| {
        std::process::Command::new("git").args(args).current_dir(dir).output()
            .map(|o| o.status.success()).unwrap_or(false)
    };
    let _ = git(repo, &["worktree", "prune"]);
    // Reuse the existing swarm/auto branch (accumulate), else create from HEAD.
    if !git(repo, &["worktree", "add", "--force", &ld, "swarm/auto"])
        && !git(repo, &["worktree", "add", "--force", "-b", "swarm/auto", &ld, "HEAD"])
    {
        warn!(run_id, "land: could not create swarm/auto worktree");
        return;
    }
    for (path, content) in files {
        let full = land_dir.join(path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&full, content);
    }
    let _ = git(&land_dir, &["add", "-A"]);
    let msg = format!("swarm: {} [{}]", goal.chars().take(60).collect::<String>(), run_id);
    let committed = git(&land_dir, &[
        "-c", "user.email=swarm@local", "-c", "user.name=alpha-swarm",
        "commit", "-m", &msg, "--no-verify",
    ]);
    let _ = git(repo, &["worktree", "remove", "--force", &ld]);
    if committed {
        info!(run_id, branch = "swarm/auto", files = files.len(), "landed changes (review + merge swarm/auto)");
    }
}

fn discover_source_files(repo_path: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    let extensions = ["rs", "ts", "js", "go", "py", "md", "toml", "json", "yaml", "yml"];
    fn walk(dir: &std::path::Path, base: &std::path::Path, ext: &[&str], out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with('.') || name == "target" || name == "node_modules" || name == "dist" { continue; }
                walk(&path, base, ext, out);
            } else if let Some(e) = path.extension().and_then(|e| e.to_str())
                && ext.contains(&e)
                && let Ok(rel) = path.strip_prefix(base) {
                    out.push(rel.to_string_lossy().to_string());
            }
        }
    }
    walk(repo_path, repo_path, &extensions, &mut files);
    files.sort();
    files
}

/// Dispatch a task based on its status: planning, approved, or pending (legacy).
#[allow(clippy::too_many_arguments)]
pub async fn handle_task(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    publisher: Option<Arc<EventPublisher>>,
    engine: Arc<swarm_workflow::WorkflowEngine>,
    task_id: &str,
    project: &str,
    goal: &str,
    status: &str,
) {
    match status {
        "planning" => handle_planning(config, router, ollama, store, task_id, project, goal).await,
        "approved" => handle_approved(config, router, ollama, store, publisher, engine, task_id, project, goal).await,
        _ => handle_execute(config, router, ollama, store, publisher, engine, task_id, project, goal).await,
    }
}

/// Convert a persisted `PlannedTask` back into the runner's `SubTask`.
fn planned_to_subtask(t: &knowledge_base::PlannedTask) -> swarm_orchestrator::SubTask {
    let complexity = match t.complexity.to_lowercase().as_str() {
        "medium" => inference_client::Complexity::Medium,
        "complex" => inference_client::Complexity::Complex,
        _ => inference_client::Complexity::Simple,
    };
    swarm_orchestrator::SubTask {
        id: t.id.clone(),
        description: t.description.clone(),
        files: t.files.clone(),
        complexity,
        depends_on: t.depends_on.clone(),
        edit: t.edit.as_ref().map(|e| swarm_orchestrator::planner_types::DirectEdit {
            path: e.path.clone(),
            old: e.old.clone(),
            new: e.new.clone(),
        }),
        template: t.template.clone(),
    }
}

/// Planning-only: generate plan, store it, set status to 'planned', STOP.
async fn handle_planning(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    task_id: &str,
    project: &str,
    goal: &str,
) {
    info!(task_id, project, goal, "Planning only (no execution)");

    // Claim
    let now = chrono::Utc::now().to_rfc3339();
    let claim = format_update(task_id, &format!("SET status = 'running', agent_id = 'planner', last_activity_at = '{}', progress_message = 'Planning goal decomposition...'", now));
    if store.db_query_raw(&claim).await.is_err() { return; }

    // Look up repo
    let repo_url = match store.get_project_repo(project).await {
        Ok(Some(url)) => url,
        _ => {
            let _ = store.db_query_raw(&format_update(task_id, "SET status = 'failed', error_message = 'No repo URL for project'")).await;
            return;
        }
    };

    let git = crate::provider_client::GitProviderClient::new(&config.nats.url).await;
    let repo_path = match git.ensure_repo(project, &repo_url).await {
        Ok(p) => std::path::PathBuf::from(p),
        Err(e) => {
            let _ = store.db_query_raw(&format_update(task_id, &format!("SET status = 'failed', error_message = 'Git clone failed: {}'", e.replace('\'', "")))).await;
            return;
        }
    };

    // Discover files and plan
    let repo_files = discover_source_files(&repo_path);
    let start = std::time::Instant::now();

    // Check for previous plans and feedback (for iterative re-planning)
    let previous_plans = store.get_plans(task_id).await.unwrap_or_default();
    let version = previous_plans.last().map(|p| p.version + 1).unwrap_or(1);

    // Build planning prompt with full conversation history
    let plan_goal = if previous_plans.is_empty() {
        goal.to_string()
    } else {
        let mut context = format!("{}\n\n", goal);
        for plan in &previous_plans {
            // Show previous plan
            let task_list: Vec<String> = plan.sub_tasks.iter()
                .map(|t| format!("  - {}: {} (files: {:?}, {})", t.id, t.description, t.files, t.complexity))
                .collect();
            context.push_str(&format!("PREVIOUS PLAN (v{}):\n{}\n\n", plan.version, task_list.join("\n")));

            // Show user feedback if any
            if let Some(fb) = &plan.user_feedback {
                context.push_str(&format!("USER FEEDBACK:\n{}\n\n", fb));
            }
        }
        context.push_str("Generate an improved plan addressing all feedback. Output ONLY the JSON array.");
        context
    };

    // Update progress with iteration info
    let progress = if version > 1 {
        format!("Re-planning (v{}) with user feedback...", version)
    } else {
        "Planning goal decomposition...".to_string()
    };
    let _ = store.db_query_raw(&format_update(task_id, &format!("SET progress_message = '{}'", progress.replace('\'', "")))).await;

    // SONA: feedback capture + retrieval of past proven plans.
    let memory = config.learning.enabled.then(|| knowledge_base::MemoryStore::new(
        Arc::clone(&store), Arc::clone(&ollama), config.defaults.embed_model.clone(),
    ));

    // User feedback on a previous plan version is high-signal — persist it
    // into the feedback namespace keyed by goal shape.
    if let Some(memory) = &memory
        && let Some(fb) = previous_plans.last().and_then(|p| p.user_feedback.clone()) {
            let now = chrono::Utc::now().to_rfc3339();
            let entry = knowledge_base::MemoryEntry {
                id: None,
                namespace: knowledge_base::MEM_NS_FEEDBACK.into(),
                key: task_id.to_string(),
                content: format!("GOAL: {goal}\nFEEDBACK: {fb}"),
                embedding: Vec::new(), // embedded from content on store
                metadata: serde_json::json!({ "run_id": task_id }),
                project: project.to_string(),
                created_at: now.clone(),
                last_used_at: now,
                use_count: 0,
                ttl_secs: None,
            };
            if let Err(e) = memory.store(entry).await {
                warn!(task_id, error = %e, "feedback memory store failed");
            }
    }

    // Retrieve past proven plans for similar goals and inject as guidance.
    // Weighted by closed-loop effectiveness — proven patterns rank higher.
    let (past_plans_block, retrieved_pattern_ids) = if let Some(memory) = &memory {
        let namespaces = [knowledge_base::MEM_NS_PATTERNS, knowledge_base::MEM_NS_SOLUTIONS];
        match memory.search_text_weighted(&namespaces, project, goal, config.learning.max_proven_plans).await {
            Ok(hits) if !hits.is_empty() => {
                let mut block = String::new();
                let mut ids = Vec::new();
                for hit in &hits {
                    if hit.similarity < config.learning.min_similarity { continue; }
                    if block.len() + hit.entry.content.len() > config.learning.proven_plans_char_budget { break; }
                    block.push_str(&format!("- (sim {:.2}) {}\n", hit.similarity, hit.entry.content));
                    if let Some(id) = &hit.entry.id { ids.push(id.clone()); }
                }
                if block.is_empty() { (None, Vec::new()) } else {
                    info!(task_id, patterns = ids.len(), "SONA: injecting past proven plans into planner");
                    (Some(block), ids)
                }
            }
            _ => (None, Vec::new()),
        }
    } else {
        (None, Vec::new())
    };

    match swarm_orchestrator::plan_goal(&router, &plan_goal, &repo_files, &config.tiers.orchestrator, None, past_plans_block.as_deref()).await {
        Ok(tasks) => {
            let duration_ms = start.elapsed().as_millis() as u64;

            // Convert SubTasks to PlannedTasks (lossless: DAG edges, template, and
            // direct-edit payload are persisted so approved plans can be executed
            // without re-planning).
            let sub_tasks: Vec<knowledge_base::PlannedTask> = tasks.iter().map(|t| {
                knowledge_base::PlannedTask {
                    id: t.id.clone(),
                    description: t.description.clone(),
                    files: t.files.clone(),
                    complexity: format!("{:?}", t.complexity),
                    rationale: String::new(),
                    depends_on: t.depends_on.clone(),
                    template: t.template.clone(),
                    edit: t.edit.as_ref().map(|e| knowledge_base::PlannedEdit {
                        path: e.path.clone(),
                        old: e.old.clone(),
                        new: e.new.clone(),
                    }),
                }
            }).collect();

            let plan = knowledge_base::GoalPlan {
                id: None,
                run_id: task_id.to_string(),
                project: project.to_string(),
                goal: goal.to_string(),
                version,
                sub_tasks,
                model_used: config.tiers.orchestrator.model.clone(),
                tokens_input: 0,
                tokens_output: 0,
                duration_ms,
                user_feedback: previous_plans.last().and_then(|p| p.user_feedback.clone()),
                status: "draft".to_string(),
                context_files: repo_files,
                web_searches: vec![],
                reasoning: format!("Decomposed into {} sub-tasks", tasks.len()),
                created_at: chrono::Utc::now().to_rfc3339(),
                retrieved_pattern_ids,
            };

            let _ = store.store_plan(&plan).await;
            let msg = format!("Plan v{} ready — {} sub-tasks. Waiting for approval.", version, tasks.len());
            let _ = store.db_query_raw(&format_update(task_id, &format!("SET status = 'planned', progress_message = '{}'", msg.replace('\'', "")))).await;
            info!(task_id, version, tasks = tasks.len(), "Plan generated, awaiting approval");
        }
        Err(e) => {
            let _ = store.db_query_raw(&format_update(task_id, &format!("SET status = 'failed', error_message = 'Planning failed: {}'", e.to_string().replace('\'', "")))).await;
        }
    }
}

/// Execute with an approved plan. `handle_execute` detects the approved plan
/// and routes through the persisted workflow engine (no re-planning).
#[allow(clippy::too_many_arguments)]
async fn handle_approved(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    publisher: Option<Arc<EventPublisher>>,
    engine: Arc<swarm_workflow::WorkflowEngine>,
    task_id: &str,
    project: &str,
    goal: &str,
) {
    info!(task_id, project, "Executing approved plan");
    handle_execute(config, router, ollama, store, publisher, engine, task_id, project, goal).await;
}

/// Standard execution: claim → plan → execute → PR.
#[allow(clippy::too_many_arguments)]
async fn handle_execute(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    publisher: Option<Arc<EventPublisher>>,
    engine: Arc<swarm_workflow::WorkflowEngine>,
    task_id: &str,
    project: &str,
    goal: &str,
) {
    info!(task_id, project, goal, "Starting task execution");

    // 1. Claim the task atomically
    let now = chrono::Utc::now().to_rfc3339();
    let claim_query = format_update_where(task_id, &format!("SET status = 'running', agent_id = 'daemon', last_activity_at = '{}'", now), "status IN ['pending', 'approved']");
    match store.db_query_raw(&claim_query).await {
        Ok(_) => {}
        Err(e) => {
            warn!(task_id, "Failed to claim task: {e}");
            return;
        }
    }

    // Emit agent started event
    if let Some(pub_) = &publisher {
        let _ = pub_.publish(&SwarmEvent::AgentStarted {
            project: project.into(),
            agent_id: format!("daemon-{}", &task_id[..task_id.len().min(8)]),
            task: goal.into(),
            model: "auto".into(),
            files: vec![],
            timestamp: SwarmEvent::timestamp(),
        }).await;
    }

    // 2. Look up repo URL
    let repo_url = match store.get_project_repo(project).await {
        Ok(Some(url)) => url,
        Ok(None) => {
            fail_task(store.as_ref(), &publisher, task_id, project, goal, "No repo URL configured for project").await;
            return;
        }
        Err(e) => {
            fail_task(store.as_ref(), &publisher, task_id, project, goal, &format!("Failed to query project: {e}")).await;
            return;
        }
    };

    // 3. Clone/update repo (via git-provider NATS service, local fallback)
    let git = crate::provider_client::GitProviderClient::new(&config.nats.url).await;
    let repo_path_str = match git.ensure_repo(project, &repo_url).await {
        Ok(p) => p,
        Err(e) => {
            fail_task(store.as_ref(), &publisher, task_id, project, goal, &format!("Git clone failed: {e}")).await;
            return;
        }
    };
    let repo_path = std::path::PathBuf::from(&repo_path_str);

    info!(task_id, repo = %repo_path.display(), "Repo ready, executing swarm");

    // Lifecycle hooks: fired by the runner (per-task) and below (run-level).
    let hooks = {
        let mut hs = swarm_orchestrator::hooks::HookSet::new();
        hs.register(Arc::new(swarm_orchestrator::hooks::TracingHook));
        if config.learning.enabled {
            let memory = Arc::new(knowledge_base::MemoryStore::new(
                Arc::clone(&store), Arc::clone(&ollama), config.defaults.embed_model.clone(),
            ));
            hs.register(Arc::new(crate::hooks::TrajectoryRecorder::new(
                memory,
                Arc::clone(&store),
                Arc::clone(&router),
                config.tiers.orchestrator.clone(),
                config.learning.clone(),
            )));
        }
        Arc::new(hs)
    };

    // === PHASE TIMING ===
    let phase_start = std::time::Instant::now();
    let embed_ms: u64;

    // Phase 1: Embeddings
    let embed_model = config.defaults.embed_model.clone();
    let emb_manager = std::sync::Arc::new(knowledge_base::embedding_manager::EmbeddingManager::new(
        Arc::clone(&store), Arc::clone(&ollama), embed_model,
    ));
    {
        let indexed = emb_manager.on_agent_start(project, &repo_path).await;
        embed_ms = phase_start.elapsed().as_millis() as u64;
        if indexed > 0 {
            info!(indexed, duration_ms = embed_ms, "Phase 1: Embeddings (indexed)");
        } else {
            info!(duration_ms = embed_ms, "Phase 1: Embeddings (cached)");
        }
        update_progress(store.as_ref(), task_id, &format!("Phase 1: Embeddings done ({}ms)", embed_ms)).await;
    }

    // Helper: update progress on the running task
    async fn update_progress(store: &dyn KnowledgeBackend, task_id: &str, msg: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        let safe_msg = msg.replace('\'', "");
        let query = if task_id.contains(':') {
            format!("UPDATE {} SET last_activity_at = '{}', progress_message = '{}'", task_id, now, safe_msg)
        } else {
            format!("UPDATE type::thing('agent_run', '{}') SET last_activity_at = '{}', progress_message = '{}'", task_id, now, safe_msg)
        };
        let _ = store.db_query_raw(&query).await;
    }

    update_progress(store.as_ref(), task_id, "Planning goal decomposition...").await;

    // 4. Run the swarm orchestrator with retry loop (orchestrator tier fuel)
    let tier = &config.tiers.orchestrator;
    let start = std::time::Instant::now();
    let max_iterations = tier.max_iterations;
    let time_limit_ms: u64 = tier.time_limit_secs * 1000;
    let token_limit: u32 = tier.token_limit;
    let max_backoff = tier.max_backoff_secs;
    info!(task_id, model = %tier.model, time_limit = tier.time_limit_secs, token_limit, max_iterations, "Using orchestrator tier");
    let mut total_tokens_used: u32 = 0;
    let mut iteration = 0;
    let mut last_errors = String::new();
    let mut final_result = None;
    // Pattern ids injected into the approved plan's prompt (SONA signal).
    let mut plan_pattern_ids: Vec<String> = Vec::new();

    loop {
        iteration += 1;
        let elapsed = start.elapsed().as_millis() as u64;

        if elapsed > time_limit_ms {
            warn!(task_id, iteration, "Time fuel exhausted ({elapsed}ms > {time_limit_ms}ms)");
            break;
        }
        if total_tokens_used > token_limit {
            warn!(task_id, iteration, total_tokens_used, "Token fuel exhausted");
            break;
        }
        if iteration > max_iterations {
            warn!(task_id, iteration, "Max iterations reached");
            break;
        }

        // Exponential backoff between retries
        if iteration > 1 {
            let backoff = std::cmp::min(2u64.pow(iteration - 1), max_backoff);
            info!(task_id, iteration, backoff_secs = backoff, errors = %last_errors.chars().take(100).collect::<String>(), "Retrying after backoff");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        }

        // Build goal with error context from previous iteration
        let augmented_goal = if last_errors.is_empty() {
            goal.to_string()
        } else {
            format!("{}\n\nPREVIOUS ATTEMPT FAILED:\n{}\n\nFix the issues from the previous attempt.", goal, last_errors)
        };

        let progress_msg = if iteration > 1 {
            format!("Retry {}/{} — replanning...", iteration, max_iterations)
        } else {
            "Running agents...".to_string()
        };
        update_progress(store.as_ref(), task_id, &progress_msg).await;

        let wf_control = engine.control_for(task_id).await;
        let mut runner = swarm_orchestrator::SwarmRunner::new(Arc::clone(&router), Arc::clone(&ollama), &repo_path, project);
        runner = runner
            .with_store(Arc::clone(&store))
            .with_parent_run_id(task_id)
            .with_max_concurrent(config.resources.max_concurrent_agents)
            .with_planner_tier(config.tiers.orchestrator.clone())
            .with_depth(config.resources.max_sub_plan_depth)
            .with_embed_model(config.defaults.embed_model.clone())
            .with_hooks(Arc::clone(&hooks))
            .with_control(wf_control)
            .with_learning(config.learning.clone());

        // Zero-disk mode: opt-in via ZERO_DISK=1 (not just GITHUB_TOKEN)
        // GITHUB_TOKEN is used for PR creation regardless
        if std::env::var("ZERO_DISK").is_ok() {
            if let Ok(token) = std::env::var("GITHUB_TOKEN") {
                let gh_repo = std::env::var("GITHUB_REPO").unwrap_or_else(|_| "alpha-swarm/alpha-swarm2".into());
                let parts: Vec<&str> = gh_repo.splitn(2, '/').collect();
                if parts.len() == 2 {
                    runner = runner.with_github(swarm_orchestrator::GitHubRepo {
                        owner: parts[0].into(),
                        repo: parts[1].into(),
                        token,
                        branch: "main".into(),
                    });
                    info!("Zero-disk mode enabled (ZERO_DISK=1)");
                }
            }
        }

        // Connect to NATS for distributed tool dispatch (best-effort)
        if let Ok(nats_client) = async_nats::connect(&config.nats.url).await {
            runner = runner.with_nats_client(nats_client);
        }

        let run_start = std::time::Instant::now();
        update_progress(store.as_ref(), task_id, "Phase 2: Planning + Agent execution...").await;

        // Workflow path: an approved plan with persisted steps executes through
        // the workflow engine (resumable, replans on step failure — never
        // re-plans from scratch). Legacy goals fall back to runner.run().
        // NOTE: approval is recorded on agent_run.status (the approve route
        // does not touch goal_plan.status) — a run reaching execution with a
        // persisted plan means that plan IS the approved plan.
        let approved_tasks: Option<Vec<swarm_orchestrator::SubTask>> = if iteration == 1 {
            match store.get_latest_plan(task_id).await {
                Ok(Some(plan)) if !plan.sub_tasks.is_empty() => {
                    plan_pattern_ids = plan.retrieved_pattern_ids.clone();
                    Some(plan.sub_tasks.iter().map(planned_to_subtask).collect())
                }
                _ => None,
            }
        } else {
            None
        };

        let exec_result = if let Some(tasks) = approved_tasks {
            info!(task_id, steps = tasks.len(), "Executing via workflow engine (approved plan)");
            match run_workflow(&engine, store.as_ref(), &runner, &router, config, task_id, project, goal, &repo_path, tasks).await {
                Ok(Some(result)) => Ok(result),
                Ok(None) => {
                    // Paused or cancelled — workflow_run row is the durable
                    // state; this task releases its locks and exits.
                    info!(task_id, "Workflow paused/cancelled — exiting executor");
                    return;
                }
                Err(e) => Err(e),
            }
        } else {
            runner.run(&augmented_goal).await
        };

        match exec_result {
            Ok(result) => {
                let run_ms = run_start.elapsed().as_millis() as u64;
                let total_ms = phase_start.elapsed().as_millis() as u64;
                let pt = &result.phase_timings;
                info!(task_id, run_ms, total_ms, quality = result.quality_passed,
                    tasks = result.tasks.len(),
                    rag_ms = pt.rag_ms, planning_ms = pt.planning_ms,
                    agent_ms = pt.agent_execution_ms, qg_ms = pt.quality_gate_ms,
                    "Phase 2+3: Plan + Execute + QG complete");
                info!(task_id, summary = %pt.summary(), "Phase timing breakdown");
                // Track token usage
                let iter_tokens: u32 = result.results.iter()
                    .filter_map(|r| r.agent_result.as_ref())
                    .map(|a| a.inference_response.tokens_input + a.inference_response.tokens_output)
                    .sum();
                total_tokens_used += iter_tokens;

                if result.quality_passed {
                    let tasks_done = result.results.iter().filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied)).count();
                    update_progress(store.as_ref(), task_id, &format!(
                        "Quality passed — {} tasks done, creating PR... [{}]",
                        tasks_done, pt.summary()
                    )).await;
                    info!(task_id, iteration, total_tokens_used, "Quality gate passed!");
                    final_result = Some(result);
                    break;
                } else {
                    // Collect errors for next iteration
                    last_errors = result.results.iter()
                        .filter_map(|r| r.error.as_ref())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    info!(task_id, iteration, total_tokens_used, "Quality gate failed, will retry");
                    final_result = Some(result);
                    // Reset repo to clean state for next attempt
                    let _ = std::process::Command::new("git")
                        .args(["checkout", "."])
                        .current_dir(&repo_path)
                        .output();
                }
            }
            Err(e) => {
                last_errors = e.to_string();
                warn!(task_id, iteration, error = %e, "Swarm execution failed");
                final_result = None;
            }
        }
    }

    let duration = start.elapsed().as_millis() as u64;

    let start_time_rfc3339 = chrono::Utc::now().checked_sub_signed(chrono::Duration::milliseconds(duration as i64))
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    match final_result {
        Some(result) => {
            let tasks_passed = result.results.iter().filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied)).count();
            let tasks_failed = result.results.iter().filter(|r| r.error.is_some()).count();
            // Check if any agent produced work — either via edits or via tool-based file writes
            let any_work_done = tasks_passed > 0 || !result.merged_diff.as_ref().is_none_or(|d| d.is_empty());

            // A run with zero successful sub-agents is always a failure
            let status = if !any_work_done {
                RunStatus::Failed
            } else if result.quality_passed {
                RunStatus::Passed
            } else {
                RunStatus::Failed
            };

            // Land verified changes onto swarm/auto for review/merge (the diff
            // was only RECORDED before; this actually commits it on a branch).
            if matches!(status, RunStatus::Passed) {
                land_to_branch(&repo_url, task_id, goal, &result.modified_files);
            }

            info!(
                task_id, project,
                quality_passed = result.quality_passed,
                any_work_done,
                tasks = result.tasks.len(),
                tasks_passed,
                tasks_failed,
                duration_ms = duration,
                "Swarm completed"
            );

            // Collect actual models used by sub-agents
            let models_used: Vec<String> = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .map(|a| a.inference_response.model.clone())
                .filter(|m| !m.is_empty())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let model_str = if models_used.is_empty() { "unknown".to_string() } else { models_used.join(", ") };

            // Aggregate token counts from sub-agents
            let (total_in, total_out) = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .fold((0u32, 0u32), |(i, o), a| (i + a.inference_response.tokens_input, o + a.inference_response.tokens_output));

            // Store the diff regardless of PR outcome
            let captured_diff = result.merged_diff.clone();

            // Build run record with full tracking data
            let mut final_run = AgentRun::new(project, goal, "daemon", &model_str);
            final_run.status = status;
            final_run.duration_ms = duration;
            final_run.quality_gate_passed = Some(result.quality_passed && any_work_done);
            final_run.diff = captured_diff;
            final_run.tokens_input = total_in;
            final_run.tokens_output = total_out;
            final_run.started_at = Some(start_time_rfc3339);
            final_run.last_activity_at = Some(chrono::Utc::now().to_rfc3339());

            // Store phase timing breakdown
            let pt = &result.phase_timings;
            final_run.phase_timings = Some(knowledge_base::PhaseTimingRecord {
                embedding_ms: embed_ms,
                rag_ms: pt.rag_ms,
                planning_ms: pt.planning_ms,
                agent_execution_ms: pt.agent_execution_ms,
                quality_gate_ms: pt.quality_gate_ms,
            });
            let total_profiled = embed_ms + pt.rag_ms + pt.planning_ms + pt.agent_execution_ms + pt.quality_gate_ms;
            info!(task_id, embed_ms, rag_ms = pt.rag_ms, planning_ms = pt.planning_ms,
                agent_ms = pt.agent_execution_ms, qg_ms = pt.quality_gate_ms,
                total_profiled, total_wall = duration,
                "Full phase timing (embed + runner)");

            // Build attempts from sub-agent results (one per sub-task)
            final_run.attempts = result.results.iter().enumerate().map(|(i, r)| {
                AttemptRecord {
                    attempt: (i + 1) as u32,
                    model: r.agent_result.as_ref().map(|a| a.inference_response.model.clone()).unwrap_or_default(),
                    prompt_preview: r.task.description.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
                    response_preview: r.agent_result.as_ref()
                        .map(|a| a.inference_response.content.chars().take(ATTEMPT_PREVIEW_CHARS).collect())
                        .unwrap_or_default(),
                    tokens_input: r.agent_result.as_ref().map(|a| a.inference_response.tokens_input).unwrap_or(0),
                    tokens_output: r.agent_result.as_ref().map(|a| a.inference_response.tokens_output).unwrap_or(0),
                    duration_ms: r.agent_result.as_ref().map(|a| a.inference_response.duration_ms).unwrap_or(0),
                    quality_passed: r.agent_result.as_ref().map(|a| a.applied),
                    error: r.error.clone(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                }
            }).collect();

            // Aggregate response text from all sub-agents
            let responses: Vec<String> = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .map(|a| a.inference_response.content.clone())
                .collect();
            if !responses.is_empty() {
                final_run.response_text = Some(responses.join("\n---\n"));
            }

            // Collect files modified from sub-agents
            final_run.files_modified = result.results.iter()
                .flat_map(|r| r.task.files.iter().cloned())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            // Collect tool calls from all sub-agents
            final_run.tool_calls = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .flat_map(|a| a.tool_calls.iter().cloned())
                .collect();

            // Collect errors
            let agent_errors: Vec<String> = result.results.iter()
                .filter_map(|r| r.error.as_ref())
                .cloned()
                .collect();

            if !any_work_done {
                final_run.error_message = Some(if agent_errors.is_empty() {
                    "All agents completed without making changes".into()
                } else {
                    format!("All agents failed:\n{}", agent_errors.join("\n"))
                });
            } else if !result.quality_passed && !agent_errors.is_empty() {
                final_run.error_message = Some(agent_errors.join("\n"));
            }

            // 5. Apply captured files to main repo for git CLI PR
            if any_work_done {
                for (path, content) in &result.modified_files {
                    let full_path = repo_path.join(path);
                    if let Some(parent) = full_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&full_path, content);
                }
            }

            // 6. Create PR if there are actual changes
            if any_work_done {
                let _sub_tasks_info: Vec<(String, String, String)> = result.results.iter()
                    .map(|r| {
                        let model = r.agent_result.as_ref().map(|a| a.inference_response.model.clone()).unwrap_or_default();
                        let st = if r.error.is_some() { "failed" } else if r.agent_result.as_ref().is_some_and(|a| a.applied) { "passed" } else { "skipped" };
                        (r.task.description.clone(), model, st.to_string())
                    })
                    .collect();

                // Create PR via GitHub API (no git CLI, no disk writes)
                let gh_token = std::env::var("GITHUB_TOKEN").ok();
                let gh_repo = std::env::var("GITHUB_REPO").unwrap_or_else(|_| "alpha-swarm/alpha-swarm2".into());

                if let Some(token) = gh_token {
                    if !result.modified_files.is_empty() {
                        let parts: Vec<&str> = gh_repo.splitn(2, '/').collect();
                        let (owner, repo_name) = (parts[0], parts.get(1).unwrap_or(&""));

                        let gh_config = virt_git::GitHubConfig {
                            owner: owner.into(),
                            repo: repo_name.to_string(),
                            token: token.clone(),
                            base_branch: "main".into(),
                        };

                        // Build workspace from captured files
                        let mut blob_store = virt_git::MemoryBlobStore::new();
                        let mut ws = virt_git::VirtWorkspace::new();

                        // Load original files from repo
                        for (path, _) in &result.modified_files {
                            if let Ok(original) = std::fs::read_to_string(repo_path.join(path)) {
                                ws.load_file(&mut blob_store, path, &original);
                            }
                        }
                        // Apply modified versions
                        for (path, content) in &result.modified_files {
                            if let Ok(text) = std::str::from_utf8(content) {
                                ws.write_file(&mut blob_store, path, text);
                            }
                        }

                        let branch = format!("agent/{}", task_id.replace(':', "-").chars().take(40).collect::<String>());
                        let diff_text = ws.diff_text(&blob_store);
                        let pr_body = format!("Generated by alpha-swarm agent.\n\n```diff\n{}\n```\n\n🤖 alpha-swarm", &diff_text[..diff_text.len().min(3000)]);

                        let http_client = reqwest::Client::new();
                        match virt_git::create_pr(
                            &gh_config, &ws, &blob_store,
                            &format!("agent: {}", &goal[..goal.len().min(60)]),
                            &format!("agent: {}", &goal[..goal.len().min(60)]),
                            &pr_body, &branch,
                            &|method, url, body, token| {
                                tokio::task::block_in_place(|| {
                                    tokio::runtime::Handle::current().block_on(async {
                                        let mut req = match method {
                                            "GET" => http_client.get(url),
                                            "POST" => http_client.post(url),
                                            _ => return Err(format!("Unknown method: {method}")),
                                        };
                                        req = req.header("Authorization", format!("Bearer {token}"))
                                            .header("Accept", "application/vnd.github+json")
                                            .header("User-Agent", "alpha-swarm");
                                        if !body.is_empty() {
                                            req = req.header("Content-Type", "application/json").body(body.to_string());
                                        }
                                        let resp = req.send().await.map_err(|e| format!("HTTP: {e}"))?;
                                        let status = resp.status();
                                        let text = resp.text().await.map_err(|e| format!("Read: {e}"))?;
                                        if !status.is_success() {
                                            return Err(format!("GitHub {status}: {}", &text[..text.len().min(200)]));
                                        }
                                        Ok(text)
                                    })
                                })
                            },
                        ) {
                            Ok(pr) => {
                                info!(pr_url = %pr.pr_url, "PR created via GitHub API (no git CLI)");
                                final_run.diff = Some(format!("PR: {}", pr.pr_url));
                            }
                            Err(e) => {
                                warn!("GitHub API PR failed: {e}, falling back to git CLI");
                                // Fallback to git CLI
                                match git.create_pr(&repo_path_str, goal, result.quality_passed, duration, total_in, total_out).await {
                                    Ok(pr_url) => {
                                        info!(pr_url = %pr_url, "PR created via git CLI fallback");
                                        final_run.diff = Some(format!("PR: {}", pr_url));
                                    }
                                    Err(e) => warn!("Git CLI PR also failed: {e}"),
                                }
                            }
                        }
                    } else {
                        warn!("No modified files captured, falling back to git CLI PR");
                        match git.create_pr(&repo_path_str, goal, result.quality_passed, duration, total_in, total_out).await {
                            Ok(pr_url) => { final_run.diff = Some(format!("PR: {}", pr_url)); }
                            Err(e) => { warn!("PR creation failed: {e}"); }
                        }
                    }
                } else {
                    // No GITHUB_TOKEN, use git CLI
                    match git.create_pr(&repo_path_str, goal, result.quality_passed, duration, total_in, total_out).await {
                        Ok(pr_url) => { final_run.diff = Some(format!("PR: {}", pr_url)); }
                        Err(e) => { warn!("PR creation failed: {e}"); }
                    }
                }
            }

            let _ = store.update_run(task_id, &final_run).await;

            // Run-level hook: fired after the run record is persisted.
            {
                let plan_summary: String = result.results.iter()
                    .map(|r| {
                        let st = if r.error.is_some() { "failed" }
                            else if r.agent_result.as_ref().is_some_and(|a| a.applied) { "passed" }
                            else { "noop" };
                        format!("[{}] {}: {}", st, r.task.id, r.task.description)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut pattern_ids = result.retrieved_pattern_ids.clone();
                pattern_ids.extend(plan_pattern_ids.iter().cloned());
                pattern_ids.dedup();
                hooks.emit_run_complete(&swarm_orchestrator::hooks::RunCompleteCtx {
                    run_id: task_id,
                    project,
                    goal,
                    quality_passed: result.quality_passed,
                    tasks_passed,
                    tasks_failed,
                    total_duration_ms: duration,
                    retrieved_pattern_ids: &pattern_ids,
                    plan_summary: &plan_summary,
                }).await;
            }

            // Lifecycle: on_agent_done — update embeddings for modified files only
            if !final_run.files_modified.is_empty() {
                emb_manager.on_agent_done(project, &repo_path, &final_run.files_modified).await;
            }

            // Emit completion event
            if let Some(pub_) = &publisher {
                let _ = pub_.publish(&SwarmEvent::SwarmCompleted {
                    project: project.into(),
                    goal: goal.into(),
                    quality_passed: result.quality_passed,
                    tasks_passed: tasks_passed as u32,
                    tasks_failed: tasks_failed as u32,
                    total_duration_ms: duration,
                    timestamp: SwarmEvent::timestamp(),
                }).await;
            }
        }
        None => {
            fail_task_with_duration(store.as_ref(), &publisher, task_id, project, goal,
                &format!("All {} iterations failed. Last error: {}", iteration, last_errors), duration).await;
        }
    }
}

/// Execute (or resume) a workflow run through the engine.
/// Returns `Ok(Some(result))` on completion, `Ok(None)` when the run was
/// paused/cancelled (agent_run status already updated), `Err` on failure.
#[allow(clippy::too_many_arguments)]
async fn run_workflow(
    engine: &swarm_workflow::WorkflowEngine,
    store: &dyn KnowledgeBackend,
    runner: &swarm_orchestrator::SwarmRunner,
    router: &InferenceRouter,
    config: &SwarmConfig,
    task_id: &str,
    project: &str,
    goal: &str,
    repo_path: &std::path::Path,
    tasks: Vec<swarm_orchestrator::SubTask>,
) -> anyhow::Result<Option<swarm_orchestrator::SwarmResult>> {
    use swarm_workflow::{EngineContext, EngineOutcome, WorkflowRun};

    // Resume an existing non-terminal run, else create one from the plan.
    let mut wf = match engine.repo().get_by_run_id(task_id).await? {
        Some(existing) if !existing.state.is_terminal() => {
            info!(task_id, state = ?existing.state, "Resuming persisted workflow run");
            existing
        }
        _ => {
            let wf = WorkflowRun::from_tasks(
                project, goal, task_id, tasks,
                chrono::Utc::now().to_rfc3339(),
            ).with_trailing_quality_gate();
            engine.repo().create_run(&wf).await?;
            wf
        }
    };

    let ctx = EngineContext {
        runner,
        router,
        planner_tier: &config.tiers.orchestrator,
        repo_files: discover_source_files(repo_path),
        repo_path: repo_path.to_path_buf(),
    };

    match engine.execute(&mut wf, &ctx).await? {
        EngineOutcome::Completed(result) => Ok(Some(result)),
        EngineOutcome::Failed { result: _, error } => Err(anyhow::anyhow!(error)),
        EngineOutcome::Paused => {
            let _ = store.db_query_raw(&format_update(
                task_id,
                "SET status = 'paused', progress_message = 'Workflow paused — awaiting resume'",
            )).await;
            Ok(None)
        }
        EngineOutcome::Cancelled => {
            let _ = store.db_query_raw(&format_update(
                task_id,
                "SET status = 'failed', error_message = 'Workflow cancelled by user'",
            )).await;
            Ok(None)
        }
    }
}

async fn fail_task(
    store: &dyn KnowledgeBackend,
    publisher: &Option<Arc<EventPublisher>>,
    task_id: &str, project: &str, goal: &str, error: &str,
) {
    fail_task_with_duration(store, publisher, task_id, project, goal, error, 0).await;
}

async fn fail_task_with_duration(
    store: &dyn KnowledgeBackend,
    publisher: &Option<Arc<EventPublisher>>,
    task_id: &str, project: &str, goal: &str, error_msg: &str, duration_ms: u64,
) {
    error!(task_id, project, error = error_msg, "Task failed");

    let mut run = AgentRun::new(project, goal, "daemon", "error");
    run.status = RunStatus::Failed;
    run.error_message = Some(error_msg.to_string());
    run.duration_ms = duration_ms;
    let _ = store.update_run(task_id, &run).await;

    if let Some(pub_) = &publisher {
        let _ = pub_.publish(&SwarmEvent::AgentFailed {
            project: project.into(),
            agent_id: format!("daemon-{}", &task_id[..task_id.len().min(8)]),
            error: error_msg.into(),
            model: String::new(),
            duration_ms,
            timestamp: SwarmEvent::timestamp(),
        }).await;
    }
}
