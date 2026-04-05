use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::info;
use wasmcloud_provider_sdk::{run_provider, LinkConfig, Provider};

use inference_client::OllamaBackend;

/// Ollama capability provider for WasmCloud.
/// Manages Ollama backends per linked component.
/// Components receive the Ollama URL in link config.
#[derive(Clone)]
pub struct OllamaProvider {
    /// Ollama URL per linked component.
    urls: Arc<RwLock<HashMap<String, String>>>,
    default_url: String,
}

impl OllamaProvider {
    pub fn new() -> Self {
        let default_url = std::env::var("ALPHA_SWARM_OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".into());
        info!(url = %default_url, "Ollama provider initialized");
        Self {
            urls: Arc::new(RwLock::new(HashMap::new())),
            default_url,
        }
    }
}

impl Provider for OllamaProvider {
    fn receive_link_config_as_target(
        &self,
        link: LinkConfig<'_>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let component = link.source_id.to_string();
        let url = link.config
            .iter()
            .find(|(k, _)| k.as_str() == "ollama_url")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| self.default_url.clone());
        let urls = Arc::clone(&self.urls);

        async move {
            // Verify Ollama is reachable
            let backend = OllamaBackend::new(&url);
            match inference_client::InferenceBackend::health_check(&backend).await {
                Ok(()) => info!(component = %component, url = %url, "Ollama reachable, link established"),
                Err(e) => tracing::warn!(component = %component, url = %url, "Ollama not reachable: {e}"),
            }
            urls.write().await.insert(component, url);
            Ok(())
        }
    }

    fn delete_link_as_target(
        &self,
        info: impl wasmcloud_provider_sdk::LinkDeleteInfo,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let source_id = info.get_source_id().to_string();
        let urls = Arc::clone(&self.urls);
        async move {
            info!(component = %source_id, "Link removed");
            urls.write().await.remove(&source_id);
            Ok(())
        }
    }

    fn shutdown(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async {
            info!("Ollama provider shutting down");
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let provider = OllamaProvider::new();

    let shutdown = run_provider(provider, "ollama-provider")
        .await
        .expect("Failed to initialize provider");

    shutdown.await;
    Ok(())
}
