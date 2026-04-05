use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{info, warn};
use wasmcloud_provider_sdk::{
    get_connection, run_provider, serve_provider_exports, LinkConfig, Provider,
};

use inference_client::{
    ChatMessage, Complexity, InferenceBackend, InferenceOptions, OllamaBackend,
};

mod bindings {
    wit_bindgen_wrpc::generate!({
        world: "ollama-provider",
        path: "wit",
        generate_all,
    });
}

use bindings::exports::alpha_swarm::ollama_provider::completions;

/// Ollama capability provider for WasmCloud.
#[derive(Clone)]
pub struct OllamaProvider {
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
        let url = link
            .config
            .iter()
            .find(|(k, _)| k.as_str() == "ollama_url")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| self.default_url.clone());
        let urls = Arc::clone(&self.urls);

        async move {
            let backend = OllamaBackend::new(&url);
            match backend.health_check().await {
                Ok(()) => info!(component = %component, url = %url, "Ollama reachable"),
                Err(e) => warn!(component = %component, url = %url, "Ollama check: {e}"),
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

/// Handler for the completions interface.
#[derive(Clone)]
pub struct OllamaHandler {
    default_url: String,
}

impl completions::Handler<Option<wasmcloud_provider_sdk::Context>> for OllamaHandler {
    async fn chat(
        &self,
        _ctx: Option<wasmcloud_provider_sdk::Context>,
        messages: Vec<completions::ChatMessage>,
        complexity: completions::Complexity,
        _options: Option<completions::InferenceOptions>,
    ) -> anyhow::Result<Result<completions::InferenceResponse, completions::SwarmError>> {
        let backend = OllamaBackend::new(&self.default_url);

        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let complexity = match complexity {
            completions::Complexity::Simple => Complexity::Simple,
            completions::Complexity::Medium => Complexity::Medium,
            completions::Complexity::Complex => Complexity::Complex,
        };

        // Pick model based on complexity
        let model = match complexity {
            Complexity::Simple => "qwen2.5-coder:7b",
            Complexity::Medium => "deepseek-coder:33b",
            Complexity::Complex => "codellama:34b",
        };

        let options = InferenceOptions::default();
        match backend.chat(model, &chat_messages, &options).await {
            Ok(resp) => Ok(Ok(completions::InferenceResponse {
                content: resp.content,
                model: resp.model,
                backend: completions::BackendKind::Ollama,
                tokens_input: resp.tokens_input,
                tokens_output: resp.tokens_output,
                duration_ms: resp.duration_ms,
            })),
            Err(e) => Ok(Err(completions::SwarmError::InferenceFailed(e.to_string()))),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let provider = OllamaProvider::new();
    let default_url = provider.default_url.clone();

    let shutdown = run_provider(provider, "ollama-provider")
        .await
        .expect("Failed to initialize provider");

    let connection = get_connection();
    let wrpc_client = connection
        .get_wrpc_client(connection.provider_key())
        .await?;

    let handler = OllamaHandler { default_url };

    // Serve wRPC exports in parallel with provider lifecycle
    tokio::select! {
        _ = shutdown => {
            info!("Provider shutdown requested");
        }
        result = serve_provider_exports(
            &wrpc_client,
            handler,
            futures::future::pending::<()>(),
            bindings::serve,
        ) => {
            if let Err(e) = result {
                tracing::error!("wRPC serve error: {e}");
            }
        }
    }

    Ok(())
}
