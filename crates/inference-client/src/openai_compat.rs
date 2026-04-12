//! OpenAI-compatible inference backend.
//! Works with: Together, DeepInfra, Fireworks, Groq, OpenRouter, any OpenAI-compatible API.

use std::time::{Duration, Instant};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::backend::InferenceBackend;
use crate::types::*;

const TIMEOUT: Duration = Duration::from_secs(300);

pub struct OpenAICompatBackend {
    client: Client,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl OpenAICompatBackend {
    pub fn new(base_url: &str, api_key: &str, default_model: &str) -> Self {
        Self {
            client: Client::builder().timeout(TIMEOUT).build().unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            default_model: default_model.to_string(),
        }
    }
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize, Deserialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    model: String,
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[async_trait]
impl InferenceBackend for OpenAICompatBackend {
    fn kind(&self) -> BackendKind { BackendKind::Ollama }

    async fn health_check(&self) -> Result<()> { Ok(()) }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            name: self.default_model.clone(),
            backend: BackendKind::Ollama,
            family: "cloud".into(),
            parameter_size: "cloud".into(),
            context_window: 32768,
            ready: true,
        }])
    }

    async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        let model = if model.is_empty() { &self.default_model } else { model };
        debug!(model, messages = messages.len(), "OpenAI-compat chat");

        let oai_messages: Vec<OpenAIMessage> = messages.iter()
            .map(|m| OpenAIMessage { role: m.role.clone(), content: m.content.clone() })
            .collect();

        let request = OpenAIRequest {
            model: model.to_string(),
            messages: oai_messages,
            max_tokens: options.max_tokens,
            temperature: options.temperature,
        };

        let start = Instant::now();
        let response = self.client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send().await
            .context("Failed to send request to cloud provider")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Cloud API error {status}: {}", &body[..body.len().min(200)]);
        }

        let resp: OpenAIResponse = response.json().await
            .context("Failed to parse cloud response")?;

        let content = resp.choices.first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let (tokens_in, tokens_out) = resp.usage
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        Ok(InferenceResponse {
            content,
            model: resp.model,
            backend: BackendKind::Ollama,
            tokens_input: tokens_in,
            tokens_output: tokens_out,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

}
