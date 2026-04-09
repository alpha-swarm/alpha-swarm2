use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use inference_client::{Complexity, InferenceOptions, InferenceResponse, InferenceRouter, OllamaBackend};
use knowledge_base::{AgentRun, AttemptRecord, KnowledgeStore, RunStatus};
use swarm_events::{EventPublisher, SwarmEvent};

use crate::parser::{FileEdit, parse_edits};
use crate::prompt::build_prompt;

// --- Constants ---

/// Max characters to store in attempt preview fields.
const ATTEMPT_PREVIEW_CHARS: usize = 500;
/// Max characters for tool result fed back to model context.
const TOOL_RESULT_MAX_CHARS: usize = 2_000;
/// Max characters for tool call/result previews stored in records.
const TOOL_RECORD_PREVIEW_CHARS: usize = 200;

// --- Helper functions ---

/// Inference options for a given tier.
fn tier_options(tier: &swarm_config::TierConfig) -> InferenceOptions {
    InferenceOptions {
        max_tokens: Some(tier.context_window),
        preferred_model: Some(tier.model.clone()),
        preferred_backend: Some(inference_client::BackendKind::Ollama),
        ..Default::default()
    }
}

/// Default inference options — agent tier.
/// Respects ALPHA_SWARM_AGENT_MODEL env var for model override.
fn default_options() -> InferenceOptions {
    let mut tier = swarm_config::TierConfig::agent();
    if let Ok(model) = std::env::var("ALPHA_SWARM_AGENT_MODEL") {
        tier.model = model;
    }
    if let Ok(ctx) = std::env::var("ALPHA_SWARM_AGENT_CTX")
        && let Ok(n) = ctx.parse() { tier.context_window = n; }
    tier_options(&tier)
}

/// Auto-wrap heuristic: if model output content without <<< blocks,
/// and it doesn't look conversational, wrap as <<<CREATE target_file>>>.
fn try_auto_wrap(content: &str, file_paths: &[String], repo_path: &Path) -> Vec<FileEdit> {
    if content.is_empty() || content.contains("<<<") {
        return Vec::new();
    }

    let is_conversational = content.starts_with("I ") || content.starts_with("Here ")
        || content.starts_with("To ") || content.starts_with("You ")
        || content.starts_with("The ") || content.starts_with("This ")
        || content.contains("you can") || content.contains("you should");

    if is_conversational {
        return Vec::new();
    }

    // Only wrap for non-existent files — CREATE on existing files would overwrite them
    let target = file_paths.iter()
        .find(|f| !repo_path.join(f).exists());

    if let Some(target) = target {
        info!(file = %target, "Auto-wrapping response as CREATE");
        let wrapped = format!("<<<CREATE {target}\n{content}\n>>>");
        parse_edits(&wrapped).unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Result of an agent run.
pub struct AgentResult {
    pub edits: Vec<FileEdit>,
    pub inference_response: InferenceResponse,
    pub applied: bool,
    pub skipped: bool,
    pub run_id: Option<String>,
    pub attempt: u32,
    pub escalated_from: Option<String>,
    pub tool_calls: Vec<knowledge_base::ToolCallRecord>,
}

/// Configuration for knowledge-aware agent.
pub struct KnowledgeConfig {
    pub store: Arc<KnowledgeStore>,
    pub embedder: Arc<OllamaBackend>,
    pub embed_model: String,
    pub project: String,
    pub skip_threshold: f32,
    pub parent_run_id: Option<String>,
}

/// A one-shot code modification agent.
pub struct Agent {
    router: Arc<InferenceRouter>,
    repo_path: PathBuf,
    knowledge: Option<KnowledgeConfig>,
    events: Option<Arc<EventPublisher>>,
    project: String,
    /// Optional file provider for zero-disk mode.
    file_provider: Option<Box<dyn crate::file_provider::FileProvider>>,
}

impl Agent {
    pub fn new(router: Arc<InferenceRouter>, repo_path: impl Into<PathBuf>) -> Self {
        Self {
            router,
            repo_path: repo_path.into(),
            knowledge: None,
            events: None,
            project: "default".into(),
            file_provider: None,
        }
    }

    /// Use a VirtFileProvider for zero-disk operation.
    pub fn with_file_provider(mut self, provider: impl crate::file_provider::FileProvider + 'static) -> Self {
        self.file_provider = Some(Box::new(provider));
        self
    }

    /// Take the file provider out (for extracting modified files after agent completes).
    pub fn take_file_provider(&mut self) -> Option<Box<dyn crate::file_provider::FileProvider>> {
        self.file_provider.take()
    }

    pub fn with_knowledge(mut self, config: KnowledgeConfig) -> Self {
        self.project = config.project.clone();
        self.knowledge = Some(config);
        self
    }

    pub fn with_events(mut self, publisher: Arc<EventPublisher>) -> Self {
        self.events = Some(publisher);
        self
    }

    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = project.into();
        self
    }

    /// Run a one-shot task: read files, call LLM, parse edits, apply them.
    pub async fn run(
        &mut self,
        task: &str,
        file_paths: &[String],
        complexity: Complexity,
    ) -> Result<AgentResult> {
        let agent_id = uuid::Uuid::new_v4().to_string();

        // --- Knowledge: check for similar past work ---
        let mut context_from_knowledge = String::new();
        if let Some(kc) = &self.knowledge {
            let embedding = kc.embedder.embed(&kc.embed_model, task).await
                .unwrap_or_default();

            if !embedding.is_empty() {
                // Check if already done
                if let Ok(Some(past)) = kc.store.task_already_done(&kc.project, &embedding, kc.skip_threshold).await {
                    info!(
                        past_task = %past.task_description,
                        "Similar task already completed — skipping"
                    );
                    return Ok(AgentResult {
                        edits: Vec::new(),
                        inference_response: InferenceResponse {
                            content: format!("Skipped: similar task already done ({})", past.task_description),
                            model: String::new(),
                            backend: inference_client::BackendKind::Ollama,
                            tokens_input: 0,
                            tokens_output: 0,
                            duration_ms: 0,
                        },
                        applied: false,
                        skipped: true,
                        run_id: None,
                        attempt: 1,
                        escalated_from: None,
                        tool_calls: Vec::new(),
                    });
                }

                // Check for past errors to avoid
                if let Ok(errors) = kc.store.find_past_errors(&kc.project, &embedding, 3).await
                    && !errors.is_empty() {
                        context_from_knowledge.push_str("\nPAST ERRORS TO AVOID:\n");
                        for e in &errors {
                            context_from_knowledge.push_str(&format!(
                                "- Task '{}' failed (model: {}): {}\n",
                                e.task_description,
                                e.model_used,
                                e.error_message.as_deref().unwrap_or("unknown error"),
                            ));
                        }
                        info!(error_count = errors.len(), "Found past errors to avoid");
                }

                // Check what parallel agents are doing
                if let Ok(running) = kc.store.running_agents(&kc.project).await
                    && !running.is_empty() {
                        context_from_knowledge.push_str("\nCURRENTLY RUNNING AGENTS:\n");
                        for r in &running {
                            context_from_knowledge.push_str(&format!(
                                "- Agent {} working on: '{}' (files: {:?})\n",
                                r.agent_id, r.task_description, r.files_modified,
                            ));
                        }
                        info!(running_count = running.len(), "Found parallel agents");
                }
            }
        }

        // 1. Read files
        let files = self.read_files(file_paths)?;
        info!(file_count = files.len(), "Read source files");

        // 2. Build prompt (with knowledge context if available)
        let mut messages = build_prompt(task, &files);
        if !context_from_knowledge.is_empty() {
            // Insert knowledge context before the user message
            if let Some(last) = messages.last_mut() {
                last.content.push_str(&context_from_knowledge);
            }
        }

        // --- Emit: agent started ---
        if let Some(pub_) = &self.events {
            let _ = pub_.publish(&SwarmEvent::AgentStarted {
                project: self.project.clone(),
                agent_id: agent_id.clone(),
                task: task.to_string(),
                model: "pending".into(),
                files: file_paths.to_vec(),
                timestamp: SwarmEvent::timestamp(),
            }).await;
        }

        // Serialize prompt for tracking
        let prompt_json = serde_json::to_string(&messages).unwrap_or_default();
        let now = chrono::Utc::now().to_rfc3339();

        // --- Knowledge: record run start ---
        let mut run_record = None;
        let mut run_id = None;
        if let Some(kc) = &self.knowledge {
            let mut record = AgentRun::new(&kc.project, task, &agent_id, "pending");
            record.files_modified = file_paths.to_vec();
            record.prompt_sent = Some(prompt_json.clone());
            record.started_at = Some(now.clone());
            record.last_activity_at = Some(now);
            record.parent_run_id = kc.parent_run_id.clone();
            match kc.store.store_run(&record).await {
                Ok(id) => {
                    info!(run_id = %id, "Recorded run start in knowledge base");
                    run_id = Some(id);
                    run_record = Some(record);
                }
                Err(e) => warn!("Failed to record run: {e}"),
            }
        }

        // 3. Call inference
        let options = default_options();
        let response = self.router.chat(&messages, complexity, &options).await
            .context("Inference failed")?;

        info!(
            model = %response.model,
            backend = ?response.backend,
            tokens_in = response.tokens_input,
            tokens_out = response.tokens_output,
            duration_ms = response.duration_ms,
            "Inference complete"
        );

        // 4. Parse edits from response
        let content_preview: String = response.content.trim().chars().take(300).collect();
        info!(preview = %content_preview, tokens = response.tokens_output, "Standard run: model output");

        let edits = match parse_edits(&response.content) {
            Ok(edits) => edits,
            Err(e) => {
                warn!(error = %e, "Failed to parse edits");

                // Knowledge: record failure
                if let (Some(kc), Some(id), Some(mut record)) = (&self.knowledge, &run_id, run_record.take()) {
                    record.status = RunStatus::Failed;
                    record.error_message = Some(format!("Parse error: {e}"));
                    record.model_used = response.model.clone();
                    record.tokens_input = response.tokens_input;
                    record.tokens_output = response.tokens_output;
                    record.duration_ms = response.duration_ms;
                    record.response_text = Some(response.content.clone());
                    record.last_activity_at = Some(chrono::Utc::now().to_rfc3339());
                    let _ = kc.store.update_run(id, &record).await;

                    // Store embedding for future reference
                    if let Ok(emb) = kc.embedder.embed(&kc.embed_model, task).await {
                        let _ = kc.store.store_embedding(id, &emb).await;
                    }
                }

                return Ok(AgentResult {
                    edits: Vec::new(),
                    inference_response: response,
                    applied: false,
                    skipped: false,
                    run_id,
                    attempt: 1,
                    escalated_from: None,
                    tool_calls: Vec::new(),
                });
            }
        };

        // Auto-wrap: if no edits parsed, try wrapping raw content as CREATE
        let edits = if edits.is_empty() {
            let wrapped = try_auto_wrap(response.content.trim(), file_paths, &self.repo_path);
            if !wrapped.is_empty() { wrapped } else { edits }
        } else { edits };

        let edits = self.validate_edits(edits);
        info!(edit_count = edits.len(), "Parsed and validated file edits");

        // 5. Apply edits
        if !edits.is_empty() {
            self.apply_edits(&edits)?;
        }

        // --- Knowledge: record completion ---
        if let (Some(kc), Some(id), Some(mut record)) = (&self.knowledge, &run_id, run_record.take()) {
            record.status = if edits.is_empty() { RunStatus::Skipped } else { RunStatus::Passed };
            record.model_used = response.model.clone();
            record.tokens_input = response.tokens_input;
            record.tokens_output = response.tokens_output;
            record.duration_ms = response.duration_ms;
            record.diff = Some(response.content.clone());
            record.response_text = Some(response.content.clone());
            record.last_activity_at = Some(chrono::Utc::now().to_rfc3339());
            let _ = kc.store.update_run(id, &record).await;

            // Store embedding
            if let Ok(emb) = kc.embedder.embed(&kc.embed_model, task).await {
                let _ = kc.store.store_embedding(id, &emb).await;
            }
        }

        let has_edits = !edits.is_empty();

        // --- Emit: agent finished ---
        if let Some(pub_) = &self.events {
            let _ = pub_.publish(&SwarmEvent::AgentFinished {
                project: self.project.clone(),
                agent_id: agent_id.clone(),
                status: if has_edits { "passed".into() } else { "skipped".into() },
                edits: edits.len() as u32,
                tokens_input: response.tokens_input,
                tokens_output: response.tokens_output,
                duration_ms: response.duration_ms,
                model: response.model.clone(),
                timestamp: SwarmEvent::timestamp(),
            }).await;
        }

        Ok(AgentResult {
            edits,
            inference_response: response,
            applied: has_edits,
            skipped: false,
            run_id,
            attempt: 1,
            escalated_from: None,
            tool_calls: Vec::new(),
        })
    }

    /// Run with retry and model escalation.
    /// On quality gate failure: retry same model once, then escalate to larger model.
    pub async fn run_with_retry(
        &mut self,
        task: &str,
        file_paths: &[String],
        complexity: Complexity,
        repo_path: &std::path::Path,
        max_attempts: u32,
    ) -> Result<AgentResult> {
        let mut attempts = Vec::<AttemptRecord>::new();
        let mut last_result = self.run(task, file_paths, complexity).await?;

        // Record attempt 1
        attempts.push(AttemptRecord {
            attempt: 1,
            model: last_result.inference_response.model.clone(),
            prompt_preview: String::new(), // already stored in prompt_sent
            response_preview: last_result.inference_response.content.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
            tokens_input: last_result.inference_response.tokens_input,
            tokens_output: last_result.inference_response.tokens_output,
            duration_ms: last_result.inference_response.duration_ms,
            quality_passed: None,
            error: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        if !last_result.applied || last_result.skipped {
            self.update_attempts(&last_result.run_id, &attempts).await;
            return Ok(last_result);
        }

        // Check quality gate
        let config = quality_gate_lib::detect_toolchain(repo_path);
        let checks = quality_gate_lib::run_all(repo_path, &config).await?;
        let passed = checks.iter().all(|c| c.passed);

        attempts.last_mut().unwrap().quality_passed = Some(passed);

        if passed {
            self.update_attempts(&last_result.run_id, &attempts).await;
            return Ok(last_result);
        }

        // Quality gate failed — collect error context for retry
        let error_context: String = checks.iter()
            .filter(|c| !c.passed)
            .map(|c| format!("{}: {}", c.check_name, c.stderr.chars().take(300).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n");

        let failed_model = last_result.inference_response.model.clone();

        for attempt in 2..=max_attempts {
            info!(attempt, failed_model = %failed_model, "Retrying after quality gate failure");

            // Build retry task with error context
            let retry_task = format!(
                "{task}\n\nPREVIOUS ATTEMPT FAILED QUALITY GATE:\n{error_context}\n\nFix the issues from the previous attempt."
            );

            // Escalate model on 3rd+ attempt
            let options = if attempt >= 3 {
                match self.router.escalate_model(&failed_model, complexity).await {
                    Ok(bigger) => {
                        last_result.escalated_from = Some(failed_model.clone());
                        InferenceOptions {
                            preferred_model: Some(bigger.name.clone()),
                            preferred_backend: Some(bigger.backend),
                            ..Default::default()
                        }
                    }
                    Err(_) => InferenceOptions::default(),
                }
            } else {
                InferenceOptions::default()
            };

            // Re-read files (previous edits may have been applied)
            let messages = crate::prompt::build_prompt(&retry_task, &self.read_files(file_paths)?);
            let response = self.router.chat(&messages, complexity, &options).await
                .context("Retry inference failed")?;

            let edits = match parse_edits(&response.content) {
                Ok(e) => e,
                Err(e) => {
                    warn!("Retry parse failed: {e}");
                    attempts.push(AttemptRecord {
                        attempt,
                        model: response.model.clone(),
                        prompt_preview: retry_task.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
                        response_preview: response.content.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
                        tokens_input: response.tokens_input,
                        tokens_output: response.tokens_output,
                        duration_ms: response.duration_ms,
                        quality_passed: None,
                        error: Some(format!("Parse error: {e}")),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    });
                    self.update_attempts(&last_result.run_id, &attempts).await;
                    continue;
                }
            };

            if !edits.is_empty() {
                self.apply_edits(&edits)?;
            }

            // Re-check quality gate
            let checks = quality_gate_lib::run_all(repo_path, &config).await?;
            let passed = checks.iter().all(|c| c.passed);

            attempts.push(AttemptRecord {
                attempt,
                model: response.model.clone(),
                prompt_preview: retry_task.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
                response_preview: response.content.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
                tokens_input: response.tokens_input,
                tokens_output: response.tokens_output,
                duration_ms: response.duration_ms,
                quality_passed: Some(passed),
                error: None,
                timestamp: chrono::Utc::now().to_rfc3339(),
            });

            last_result = AgentResult {
                edits,
                inference_response: response,
                applied: true,
                skipped: false,
                run_id: last_result.run_id,
                attempt,
                escalated_from: last_result.escalated_from,
                tool_calls: Vec::new(),
            };

            self.update_attempts(&last_result.run_id, &attempts).await;

            if passed {
                info!(attempt, "Retry succeeded");
                return Ok(last_result);
            }
        }

        warn!("All retry attempts exhausted");
        Ok(last_result)
    }

    async fn update_attempts(&self, run_id: &Option<String>, attempts: &[AttemptRecord]) {
        if let (Some(kc), Some(id)) = (&self.knowledge, run_id) {
            let now = chrono::Utc::now().to_rfc3339();
            let attempts_json = serde_json::to_string(attempts).unwrap_or_default();
            let query = if id.contains(':') {
                format!("UPDATE {} SET attempts = {}, last_activity_at = '{}'", id, attempts_json, now)
            } else {
                format!("UPDATE type::thing('agent_run', '{}') SET attempts = {}, last_activity_at = '{}'", id, attempts_json, now)
            };
            if let Err(e) = kc.store.db_query_raw(&query).await {
                warn!("Failed to update attempts: {e}");
            }
        }
    }

    /// Run a task using the text-based tool protocol.
    /// Model outputs <<<TOOL>>>, <<<EDIT>>>, <<<CREATE>>>, <<<DELETE>>>, <<<DONE>>> blocks.
    /// Works with ANY model — no Ollama-specific API needed.
    pub async fn run_with_tools(
        &mut self,
        task: &str,
        file_paths: &[String],
        complexity: Complexity,
        tools: &swarm_tools::ToolRegistry,
        max_steps: u32,
    ) -> Result<AgentResult> {
        use crate::parser::{parse_actions, AgentAction};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let files = self.read_files(file_paths)?;
        let tool_names = tools.tool_names();
        let messages_init = crate::prompt::build_tool_prompt(task, &files, &tool_names);

        let mut messages = messages_init;

        let _file_cache: Arc<RwLock<HashMap<String, String>>> = Arc::new(RwLock::new(
            files.iter().map(|(p, c)| (p.clone(), c.clone())).collect()
        ));

        let ctx = swarm_tools::ToolContext {
            repo_path: self.repo_path.clone(),
            project: self.project.clone(),
            timeout: std::time::Duration::from_secs(60),
        };

        let options = default_options();
        let mut total_tokens_in = 0u32;
        let mut total_tokens_out = 0u32;
        let mut total_duration = 0u64;
        let mut all_edits = Vec::new();
        let mut tool_call_records = Vec::new();

        for step in 1..=max_steps {
            let response = match self.router.chat(&messages, complexity, &options).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(step, error = %e, "Tool loop inference failed, falling back to standard run");
                    return self.run(task, file_paths, complexity).await;
                }
            };

            total_tokens_in += response.tokens_input;
            total_tokens_out += response.tokens_output;
            total_duration += response.duration_ms;

            info!(step, model = %response.model, tokens_out = response.tokens_output, "Tool loop step");

            let content = response.content.trim();

            // Debug: log first 200 chars of model output for diagnosis
            let preview: String = content.chars().take(200).collect();
            info!(step, preview = %preview, "Model output preview");

            let actions = match parse_actions(content) {
                Ok(a) if !a.is_empty() => a,
                Ok(_) | Err(_) => {
                    // No structured actions — try plain edit blocks
                    let edits = crate::parser::parse_edits(content).unwrap_or_default();
                    if !edits.is_empty() {
                        for edit in &edits {
                            let _ = self.apply_edits(std::slice::from_ref(edit));
                        }
                        all_edits.extend(edits);
                        info!(step, edits = all_edits.len(), "Parsed plain edit blocks");
                        break;
                    }

                    // Auto-wrap: try wrapping raw content as CREATE
                    let wrapped = try_auto_wrap(content, file_paths, &self.repo_path);
                    if !wrapped.is_empty() {
                        for edit in &wrapped {
                            let _ = self.apply_edits(std::slice::from_ref(edit));
                        }
                        all_edits.extend(wrapped);
                        break;
                    }

                    // Step 1 with no structured output → fall back to simpler EDIT_FORMAT
                    if step == 1 {
                        info!(step, content_len = content.len(), "No structured actions on step 1, falling back to standard run");
                        return self.run(task, file_paths, complexity).await;
                    }

                    info!(step, content_len = content.len(), "No parseable actions, treating as final response");
                    break;
                }
            };

            messages.push(inference_client::ChatMessage::assistant(&response.content));

            let mut feedback_parts = Vec::new();
            let mut done = false;

            for action in &actions {
                match action {
                    AgentAction::Tool(call) => {
                        let params: serde_json::Value = serde_json::from_str(&call.params_json)
                            .unwrap_or(serde_json::Value::Object(Default::default()));
                        info!(step, tool = %call.name, "Executing tool");
                        let result = tools.execute(&call.name, params.clone(), &ctx).await;
                        let prefix = if result.is_error { "ERROR" } else { "OK" };

                        // Record tool call
                        tool_call_records.push(knowledge_base::ToolCallRecord {
                            tool: call.name.clone(),
                            params_preview: call.params_json.chars().take(TOOL_RECORD_PREVIEW_CHARS).collect(),
                            result_preview: result.content.chars().take(TOOL_RECORD_PREVIEW_CHARS).collect(),
                            is_error: result.is_error,
                            duration_ms: result.duration_ms,
                        });

                        // Truncate long results to avoid context bloat
                        let content = if result.content.len() > TOOL_RESULT_MAX_CHARS {
                            format!("{}...(truncated)", &result.content[..TOOL_RESULT_MAX_CHARS])
                        } else {
                            result.content
                        };
                        feedback_parts.push(format!("[{} {}] {}", call.name, prefix, content));
                    }
                    AgentAction::Edit(edit) => {
                        let validated = self.validate_edits(vec![edit.clone()]);
                        if validated.is_empty() {
                            feedback_parts.push("[edit REJECTED] Invalid file path".into());
                        } else if let Err(e) = self.apply_edits(&validated) {
                            feedback_parts.push(format!("[edit ERROR] {e}"));
                        } else {
                            all_edits.extend(validated);
                            feedback_parts.push("[edit OK] Applied".into());
                        }
                    }
                    AgentAction::Agent { description, files, .. } => {
                        feedback_parts.push(format!("[sub-agent] {description} on {files:?}"));
                        match self.run(description, files, complexity).await {
                            Ok(sub) => {
                                all_edits.extend(sub.edits);
                                feedback_parts.push("[sub-agent OK]".into());
                            }
                            Err(e) => feedback_parts.push(format!("[sub-agent ERROR] {e}")),
                        }
                    }
                    AgentAction::Done { summary } => {
                        if all_edits.is_empty() && step <= 2 {
                            // Model said DONE without making edits — nudge it
                            info!(step, summary = %summary, "Tool loop: DONE with no edits, nudging");
                            feedback_parts.push(format!("[DONE] {summary}\n\nYou haven't made any edits yet. Please produce <<<EDIT>>> blocks to modify the files. Read the file first if needed, then output the edit."));
                        } else {
                            info!(step, summary = %summary, "Tool loop: DONE");
                            done = true;
                        }
                    }
                }
            }

            if done { break; }

            let feedback = feedback_parts.join("\n");
            messages.push(inference_client::ChatMessage::user(
                format!("TOOL RESULTS:\n{feedback}\n\nContinue with more <<<TOOL>>> calls, <<<EDIT>>>/<<<CREATE>>> blocks, or <<<DONE>>> if finished.")
            ));
        }

        let applied = !all_edits.is_empty();

        Ok(AgentResult {
            edits: all_edits,
            inference_response: InferenceResponse {
                content: String::new(),
                model: String::new(),
                backend: inference_client::BackendKind::Ollama,
                tokens_input: total_tokens_in,
                tokens_output: total_tokens_out,
                duration_ms: total_duration,
            },
            applied,
            skipped: false,
            run_id: None,
            attempt: 1,
            escalated_from: None,
            tool_calls: tool_call_records,
        })
    }

    /// Filter out edits with invalid/placeholder file paths.
    fn validate_edits(&self, edits: Vec<FileEdit>) -> Vec<FileEdit> {
        edits.into_iter().filter(|edit| {
            let path = match edit {
                FileEdit::Edit { path, .. } => path,
                FileEdit::Create { path, .. } => path,
                FileEdit::Delete { path } => path,
            };
            if crate::code_utils::is_valid_file_path(path) {
                true
            } else {
                warn!(path = %path, "Rejected edit with invalid/placeholder path");
                false
            }
        }).collect()
    }

    fn read_files(&self, paths: &[String]) -> Result<Vec<(String, String)>> {
        let mut files = Vec::new();
        for path in paths {
            let content = if let Some(ref fp) = self.file_provider {
                fp.read_file(path).unwrap_or_default()
            } else {
                match std::fs::read_to_string(self.repo_path.join(path)) {
                    Ok(c) => c,
                    Err(_) => {
                        info!(path = %path, "File not found, will be available for creation");
                        String::new()
                    }
                }
            };
            files.push((path.clone(), content));
        }
        Ok(files)
    }

    fn apply_edits(&mut self, edits: &[FileEdit]) -> Result<()> {
        for edit in edits {
            match edit {
                FileEdit::Edit { path, old, new } => {
                    let content = if let Some(ref fp) = self.file_provider {
                        fp.read_file(path).map_err(|e| anyhow::anyhow!(e))?
                    } else {
                        std::fs::read_to_string(self.repo_path.join(path))
                            .with_context(|| format!("Cannot read {path} for editing"))?
                    };

                    let updated = if content.contains(old.as_str()) {
                        // Exact match
                        content.replacen(old.as_str(), new.as_str(), 1)
                    } else {
                        // Try trimmed match — normalize whitespace
                        let old_trimmed = old.trim();
                        let lines: Vec<&str> = content.lines().collect();
                        let old_lines: Vec<&str> = old_trimmed.lines().map(|l| l.trim()).collect();

                        if old_lines.is_empty() {
                            warn!(path, "OLD block empty — skipping edit");
                            continue;
                        }

                        // Find the first line of OLD in the file
                        let mut found = false;
                        let mut start_line = 0;
                        for (i, line) in lines.iter().enumerate() {
                            if line.trim() == old_lines[0] {
                                // Check if subsequent lines match
                                let mut all_match = true;
                                for (j, old_line) in old_lines.iter().enumerate() {
                                    if i + j >= lines.len() || lines[i + j].trim() != *old_line {
                                        all_match = false;
                                        break;
                                    }
                                }
                                if all_match {
                                    start_line = i;
                                    found = true;
                                    break;
                                }
                            }
                        }

                        if !found {
                            warn!(path, "OLD block not found in file (even with trimmed match) — skipping");
                            continue;
                        }

                        // Replace the matched lines preserving original indentation
                        let mut result_lines: Vec<String> = Vec::new();
                        result_lines.extend(lines[..start_line].iter().map(|l| l.to_string()));
                        // Use new content as-is
                        for new_line in new.lines() {
                            result_lines.push(new_line.to_string());
                        }
                        result_lines.extend(lines[start_line + old_lines.len()..].iter().map(|l| l.to_string()));
                        info!(path, "Applied edit with trimmed matching");
                        result_lines.join("\n")
                    };
                    if let Some(ref mut fp) = self.file_provider {
                        fp.write_file(path, &updated).map_err(|e| anyhow::anyhow!(e))?;
                    } else {
                        let full_path = self.repo_path.join(path);
                        std::fs::write(&full_path, updated)
                            .with_context(|| format!("Failed to write {path}"))?;
                    }
                    info!(path, "Applied edit");
                }
                FileEdit::Create { path, content } => {
                    if let Some(ref mut fp) = self.file_provider {
                        fp.write_file(path, content).map_err(|e| anyhow::anyhow!(e))?;
                    } else {
                        let full_path = self.repo_path.join(path);
                        if let Some(parent) = full_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(&full_path, content)
                            .with_context(|| format!("Failed to create {path}"))?;
                    }
                    info!(path, "Created file");
                }
                FileEdit::Delete { path } => {
                    let full_path = self.repo_path.join(path);
                    if full_path.exists() {
                        std::fs::remove_file(&full_path)
                            .with_context(|| format!("Failed to delete {path}"))?;
                        info!(path, "Deleted file");
                    }
                }
            }
        }
        Ok(())
    }
}
