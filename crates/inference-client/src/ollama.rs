use std::time::Instant;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::backend::InferenceBackend;
use crate::types::*;

pub struct OllamaBackend {
    client: Client,
    base_url: String,
}

impl OllamaBackend {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
        }
    }
}

// --- Ollama API types ---

#[derive(Deserialize)]
struct TagsResponse {
    models: Option<Vec<OllamaModel>>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
    #[serde(default)]
    details: OllamaModelDetails,
    #[allow(dead_code)]
    size: Option<u64>,
}

#[derive(Deserialize, Default)]
struct OllamaModelDetails {
    family: Option<String>,
    parameter_size: Option<String>,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Option<OllamaMessage>,
    model: String,
    #[serde(default)]
    eval_count: u32,
    #[serde(default)]
    prompt_eval_count: u32,
}

#[async_trait]
impl InferenceBackend for OllamaBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Ollama
    }

    async fn health_check(&self) -> Result<()> {
        let resp = self.client
            .get(&self.base_url)
            .send()
            .await
            .context("Cannot reach Ollama")?;

        if !resp.status().is_success() {
            bail!("Ollama health check failed: {}", resp.status());
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let resp: TagsResponse = self.client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .context("Failed to list Ollama models")?
            .json()
            .await
            .context("Failed to parse Ollama tags response")?;

        let models = resp.models.unwrap_or_default()
            .into_iter()
            .map(|m| ModelInfo {
                name: m.name,
                backend: BackendKind::Ollama,
                family: m.details.family.unwrap_or_default(),
                parameter_size: m.details.parameter_size.unwrap_or_default(),
                context_window: 4096, // default, would need /api/show per model for actual value
                ready: true,
            })
            .collect();

        Ok(models)
    }

    async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        debug!(model, message_count = messages.len(), "Ollama chat request");

        let ollama_messages: Vec<OllamaMessage> = messages.iter()
            .map(|m| OllamaMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request = ChatRequest {
            model: model.to_string(),
            messages: ollama_messages,
            stream: false,
            options: Some(OllamaOptions {
                temperature: options.temperature,
                num_ctx: options.max_tokens.map(|t| t.max(4096)),
                stop: options.stop.clone(),
            }),
        };

        let start = Instant::now();

        let response = self.client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Ollama API error {status}: {body}");
        }

        let resp: ChatResponse = response.json().await
            .context("Failed to parse Ollama chat response")?;

        let content = resp.message
            .map(|m| m.content)
            .unwrap_or_default();

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceResponse {
            content,
            model: resp.model,
            backend: BackendKind::Ollama,
            tokens_input: resp.prompt_eval_count,
            tokens_output: resp.eval_count,
            duration_ms,
        })
    }
}

// --- Embedding support ---

#[derive(Serialize)]
struct EmbedRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaBackend {
    /// Generate an embedding vector for the given text.
    pub async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>> {
        let request = EmbedRequest {
            model: model.to_string(),
            input: text.to_string(),
        };

        let response = self.client
            .post(format!("{}/api/embed", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send embed request to Ollama")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Ollama embed API error {status}: {body}");
        }

        let resp: EmbedResponse = response.json().await
            .context("Failed to parse Ollama embed response")?;

        resp.embeddings.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("No embedding returned"))
    }
}
