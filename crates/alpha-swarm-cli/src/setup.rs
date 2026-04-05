use anyhow::Result;
use tracing::info;

use inference_client::{ClaudeBackend, InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeStore;

pub fn setup_router() -> Result<InferenceRouter> {
    let mut router = InferenceRouter::new();

    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        let model = std::env::var("ALPHA_SWARM_CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".into());
        info!("Claude backend configured (model: {model})");
        router = router.add_backend(ClaudeBackend::new(api_key).with_model(model));
    }

    let ollama_url = ollama_url();
    info!("Ollama backend configured ({ollama_url})");
    router = router.add_backend(OllamaBackend::new(&ollama_url));

    Ok(router)
}

pub fn get_ollama() -> OllamaBackend {
    OllamaBackend::new(ollama_url())
}

pub async fn get_knowledge_store() -> Result<Option<KnowledgeStore>> {
    let url = std::env::var("ALPHA_SWARM_SURREALDB_URL")
        .unwrap_or_else(|_| "127.0.0.1:8000".into());
    let ns = std::env::var("ALPHA_SWARM_SURREALDB_NS")
        .unwrap_or_else(|_| "alpha_swarm".into());
    let db = std::env::var("ALPHA_SWARM_SURREALDB_DB")
        .unwrap_or_else(|_| "swarm".into());

    match KnowledgeStore::connect(&url, &ns, &db).await {
        Ok(store) => Ok(Some(store)),
        Err(e) => {
            tracing::warn!("Knowledge base unavailable: {e}");
            Ok(None)
        }
    }
}

fn ollama_url() -> String {
    std::env::var("ALPHA_SWARM_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".into())
}

pub fn discover_files(repo: &std::path::Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let extensions = ["rs", "ts", "js", "go", "py"];

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
            } else if let Some(e) = path.extension().and_then(|e| e.to_str()) {
                if ext.contains(&e) {
                    if let Ok(rel) = path.strip_prefix(base) {
                        out.push(rel.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    walk(repo, repo, &extensions, &mut files);
    files.sort();

    if files.len() > 20 {
        info!("Found {} files, taking first 20", files.len());
        files.truncate(20);
    }

    Ok(files)
}
