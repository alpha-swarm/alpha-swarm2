use anyhow::Result;
use tracing::info;

use inference_client::{ClaudeBackend, InferenceRouter, OllamaBackend};
use knowledge_base::KnowledgeStore;
use swarm_config::SwarmConfig;

pub fn load_config() -> SwarmConfig {
    SwarmConfig::load()
}

pub fn setup_router(config: &SwarmConfig) -> Result<InferenceRouter> {
    let mut router = InferenceRouter::new();

    if !config.claude.api_key.is_empty() {
        info!(model = %config.claude.model, "Claude backend configured");
        router = router.add_backend(
            ClaudeBackend::new(&config.claude.api_key).with_model(&config.claude.model)
        );
    }

    info!(url = %config.ollama.url, "Ollama backend configured");
    router = router.add_backend(OllamaBackend::new(&config.ollama.url));

    Ok(router)
}

pub fn get_ollama(config: &SwarmConfig) -> OllamaBackend {
    OllamaBackend::new(&config.ollama.url)
}

pub async fn get_knowledge_store(config: &SwarmConfig) -> Result<Option<KnowledgeStore>> {
    match KnowledgeStore::connect(&config.surrealdb.url, &config.surrealdb.namespace, &config.surrealdb.database).await {
        Ok(store) => Ok(Some(store)),
        Err(e) => {
            tracing::warn!("Knowledge base unavailable: {e}");
            Ok(None)
        }
    }
}

pub async fn get_event_publisher(config: &SwarmConfig) -> Result<Option<swarm_events::EventPublisher>> {
    match swarm_events::EventPublisher::connect(&config.nats.url).await {
        Ok(pub_) => Ok(Some(pub_)),
        Err(e) => {
            tracing::warn!("NATS unavailable (events disabled): {e}");
            Ok(None)
        }
    }
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
            } else if let Some(e) = path.extension().and_then(|e| e.to_str())
                && ext.contains(&e)
                && let Ok(rel) = path.strip_prefix(base) {
                    out.push(rel.to_string_lossy().to_string());
            }
        }
    }

    walk(repo, repo, &extensions, &mut files);
    files.sort();

    let max_files = swarm_config::TierConfig::agent().max_context_files;
    if files.len() > max_files {
        info!("Found {} files, taking first {}", files.len(), max_files);
        files.truncate(max_files);
    }

    Ok(files)
}
