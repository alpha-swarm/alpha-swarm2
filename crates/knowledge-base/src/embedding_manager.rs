//! Embedding lifecycle manager — lazy indexing with batch processing.
//!
//! - On agent start: check if project is indexed, batch-index if not
//! - On agent done: update only modified files
//! - Batching: queue embeddings, process N at a time to avoid overloading Ollama

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn, debug};

use inference_client::OllamaBackend;
use crate::KnowledgeStore;

/// Max files to embed in a single batch call.
const BATCH_SIZE: usize = 10;
/// Max chars per summary for embedding.
const SUMMARY_MAX_CHARS: usize = 500;
/// Extensions to index.
const INDEXABLE_EXTENSIONS: &[&str] = &["rs", "ts", "js", "go", "py", "md", "toml"];

pub struct EmbeddingManager {
    store: Arc<KnowledgeStore>,
    ollama: Arc<OllamaBackend>,
    embed_model: String,
}

impl EmbeddingManager {
    pub fn new(store: Arc<KnowledgeStore>, ollama: Arc<OllamaBackend>, embed_model: String) -> Self {
        Self { store, ollama, embed_model }
    }

    /// Lifecycle hook: call at agent start.
    /// Checks if project has embeddings. If not, indexes the repo.
    /// Returns the number of files indexed (0 if already indexed).
    pub async fn on_agent_start(&self, project: &str, repo_path: &Path) -> usize {
        // Check if project already has embeddings
        match self.store.find_relevant_files(project, &[0.0; 384], 1, 0.0).await {
            Ok(files) if !files.is_empty() => {
                debug!(project, existing = files.len(), "Project already indexed, skipping");
                return 0;
            }
            _ => {}
        }

        info!(project, "Project not indexed, starting batch indexing");
        self.index_project(project, repo_path).await
    }

    /// Lifecycle hook: call after agent completes.
    /// Updates embeddings only for files that were modified.
    pub async fn on_agent_done(&self, project: &str, repo_path: &Path, modified_files: &[String]) {
        if modified_files.is_empty() { return; }

        info!(project, files = modified_files.len(), "Updating embeddings for modified files");

        let summaries: Vec<(String, String)> = modified_files.iter()
            .filter_map(|f| {
                let full_path = repo_path.join(f);
                let content = std::fs::read_to_string(&full_path).ok()?;
                let summary = build_file_summary(f, &content);
                Some((f.clone(), summary))
            })
            .collect();

        self.embed_batch(project, &summaries).await;
    }

    /// Full project indexing — discovers all source files, embeds in batches.
    async fn index_project(&self, project: &str, repo_path: &Path) -> usize {
        let files = discover_indexable_files(repo_path);
        info!(project, total_files = files.len(), "Indexing project files");

        // Build summaries for all files
        let summaries: Vec<(String, String)> = files.iter()
            .filter_map(|f| {
                let full_path = repo_path.join(f);
                let content = std::fs::read_to_string(&full_path).ok()?;
                let summary = build_file_summary(f, &content);
                Some((f.clone(), summary))
            })
            .collect();

        let total = summaries.len();
        self.embed_batch(project, &summaries).await;
        info!(project, indexed = total, "Project indexing complete");
        total
    }

    /// Embed a batch of (file_path, summary) pairs.
    /// Processes BATCH_SIZE at a time to avoid overloading Ollama.
    async fn embed_batch(&self, project: &str, items: &[(String, String)]) {
        let mut indexed = 0;

        for chunk in items.chunks(BATCH_SIZE) {
            for (file_path, summary) in chunk {
                match self.ollama.embed(&self.embed_model, summary).await {
                    Ok(embedding) => {
                        match self.store.store_file_embedding(project, file_path, summary, &embedding).await {
                            Ok(_) => { indexed += 1; }
                            Err(e) => { warn!(file = %file_path, error = %e, "Failed to store embedding"); }
                        }
                    }
                    Err(e) => {
                        warn!(file = %file_path, error = %e, "Failed to embed file");
                    }
                }
            }

            // Small pause between batches to avoid Ollama overload
            if chunk.len() == BATCH_SIZE {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        debug!(indexed, total = items.len(), "Batch embedding complete");
    }
}

/// Build a summary string for embedding: filename + first lines + signatures.
fn build_file_summary(file_path: &str, content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let first_lines: String = lines.iter().take(3).cloned().collect::<Vec<_>>().join("\n");

    let signatures: Vec<&str> = lines.iter()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("pub fn ") || t.starts_with("fn ") || t.starts_with("pub struct ")
                || t.starts_with("struct ") || t.starts_with("impl ") || t.starts_with("pub trait ")
                || t.starts_with("pub enum ") || t.starts_with("pub async fn ")
        })
        .take(10)
        .cloned()
        .collect();

    let mut summary = format!("{file_path}\n{first_lines}");
    if !signatures.is_empty() {
        summary.push_str("\nSignatures: ");
        summary.push_str(&signatures.join(", "));
    }

    summary.chars().take(SUMMARY_MAX_CHARS).collect()
}

/// Discover files eligible for embedding indexing.
fn discover_indexable_files(repo_path: &Path) -> Vec<String> {
    let mut files = Vec::new();

    fn walk(dir: &Path, base: &Path, ext: &[&str], out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with('.') || name == "target" || name == "node_modules" || name == "dist" { continue; }
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

    walk(repo_path, repo_path, INDEXABLE_EXTENSIONS, &mut files);
    files.sort();
    files
}
