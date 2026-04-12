//! Graph-based task executor — replaces chat loop with pre-defined templates.
//!
//! Templates: edit, create, refactor, doc.
//! LLM is called ONLY for content generation, not for sequencing.

use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result, bail};
use tracing::{info, warn};

use agent_core::{AgentResult, parse_edits, FileEdit};
use inference_client::{InferenceRouter, InferenceResponse, ChatMessage, InferenceOptions, Complexity, BackendKind};

/// Max chars of file content to include in the focused prompt.
const MAX_FILE_CONTEXT: usize = 12_000;
/// Max chars of build error to include in fix prompt.
const MAX_ERROR_CONTEXT: usize = 2_000;

/// Detect which crate a file belongs to by walking up to find Cargo.toml.
pub fn detect_crate(repo: &std::path::Path, file_path: &str) -> Option<String> {
    let full = repo.join(file_path);
    let mut dir = full.parent()?;
    loop {
        let cargo = dir.join("Cargo.toml");
        if cargo.exists() {
            let content = std::fs::read_to_string(&cargo).ok()?;
            for line in content.lines() {
                if let Some(name) = line.strip_prefix("name = ") {
                    return Some(name.trim().trim_matches('"').to_string());
                }
            }
        }
        if dir == repo { break; }
        dir = dir.parent()?;
    }
    None
}

pub struct GraphExecutor {
    router: Arc<InferenceRouter>,
    workspace: PathBuf,
    crate_name: Option<String>,
    max_retries: u32,
}

impl GraphExecutor {
    pub fn new(router: Arc<InferenceRouter>, workspace: PathBuf, crate_name: Option<String>, max_retries: u32) -> Self {
        Self { router, workspace, crate_name, max_retries }
    }

    /// Edit an existing file.
    pub async fn execute_edit(&self, task: &str, path: &str) -> Result<GraphResult> {
        info!(template = "edit", path, "Graph executor: edit");
        let content = self.read_file(path)?;
        let response = self.llm_edit(task, path, &content).await?;
        let edits = self.apply_response(path, &response.content)?;
        self.check_and_fix(task, path, &response, edits).await
    }

    /// Create a new file.
    pub async fn execute_create(&self, task: &str, path: &str) -> Result<GraphResult> {
        info!(template = "create", path, "Graph executor: create");
        let response = self.llm_create(task, path).await?;
        let edits = self.apply_response(path, &response.content)?;
        self.check_and_fix(task, path, &response, edits).await
    }

    /// Multi-file refactor.
    pub async fn execute_refactor(&self, task: &str, paths: &[String]) -> Result<GraphResult> {
        info!(template = "refactor", files = paths.len(), "Graph executor: refactor");
        let files: Vec<(String, String)> = paths.iter()
            .filter_map(|p| self.read_file(p).ok().map(|c| (p.clone(), c)))
            .collect();
        let response = self.llm_refactor(task, &files).await?;
        let edits = parse_edits(&response.content).unwrap_or_default();
        self.apply_edits(&edits)?;
        self.check_and_fix_multi(task, paths, &response, edits).await
    }

    /// Doc/config edit — no build check.
    pub async fn execute_doc(&self, task: &str, path: &str) -> Result<GraphResult> {
        info!(template = "doc", path, "Graph executor: doc (no build check)");
        let content = self.read_file(path).unwrap_or_default();
        let response = self.llm_edit(task, path, &content).await?;
        let edits = self.apply_response(path, &response.content)?;
        Ok(GraphResult { response, edits, escalated: false })
    }

    // --- Internal ---

    fn read_file(&self, path: &str) -> Result<String> {
        std::fs::read_to_string(self.workspace.join(path))
            .with_context(|| format!("Failed to read {path}"))
    }

    fn write_file(&self, path: &str, content: &str) -> Result<()> {
        let full = self.workspace.join(path);
        if let Some(parent) = full.parent() { let _ = std::fs::create_dir_all(parent); }
        std::fs::write(&full, content).with_context(|| format!("Failed to write {path}"))
    }

    fn apply_response(&self, path: &str, response: &str) -> Result<Vec<FileEdit>> {
        let edits = parse_edits(response).unwrap_or_default();
        self.apply_edits(&edits)?;
        Ok(edits)
    }

    fn apply_edits(&self, edits: &[FileEdit]) -> Result<()> {
        for edit in edits {
            match edit {
                FileEdit::Edit { path, old, new } => {
                    let content = self.read_file(path)?;
                    if let Some(updated) = content.replacen(old, new, 1).into() {
                        self.write_file(path, &updated)?;
                    }
                }
                FileEdit::Create { path, content } => self.write_file(path, content)?,
                FileEdit::Delete { path } => { let _ = std::fs::remove_file(self.workspace.join(path)); }
            }
        }
        Ok(())
    }

    fn run_check(&self) -> Result<String> {
        let cmd = if let Some(ref name) = self.crate_name {
            format!("cargo check -p {name}")
        } else { "cargo check".into() };
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .current_dir(&self.workspace)
            .output()
            .context("Failed to run check")?;
        let combined = format!("{}{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
        if output.status.success() { Ok("OK".into()) } else { bail!("{}", &combined[combined.len().saturating_sub(MAX_ERROR_CONTEXT)..]) }
    }

    async fn check_and_fix(&self, task: &str, path: &str, initial: &InferenceResponse, mut edits: Vec<FileEdit>) -> Result<GraphResult> {
        for attempt in 0..self.max_retries {
            match self.run_check() {
                Ok(_) => {
                    info!(template = "edit", attempt, "Graph: check passed");
                    return Ok(GraphResult { response: initial.clone(), edits, escalated: false });
                }
                Err(e) if attempt < self.max_retries - 1 => {
                    warn!(attempt, error = %e, "Graph: check failed, fixing");
                    let content = self.read_file(path).unwrap_or_default();
                    let fix = self.llm_fix(task, path, &content, &e.to_string()).await?;
                    let fix_edits = self.apply_response(path, &fix.content)?;
                    edits.extend(fix_edits);
                }
                Err(e) => {
                    warn!(error = %e, "Graph: max retries, escalating to agent");
                    return Ok(GraphResult { response: initial.clone(), edits, escalated: true });
                }
            }
        }
        Ok(GraphResult { response: initial.clone(), edits, escalated: true })
    }

    async fn check_and_fix_multi(&self, task: &str, paths: &[String], initial: &InferenceResponse, mut edits: Vec<FileEdit>) -> Result<GraphResult> {
        for attempt in 0..self.max_retries {
            match self.run_check() {
                Ok(_) => return Ok(GraphResult { response: initial.clone(), edits, escalated: false }),
                Err(e) if attempt < self.max_retries - 1 => {
                    warn!(attempt, "Graph refactor: check failed, fixing");
                    let files: Vec<(String, String)> = paths.iter()
                        .filter_map(|p| self.read_file(p).ok().map(|c| (p.clone(), c)))
                        .collect();
                    let fix = self.llm_fix_multi(task, &files, &e.to_string()).await?;
                    let fix_edits = parse_edits(&fix.content).unwrap_or_default();
                    self.apply_edits(&fix_edits)?;
                    edits.extend(fix_edits);
                }
                Err(_) => return Ok(GraphResult { response: initial.clone(), edits, escalated: true }),
            }
        }
        Ok(GraphResult { response: initial.clone(), edits, escalated: true })
    }

    // --- LLM prompts (focused, short) ---

    async fn llm_edit(&self, task: &str, path: &str, content: &str) -> Result<InferenceResponse> {
        let truncated: String = content.chars().take(MAX_FILE_CONTEXT).collect();
        let prompt = format!("Edit {path}:\n\n{truncated}\n\nTask: {task}\n\nOutput ONLY:\n<<<EDIT {path}\n--- OLD\nexact lines to replace\n--- NEW\nreplacement\n>>>");
        self.call_llm(&prompt).await
    }

    async fn llm_create(&self, task: &str, path: &str) -> Result<InferenceResponse> {
        let prompt = format!("Create new file {path}.\n\nTask: {task}\n\nOutput ONLY:\n<<<CREATE {path}\nfile contents\n>>>");
        self.call_llm(&prompt).await
    }

    async fn llm_refactor(&self, task: &str, files: &[(String, String)]) -> Result<InferenceResponse> {
        let mut ctx = String::new();
        for (path, content) in files {
            let trunc: String = content.chars().take(MAX_FILE_CONTEXT / files.len().max(1)).collect();
            ctx.push_str(&format!("=== {path} ===\n{trunc}\n\n"));
        }
        let prompt = format!("{ctx}Task: {task}\n\nOutput ALL edits as <<<EDIT>>> or <<<CREATE>>> blocks:");
        self.call_llm(&prompt).await
    }

    async fn llm_fix(&self, task: &str, path: &str, content: &str, error: &str) -> Result<InferenceResponse> {
        let trunc: String = content.chars().take(MAX_FILE_CONTEXT).collect();
        let prompt = format!("Fix {path}:\n\n{trunc}\n\nBuild error:\n{error}\n\nOriginal task: {task}\n\nOutput ONLY the fix:\n<<<EDIT {path}\n--- OLD\n--- NEW\n>>>");
        self.call_llm(&prompt).await
    }

    async fn llm_fix_multi(&self, task: &str, files: &[(String, String)], error: &str) -> Result<InferenceResponse> {
        let mut ctx = String::new();
        for (path, content) in files {
            let trunc: String = content.chars().take(MAX_FILE_CONTEXT / files.len().max(1)).collect();
            ctx.push_str(&format!("=== {path} ===\n{trunc}\n\n"));
        }
        let prompt = format!("{ctx}Build error:\n{error}\n\nOriginal task: {task}\n\nFix with <<<EDIT>>> blocks:");
        self.call_llm(&prompt).await
    }

    async fn call_llm(&self, prompt: &str) -> Result<InferenceResponse> {
        let messages = vec![ChatMessage::user(prompt)];
        let options = InferenceOptions::default();
        self.router.chat(&messages, Complexity::Simple, &options).await
            .context("Graph LLM call failed")
    }
}

/// Result from graph executor.
pub struct GraphResult {
    pub response: InferenceResponse,
    pub edits: Vec<FileEdit>,
    /// If true, graph failed and should escalate to full agent.
    pub escalated: bool,
}
