use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Claude,
    Ollama,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    #[serde(alias = "Simple", alias = "SIMPLE")]
    Simple,
    #[serde(alias = "Medium", alias = "MEDIUM")]
    Medium,
    #[serde(alias = "Complex", alias = "COMPLEX")]
    Complex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into() }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InferenceOptions {
    pub preferred_backend: Option<BackendKind>,
    pub preferred_model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct InferenceResponse {
    pub content: String,
    pub model: String,
    pub backend: BackendKind,
    pub tokens_input: u32,
    pub tokens_output: u32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub backend: BackendKind,
    pub family: String,
    pub parameter_size: String,
    pub context_window: u32,
    pub ready: bool,
}
