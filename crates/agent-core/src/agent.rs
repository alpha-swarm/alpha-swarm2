use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing::{info, warn};

use inference_client::{Complexity, InferenceOptions, InferenceResponse, InferenceRouter, OllamaBackend};

/// Inference options for a given tier.
fn tier_options(tier: &swarm_config::TierConfig) -> InferenceOptions {
    InferenceOptions {
        max_tokens: Some(tier.context_window),
        preferred_model: Some(tier.model.clone()),
        ..Default::default()
    }
}

/// Default inference options — agent tier.
fn default_options() -> InferenceOptions {
    tier_options(&swarm_config::TierConfig::agent())
}
use knowledge_base::{AgentRun, KnowledgeStore, RunStatus};
use swarm_events::{EventPublisher, SwarmEvent};

use crate::parser::{FileEdit, parse_edits};
use crate::prompt::build_prompt;

/// Result of an agent run.
pub struct AgentResult {
    pub edits: Vec<FileEdit>,
    pub inference_response: InferenceResponse,
    pub applied: bool,
    pub skipped: bool,
    pub run_id: Option<String>,
    pub attempt: u32,
    pub escalated_from: Option<String>,
}

/// Configuration for knowledge-aware agent.
pub struct KnowledgeConfig<'a> {
    pub store: &'a KnowledgeStore,
    pub embedder: &'a OllamaBackend,
    pub embed_model: String,
    pub project: String,
    pub skip_threshold: f32,
}

/// A one-shot code modification agent.
pub struct Agent<'a> {
    router: &'a InferenceRouter,
    repo_path: PathBuf,
    knowledge: Option<KnowledgeConfig<'a>>,
    events: Option<&'a EventPublisher>,
    project: String,
}

impl<'a> Agent<'a> {
    pub fn new(router: &'a InferenceRouter, repo_path: impl Into<PathBuf>) -> Self {
        Self {
            router,
            repo_path: repo_path.into(),
            knowledge: None,
            events: None,
            project: "default".into(),
        }
    }

    pub fn with_knowledge(mut self, config: KnowledgeConfig<'a>) -> Self {
        self.project = config.project.clone();
        self.knowledge = Some(config);
        self
    }

    pub fn with_events(mut self, publisher: &'a EventPublisher) -> Self {
        self.events = Some(publisher);
        self
    }

    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = project.into();
        self
    }

    /// Run a one-shot task: read files, call LLM, parse edits, apply them.
    pub async fn run(
        &self,
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
                    });
                }

                // Check for past errors to avoid
                if let Ok(errors) = kc.store.find_past_errors(&kc.project, &embedding, 3).await {
                    if !errors.is_empty() {
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
                }

                // Check what parallel agents are doing
                if let Ok(running) = kc.store.running_agents(&kc.project).await {
                    if !running.is_empty() {
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

        // --- Knowledge: record run start ---
        let mut run_record = None;
        let mut run_id = None;
        if let Some(kc) = &self.knowledge {
            let mut record = AgentRun::new(&kc.project, task, &agent_id, "pending");
            record.files_modified = file_paths.to_vec();
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
        let edits = match parse_edits(&response.content) {
            Ok(edits) => edits,
            Err(e) => {
                warn!("Failed to parse edits: {e}");

                // Knowledge: record failure
                if let (Some(kc), Some(id), Some(mut record)) = (&self.knowledge, &run_id, run_record.take()) {
                    record.status = RunStatus::Failed;
                    record.error_message = Some(format!("Parse error: {e}"));
                    record.model_used = response.model.clone();
                    record.tokens_input = response.tokens_input;
                    record.tokens_output = response.tokens_output;
                    record.duration_ms = response.duration_ms;
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
                });
            }
        };

        info!(edit_count = edits.len(), "Parsed file edits");

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
        })
    }

    /// Run with retry and model escalation.
    /// On quality gate failure: retry same model once, then escalate to larger model.
    pub async fn run_with_retry(
        &self,
        task: &str,
        file_paths: &[String],
        complexity: Complexity,
        repo_path: &std::path::Path,
        max_attempts: u32,
    ) -> Result<AgentResult> {
        let mut last_result = self.run(task, file_paths, complexity).await?;
        if !last_result.applied || last_result.skipped {
            return Ok(last_result);
        }

        // Check quality gate
        let config = quality_gate_lib::detect_toolchain(repo_path);
        let checks = quality_gate_lib::run_all(repo_path, &config).await?;
        let passed = checks.iter().all(|c| c.passed);

        if passed {
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
                    continue;
                }
            };

            if !edits.is_empty() {
                self.apply_edits(&edits)?;
            }

            // Re-check quality gate
            let checks = quality_gate_lib::run_all(repo_path, &config).await?;
            let passed = checks.iter().all(|c| c.passed);

            last_result = AgentResult {
                edits,
                inference_response: response,
                applied: true,
                skipped: false,
                run_id: last_result.run_id,
                attempt,
                escalated_from: last_result.escalated_from,
            };

            if passed {
                info!(attempt, "Retry succeeded");
                return Ok(last_result);
            }
        }

        warn!("All retry attempts exhausted");
        Ok(last_result)
    }

    fn read_files(&self, paths: &[String]) -> Result<Vec<(String, String)>> {
        let mut files = Vec::new();
        for path in paths {
            let full_path = self.repo_path.join(path);
            let content = std::fs::read_to_string(&full_path)
                .with_context(|| format!("Failed to read {}", full_path.display()))?;
            files.push((path.clone(), content));
        }
        Ok(files)
    }

    fn apply_edits(&self, edits: &[FileEdit]) -> Result<()> {
        for edit in edits {
            match edit {
                FileEdit::Edit { path, old, new } => {
                    let full_path = self.repo_path.join(path);
                    let content = std::fs::read_to_string(&full_path)
                        .with_context(|| format!("Cannot read {path} for editing"))?;

                    if !content.contains(old.as_str()) {
                        warn!(path, "OLD block not found in file — skipping edit");
                        continue;
                    }

                    let updated = content.replacen(old.as_str(), new.as_str(), 1);
                    std::fs::write(&full_path, updated)
                        .with_context(|| format!("Failed to write {path}"))?;
                    info!(path, "Applied edit");
                }
                FileEdit::Create { path, content } => {
                    let full_path = self.repo_path.join(path);
                    if let Some(parent) = full_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&full_path, content)
                        .with_context(|| format!("Failed to create {path}"))?;
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
