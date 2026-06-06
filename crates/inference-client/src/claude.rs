use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::backend::InferenceBackend;
use crate::types::*;

/// Hard timeout for Claude API requests
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

pub struct ClaudeBackend {
    client: Client,
    api_key: String,
    default_model: String,
}

impl ClaudeBackend {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(CLAUDE_TIMEOUT)
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
            api_key: api_key.into(),
            default_model: "claude-sonnet-4-20250514".into(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    model: String,
    usage: Usage,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
}

#[async_trait]
impl InferenceBackend for ClaudeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Claude
    }

    async fn health_check(&self) -> Result<()> {
        // Quick check: list models endpoint or just verify API key format
        if self.api_key.is_empty() {
            bail!("Claude API key is empty");
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // Claude models are known statically
        Ok(vec![
            ModelInfo {
                name: "claude-sonnet-4-20250514".into(),
                backend: BackendKind::Claude,
                family: "claude-4".into(),
                parameter_size: "unknown".into(),
                context_window: 200_000,
                ready: true,
            },
            ModelInfo {
                name: "claude-haiku-4-5-20251001".into(),
                backend: BackendKind::Claude,
                family: "claude-4".into(),
                parameter_size: "unknown".into(),
                context_window: 200_000,
                ready: true,
            },
        ])
    }

    async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        let model = if model.is_empty() { &self.default_model } else { model };
        debug!(model, message_count = messages.len(), "Claude chat request");

        // Separate system message from the rest
        let system = messages.iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.clone());

        let api_messages: Vec<ApiMessage> = messages.iter()
            .filter(|m| m.role != "system")
            .map(|m| ApiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request = MessagesRequest {
            model: model.to_string(),
            max_tokens: options.max_tokens.unwrap_or(4096),
            messages: api_messages,
            system,
            temperature: options.temperature,
            stop_sequences: options.stop.clone(),
        };

        let start = Instant::now();

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Claude API")?;

        let status = response.status();
        if !status.is_success() {
            let retry_after = response.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            let body = response.text().await.unwrap_or_default();
            if status.as_u16() == 429 {
                let wait = retry_after.unwrap_or(60);
                bail!("Claude API rate limited (429) — retry after {wait}s: {body}");
            }
            if status.as_u16() == 529 {
                bail!("Claude API overloaded (529) — retry later: {body}");
            }
            bail!("Claude API error {status}: {body}");
        }

        let resp: MessagesResponse = response.json().await
            .context("Failed to parse Claude API response")?;

        let content = resp.content.into_iter()
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceResponse {
            content,
            model: resp.model,
            backend: BackendKind::Claude,
            tokens_input: resp.usage.input_tokens,
            tokens_output: resp.usage.output_tokens,
            cached_tokens: 0,
            duration_ms,
        })
    }
}
