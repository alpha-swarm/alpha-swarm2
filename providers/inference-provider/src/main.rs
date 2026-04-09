//! Inference provider: NATS service wrapping Ollama HTTP API.
//!
//! Exposes `alpha-swarm:inference/completions` via NATS request-reply.
//! Components send JSON requests to `swarm.inference.*` subjects.
//!
//! Subjects:
//!   swarm.inference.chat    — chat completion
//!   swarm.inference.models  — list available models
//!   swarm.inference.health  — health check

use std::time::{Duration, Instant};

use anyhow::Result;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

/// Ollama API timeout for inference calls.
const INFERENCE_TIMEOUT: Duration = Duration::from_secs(600);
/// Ollama API timeout for metadata calls.
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct OllamaHost {
    url: String,
    priority: u32,
    client: reqwest::Client,
}

// --- Request/Response types (match WIT interface) ---

#[derive(Deserialize)]
struct ChatRequest {
    messages: Vec<Message>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    num_ctx: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatResponse {
    content: String,
    model: String,
    tokens_input: u32,
    tokens_output: u32,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct ModelInfo {
    name: String,
    family: String,
    parameter_size: String,
    context_window: u32,
    ready: bool,
}

// --- Ollama API types ---

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaMessage>,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
    #[serde(default)]
    total_duration: u64,
}

#[derive(Deserialize)]
struct OllamaMessage {
    content: String,
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelTag>,
}

#[derive(Deserialize)]
struct OllamaModelTag {
    name: String,
    #[serde(default)]
    details: OllamaModelDetails,
}

#[derive(Deserialize, Default)]
struct OllamaModelDetails {
    #[serde(default)]
    family: String,
    #[serde(default)]
    parameter_size: String,
}

#[derive(Deserialize)]
struct OllamaPsResponse {
    models: Vec<OllamaPsModel>,
}

#[derive(Deserialize)]
struct OllamaPsModel {
    name: String,
    #[serde(default)]
    context_length: u32,
}

// --- Provider implementation ---

impl OllamaHost {
    fn new(url: &str, priority: u32) -> Self {
        let client = reqwest::Client::builder()
            .timeout(INFERENCE_TIMEOUT)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            url: url.trim_end_matches('/').to_string(),
            priority,
            client,
        }
    }

    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let model = if req.model.is_empty() {
            self.best_model().await?
        } else {
            req.model.clone()
        };

        let start = Instant::now();

        let ollama_req = OllamaChatRequest {
            model: model.clone(),
            messages: req.messages.clone(),
            stream: false,
            options: Some(OllamaOptions {
                num_ctx: req.num_ctx.map(|n| n.max(4096)),
                temperature: req.temperature,
            }),
        };

        let resp = self.client
            .post(format!("{}/api/chat", self.url))
            .json(&ollama_req)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama returned {status}: {body}");
        }

        let body: OllamaChatResponse = resp.json().await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(ChatResponse {
            content: body.message.map(|m| m.content).unwrap_or_default(),
            model,
            tokens_input: body.prompt_eval_count,
            tokens_output: body.eval_count,
            duration_ms,
            error: None,
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let client = reqwest::Client::builder()
            .timeout(METADATA_TIMEOUT)
            .build()?;

        let resp: OllamaTagsResponse = client
            .get(format!("{}/api/tags", self.url))
            .send()
            .await?
            .json()
            .await?;

        // Get running models for context window info
        let ps: OllamaPsResponse = client
            .get(format!("{}/api/ps", self.url))
            .send()
            .await
            .ok()
            .and_then(|r| futures::executor::block_on(r.json()).ok())
            .unwrap_or(OllamaPsResponse { models: vec![] });

        let running: std::collections::HashMap<String, u32> = ps.models.iter()
            .map(|m| (m.name.clone(), m.context_length))
            .collect();

        Ok(resp.models.into_iter().map(|m| {
            let ctx = running.get(&m.name).copied().unwrap_or(4096);
            ModelInfo {
                name: m.name,
                family: m.details.family,
                parameter_size: m.details.parameter_size,
                context_window: ctx,
                ready: true,
            }
        }).collect())
    }

    async fn health_check(&self) -> Result<()> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        client.get(format!("{}/api/tags", self.url))
            .send().await?;
        Ok(())
    }

    async fn best_model(&self) -> Result<String> {
        let models = self.list_models().await?;
        // Prefer code models by name, then largest
        let preferred = ["qwen2.5-coder", "deepseek-coder", "codellama"];
        for prefix in preferred {
            if let Some(m) = models.iter()
                .filter(|m| m.name.starts_with(prefix))
                .max_by_key(|m| parse_size(&m.parameter_size))
            {
                return Ok(m.name.clone());
            }
        }
        models.first()
            .map(|m| m.name.clone())
            .ok_or_else(|| anyhow::anyhow!("No models available"))
    }
}

fn parse_size(s: &str) -> u64 {
    let s = s.to_lowercase().replace('b', "");
    s.trim().parse::<f64>().unwrap_or(0.0) as u64
}

// --- NATS service ---

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let nats_url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://127.0.0.1:4223".into());

    // Parse Ollama hosts from env: OLLAMA_HOSTS=http://host1:11434,http://host2:11434
    let hosts_str = std::env::var("OLLAMA_HOSTS")
        .unwrap_or_else(|_| "http://100.81.10.8:11434".into());
    let hosts: Vec<OllamaHost> = hosts_str.split(',')
        .enumerate()
        .map(|(i, url)| OllamaHost::new(url.trim(), i as u32))
        .collect();

    info!(hosts = hosts.len(), nats = %nats_url, "Inference provider starting");
    for host in &hosts {
        info!(url = %host.url, priority = host.priority, "Ollama host registered");
    }

    let client = async_nats::connect(&nats_url).await?;
    info!("Connected to NATS");

    // Subscribe to inference subjects
    let mut chat_sub = client.subscribe("swarm.inference.chat").await?;
    let mut models_sub = client.subscribe("swarm.inference.models").await?;
    let mut health_sub = client.subscribe("swarm.inference.health").await?;

    info!("Listening on swarm.inference.{{chat,models,health}}");

    loop {
        tokio::select! {
            Some(msg) = chat_sub.next() => {
                let hosts = hosts.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    let reply = msg.reply.clone();
                    let result = handle_chat(&hosts, &msg.payload).await;
                    if let Some(reply_to) = reply {
                        let resp = match result {
                            Ok(r) => serde_json::to_vec(&r).unwrap_or_default(),
                            Err(e) => {
                                error!(error = %e, "Chat failed");
                                let err = ChatResponse {
                                    content: String::new(),
                                    model: String::new(),
                                    tokens_input: 0,
                                    tokens_output: 0,
                                    duration_ms: 0,
                                    error: Some(e.to_string()),
                                };
                                serde_json::to_vec(&err).unwrap_or_default()
                            }
                        };
                        let _ = client.publish(reply_to, resp.into()).await;
                    }
                });
            }
            Some(msg) = models_sub.next() => {
                let hosts = hosts.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    if let Some(reply_to) = msg.reply {
                        let mut all_models = Vec::new();
                        for host in &hosts {
                            if let Ok(models) = host.list_models().await {
                                all_models.extend(models);
                            }
                        }
                        let resp = serde_json::to_vec(&all_models).unwrap_or_default();
                        let _ = client.publish(reply_to, resp.into()).await;
                    }
                });
            }
            Some(msg) = health_sub.next() => {
                let hosts = hosts.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    if let Some(reply_to) = msg.reply {
                        let mut statuses = Vec::new();
                        for host in &hosts {
                            let ok = host.health_check().await.is_ok();
                            statuses.push(serde_json::json!({"url": host.url, "available": ok}));
                        }
                        let resp = serde_json::to_vec(&statuses).unwrap_or_default();
                        let _ = client.publish(reply_to, resp.into()).await;
                    }
                });
            }
        }
    }
}

async fn handle_chat(hosts: &[OllamaHost], payload: &[u8]) -> Result<ChatResponse> {
    let req: ChatRequest = serde_json::from_slice(payload)?;

    // Try hosts in priority order
    for host in hosts {
        match host.chat(&req).await {
            Ok(resp) => {
                info!(
                    host = %host.url,
                    model = %resp.model,
                    tokens_in = resp.tokens_input,
                    tokens_out = resp.tokens_output,
                    duration_ms = resp.duration_ms,
                    "Chat completed"
                );
                return Ok(resp);
            }
            Err(e) => {
                warn!(host = %host.url, error = %e, "Host failed, trying next");
            }
        }
    }

    anyhow::bail!("All inference hosts failed")
}
