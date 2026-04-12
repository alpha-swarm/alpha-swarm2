//! Embedding lifecycle manager — git-diff incremental re-vectorization.

use std::path::Path;
use std::sync::Arc;
use sha2::{Sha256, Digest};
use tracing::{info, warn, debug};
use inference_client::OllamaBackend;
use crate::KnowledgeStore;

const BATCH_SIZE: usize = 10;
const SUMMARY_MAX_CHARS: usize = 500;
const INDEXABLE_EXTENSIONS: &[&str] = &["rs", "ts", "js", "go", "py", "md", "toml"];

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn git_head(repo: &Path) -> Option<String> {
    std::process::Command::new("git").args(["log", "-1", "--format=%H"]).current_dir(repo)
        .output().ok().and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.len() == 40 { Some(s) } else { None }
        })
}

fn git_diff_files(repo: &Path, from: &str, to: &str) -> Vec<String> {
    std::process::Command::new("git").args(["diff", "--name-only", &format!("{from}..{to}")])
        .current_dir(repo).output().ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect())
        .unwrap_or_default()
}

pub struct EmbeddingManager {
    store: Arc<KnowledgeStore>,
    ollama: Arc<OllamaBackend>,
    embed_model: String,
}

impl EmbeddingManager {
    pub fn new(store: Arc<KnowledgeStore>, ollama: Arc<OllamaBackend>, embed_model: String) -> Self {
        Self { store, ollama, embed_model }
    }

    pub async fn on_agent_start(&self, project: &str, repo_path: &Path) -> usize {
        let Some(head) = git_head(repo_path) else {
            warn!("Cannot get git HEAD, falling back to full index");
            return self.full_index(project, repo_path).await;
        };

        // Check if HEAD matches last indexed commit
        if let Ok(Some(last)) = self.store.get_last_indexed_commit(project).await {
            if last == head {
                debug!(project, head = &head[..8], "HEAD unchanged, skipping re-index");
                return 0;
            }
            // Incremental: only re-embed files changed since last index
            let changed = git_diff_files(repo_path, &last, &head);
            let indexable: Vec<String> = changed.into_iter()
                .filter(|f| INDEXABLE_EXTENSIONS.iter().any(|ext| f.ends_with(&format!(".{ext}"))))
                .collect();

            if indexable.is_empty() {
                info!(project, "No indexable files changed since last commit");
                let _ = self.store.set_last_indexed_commit(project, &head).await;
                return 0;
            }

            info!(project, changed = indexable.len(), "Incremental re-index (git diff)");
            let items: Vec<_> = indexable.iter().filter_map(|f| read_file_for_embed(repo_path, f)).collect();
            self.embed_batch(project, &items).await;
            let _ = self.store.set_last_indexed_commit(project, &head).await;
            return items.len();
        }

        // No previous index — full scan
        let count = self.full_index(project, repo_path).await;
        let _ = self.store.set_last_indexed_commit(project, &head).await;
        count
    }

    pub async fn on_agent_done(&self, project: &str, repo_path: &Path, modified_files: &[String]) {
        if modified_files.is_empty() { return; }
        info!(project, files = modified_files.len(), "Updating embeddings for modified files");
        let items: Vec<_> = modified_files.iter().filter_map(|f| read_file_for_embed(repo_path, f)).collect();
        self.embed_batch(project, &items).await;
        // Update commit SHA after agent changes
        if let Some(head) = git_head(repo_path) {
            let _ = self.store.set_last_indexed_commit(project, &head).await;
        }
    }

    async fn full_index(&self, project: &str, repo_path: &Path) -> usize {
        let files = discover_indexable_files(repo_path);
        info!(project, total_files = files.len(), "Full index (SHA cache)");
        let mut items = Vec::new();
        let mut skipped = 0;
        for f in &files {
            let Some((path, summary, hash)) = read_file_for_embed(repo_path, f) else { continue };
            if let Ok(Some(existing)) = self.store.get_file_hash(project, &path).await {
                if existing == hash { skipped += 1; continue; }
            }
            items.push((path, summary, hash));
        }
        info!(project, to_embed = items.len(), skipped, "SHA check done");
        if items.is_empty() { return 0; }
        let total = items.len();
        self.embed_batch(project, &items).await;
        total
    }

    async fn embed_batch(&self, project: &str, items: &[(String, String, String)]) {
        for chunk in items.chunks(BATCH_SIZE) {
            for (path, summary, hash) in chunk {
                match self.ollama.embed(&self.embed_model, summary).await {
                    Ok(embedding) => { let _ = self.store.store_file_embedding(project, path, summary, &embedding, hash).await; }
                    Err(e) => warn!(file = %path, error = %e, "Embed failed"),
                }
            }
            if chunk.len() == BATCH_SIZE { tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
        }
    }
}

fn read_file_for_embed(repo: &Path, file_path: &str) -> Option<(String, String, String)> {
    let content = std::fs::read_to_string(repo.join(file_path)).ok()?;
    let hash = sha256_hex(&content);
    let summary = build_file_summary(file_path, &content);
    Some((file_path.to_string(), summary, hash))
}

fn build_file_summary(file_path: &str, content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let first: String = lines.iter().take(3).cloned().collect::<Vec<_>>().join("\n");
    let sigs: Vec<&str> = lines.iter()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("pub fn ") || t.starts_with("fn ") || t.starts_with("pub struct ")
                || t.starts_with("struct ") || t.starts_with("impl ") || t.starts_with("pub trait ")
                || t.starts_with("pub enum ") || t.starts_with("pub async fn ")
        })
        .take(10).cloned().collect();
    let mut s = format!("{file_path}\n{first}");
    if !sigs.is_empty() { s.push_str("\nSigs: "); s.push_str(&sigs.join(", ")); }
    s.chars().take(SUMMARY_MAX_CHARS).collect()
}

fn discover_indexable_files(repo: &Path) -> Vec<String> {
    let mut files = Vec::new();
    fn walk(dir: &Path, base: &Path, ext: &[&str], out: &mut Vec<String>) {
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
    walk(repo, repo, INDEXABLE_EXTENSIONS, &mut files);
    files.sort();
    files
}
