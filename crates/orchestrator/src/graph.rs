//! Graph-based task executor — replaces chat loop with pre-defined templates.
//!
//! Templates: edit, create, refactor, doc.
//! LLM is called ONLY for content generation, not for sequencing.

use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result, bail};
use tracing::{info, warn};

use agent_core::{parse_edits, FileEdit};
use inference_client::{InferenceRouter, InferenceResponse, OllamaBackend, ChatMessage, InferenceOptions, Complexity};

/// Max chars of file content to include in the focused prompt.
const MAX_FILE_CONTEXT: usize = 12_000;
/// Max chars of build error to include in fix prompt.
const MAX_ERROR_CONTEXT: usize = 2_000;

/// Frontend file extensions that should use pnpm check instead of cargo check.
const FRONTEND_EXTENSIONS: &[&str] = &[".tsx", ".ts", ".jsx", ".js", ".css", ".scss", ".html"];

/// Pick the right check command based on the file types being edited.
fn detect_check_command(workspace: &std::path::Path, files: &[String], crate_name: Option<&str>) -> String {
    let has_frontend = files.iter().any(|f| FRONTEND_EXTENSIONS.iter().any(|ext| f.ends_with(ext)));
    let has_rust = files.iter().any(|f| f.ends_with(".rs"));

    if has_frontend && !has_rust {
        // Pure frontend task — find the package.json directory
        if let Some(pkg_dir) = files.iter().find_map(|f| {
            // Walk up from file to find package.json
            let full = workspace.join(f);
            let mut dir = full.parent();
            while let Some(d) = dir {
                if d.join("package.json").exists() {
                    return d.strip_prefix(workspace).ok().map(|p| p.to_string_lossy().to_string());
                }
                if d == workspace { break; }
                dir = d.parent();
            }
            None
        }) {
            format!("cd {pkg_dir} && pnpm check 2>&1 || pnpm exec oxlint src/ 2>&1")
        } else {
            "echo OK".into() // no package.json found, skip check
        }
    } else if has_rust {
        if let Some(name) = crate_name {
            format!("cargo check -p {name}")
        } else {
            "cargo check".into()
        }
    } else {
        "echo OK".into() // doc/config files, no check needed
    }
}

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
    ollama: Option<Arc<OllamaBackend>>,
    workspace: PathBuf,
    crate_name: Option<String>,
    max_retries: u32,
    /// Model to use for graph inference (typically the orchestrator-tier model).
    /// Graph tasks are focused templates — fast code model is better than large general model.
    preferred_model: Option<String>,
}

impl GraphExecutor {
    pub fn new(router: Arc<InferenceRouter>, workspace: PathBuf, crate_name: Option<String>, max_retries: u32) -> Self {
        Self { router, ollama: None, workspace, crate_name, max_retries, preferred_model: None }
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.preferred_model = Some(model);
        self
    }

    pub fn with_ollama(mut self, ollama: Arc<OllamaBackend>) -> Self {
        self.ollama = Some(ollama);
        self
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

    fn apply_response(&self, _path: &str, response: &str) -> Result<Vec<FileEdit>> {
        let edits = parse_edits(response).unwrap_or_default();
        self.apply_edits(&edits)?;
        Ok(edits)
    }

    fn apply_edits(&self, edits: &[FileEdit]) -> Result<()> {
        for edit in edits {
            match edit {
                FileEdit::Edit { path, old, new } => {
                    let content = self.read_file(path)?;
                    // Fuzzy match (LE-normalize + trimmed line search) — a plain
                    // replacen miss would rewrite the file unchanged yet look
                    // applied. A real miss bails → runner escalates to the full
                    // agent instead of "passing" on an unchanged file.
                    match agent_core::fuzzy_replace(&content, old, new) {
                        Some(updated) => self.write_file(path, &updated)?,
                        None => bail!("edit OLD block not found in {path}"),
                    }
                }
                FileEdit::Create { path, content } => self.write_file(path, content)?,
                FileEdit::Delete { path } => { let _ = std::fs::remove_file(self.workspace.join(path)); }
            }
        }
        Ok(())
    }

    fn run_check(&self, files: &[String]) -> Result<String> {
        let cmd = detect_check_command(&self.workspace, files, self.crate_name.as_deref());
        info!(cmd = %cmd, "Graph: running check");
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .current_dir(&self.workspace)
            .output()
            .context("Failed to run check")?;
        let combined = format!("{}{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
        if output.status.success() { Ok("OK".into()) } else { bail!("{}", &combined[combined.len().saturating_sub(MAX_ERROR_CONTEXT)..]) }
    }

    async fn check_and_fix(&self, task: &str, path: &str, initial: &InferenceResponse, mut edits: Vec<FileEdit>) -> Result<GraphResult> {
        let files = vec![path.to_string()];
        for attempt in 0..self.max_retries {
            match self.run_check(&files) {
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
            match self.run_check(paths) {
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
        let options = InferenceOptions {
            preferred_model: self.preferred_model.clone(),
            preferred_backend: Some(inference_client::BackendKind::Ollama),
            ..Default::default()
        };

        // Non-streaming via router (uses preferred_model from options). Streaming
        // was removed: against a single busy Ollama host the streaming send
        // failed repeatedly ("Failed to send streaming request") and each retry
        // then hung on the fallback for up to the client timeout, wedging the
        // loop for tens of minutes. The daemon only needs the final result + the
        // gate verdict, not token-by-token output, so non-streaming is simpler
        // and far more robust.
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
