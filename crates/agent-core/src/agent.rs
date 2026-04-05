use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing::{info, warn};

use inference_client::{Complexity, InferenceOptions, InferenceResponse, InferenceRouter};

use crate::parser::{FileEdit, parse_edits};
use crate::prompt::build_prompt;

/// Result of an agent run.
pub struct AgentResult {
    pub edits: Vec<FileEdit>,
    pub inference_response: InferenceResponse,
    pub applied: bool,
}

/// A one-shot code modification agent.
pub struct Agent<'a> {
    router: &'a InferenceRouter,
    repo_path: PathBuf,
}

impl<'a> Agent<'a> {
    pub fn new(router: &'a InferenceRouter, repo_path: impl Into<PathBuf>) -> Self {
        Self {
            router,
            repo_path: repo_path.into(),
        }
    }

    /// Run a one-shot task: read files, call LLM, parse edits, apply them.
    pub async fn run(
        &self,
        task: &str,
        file_paths: &[String],
        complexity: Complexity,
    ) -> Result<AgentResult> {
        // 1. Read files
        let files = self.read_files(file_paths)?;
        info!(file_count = files.len(), "Read source files");

        // 2. Build prompt
        let messages = build_prompt(task, &files);

        // 3. Call inference
        let options = InferenceOptions::default();
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
                return Ok(AgentResult {
                    edits: Vec::new(),
                    inference_response: response,
                    applied: false,
                });
            }
        };

        info!(edit_count = edits.len(), "Parsed file edits");

        // 5. Apply edits
        if edits.is_empty() {
            return Ok(AgentResult {
                edits,
                inference_response: response,
                applied: false,
            });
        }

        self.apply_edits(&edits)?;

        Ok(AgentResult {
            edits,
            inference_response: response,
            applied: true,
        })
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
