use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use async_trait::async_trait;

use crate::backend::InferenceBackend;
use crate::types::*;

/// Mock inference backend for testing. Queues responses and records calls.
pub struct MockBackend {
    pub kind: BackendKind,
    pub models: Vec<ModelInfo>,
    pub responses: Arc<Mutex<Vec<String>>>,
    pub calls: Arc<Mutex<Vec<MockCall>>>,
    pub healthy: bool,
}

#[derive(Debug, Clone)]
pub struct MockCall {
    pub model: String,
    pub messages: Vec<ChatMessage>,
}

impl MockBackend {
    pub fn new(kind: BackendKind) -> Self {
        Self {
            kind,
            models: Vec::new(),
            responses: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
            healthy: true,
        }
    }

    pub fn with_model(mut self, name: &str, param_size: &str) -> Self {
        self.models.push(ModelInfo {
            name: name.to_string(),
            backend: self.kind,
            family: String::new(),
            parameter_size: param_size.to_string(),
            context_window: 4096,
            ready: true,
        });
        self
    }

    pub fn with_response(self, content: &str) -> Self {
        self.responses.lock().unwrap().push(content.to_string());
        self
    }

    pub fn unhealthy(mut self) -> Self {
        self.healthy = false;
        self
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn last_call(&self) -> Option<MockCall> {
        self.calls.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl InferenceBackend for MockBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    async fn health_check(&self) -> Result<()> {
        if self.healthy { Ok(()) } else { bail!("unhealthy") }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }

    async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        _options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        self.calls.lock().unwrap().push(MockCall {
            model: model.to_string(),
            messages: messages.to_vec(),
        });

        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            bail!("MockBackend: no responses queued");
        }
        let content = responses.remove(0);

        Ok(InferenceResponse {
            content,
            model: model.to_string(),
            backend: self.kind,
            tokens_input: 100,
            tokens_output: 50,
            duration_ms: 500,
        })
    }
}
