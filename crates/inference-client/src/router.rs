use anyhow::{Result, bail};
use tracing::{info, warn};

use crate::backend::InferenceBackend;
use crate::types::*;

/// Routes inference requests to the best available backend.
/// Tries preferred backend first, falls back to others.
pub struct InferenceRouter {
    backends: Vec<Box<dyn InferenceBackend>>,
}

impl InferenceRouter {
    pub fn new() -> Self {
        Self { backends: Vec::new() }
    }

    pub fn add_backend(mut self, backend: impl InferenceBackend + 'static) -> Self {
        self.backends.push(Box::new(backend));
        self
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let mut all_models = Vec::new();
        for backend in &self.backends {
            match backend.list_models().await {
                Ok(models) => all_models.extend(models),
                Err(e) => warn!(backend = ?backend.kind(), "Failed to list models: {e}"),
            }
        }
        Ok(all_models)
    }

    pub async fn list_backends(&self) -> Vec<(BackendKind, bool)> {
        let mut statuses = Vec::new();
        for backend in &self.backends {
            let healthy = backend.health_check().await.is_ok();
            statuses.push((backend.kind(), healthy));
        }
        statuses
    }

    /// Pick the best model for a complexity tier.
    pub async fn recommend_model(&self, complexity: Complexity) -> Result<ModelInfo> {
        let models = self.list_models().await?;
        if models.is_empty() {
            bail!("No models available on any backend");
        }

        // Simple heuristic: Claude for complex, Ollama for simple/medium
        let preferred_backend = match complexity {
            Complexity::Simple | Complexity::Medium => BackendKind::Ollama,
            Complexity::Complex => BackendKind::Claude,
        };

        // Try preferred backend first
        if let Some(model) = models.iter().find(|m| m.backend == preferred_backend && m.ready) {
            return Ok(model.clone());
        }

        // Fallback to any available model
        models.into_iter()
            .find(|m| m.ready)
            .ok_or_else(|| anyhow::anyhow!("No ready models available"))
    }

    /// Chat with automatic backend + model selection.
    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        complexity: Complexity,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        // If user specified a backend, try that first
        if let Some(preferred) = options.preferred_backend {
            if let Some(backend) = self.backends.iter().find(|b| b.kind() == preferred) {
                let model = options.preferred_model.as_deref()
                    .unwrap_or_else(|| self.default_model_for(preferred));
                match backend.chat(model, messages, options).await {
                    Ok(resp) => return Ok(resp),
                    Err(e) => warn!(backend = ?preferred, "Preferred backend failed: {e}"),
                }
            }
        }

        // Auto-select: try recommended model
        let recommended = self.recommend_model(complexity).await?;
        info!(
            model = %recommended.name,
            backend = ?recommended.backend,
            "Routing to recommended model"
        );

        let backend = self.backends.iter()
            .find(|b| b.kind() == recommended.backend)
            .ok_or_else(|| anyhow::anyhow!("Backend {:?} not configured", recommended.backend))?;

        match backend.chat(&recommended.name, messages, options).await {
            Ok(resp) => return Ok(resp),
            Err(e) => warn!(
                backend = ?recommended.backend,
                model = %recommended.name,
                "Recommended backend failed: {e}, trying fallbacks"
            ),
        }

        // Fallback: try every other backend
        for backend in &self.backends {
            if backend.kind() == recommended.backend {
                continue; // already tried
            }
            let models = match backend.list_models().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if let Some(model) = models.first() {
                match backend.chat(&model.name, messages, options).await {
                    Ok(resp) => {
                        info!(
                            backend = ?backend.kind(),
                            model = %model.name,
                            "Fallback succeeded"
                        );
                        return Ok(resp);
                    }
                    Err(e) => warn!(backend = ?backend.kind(), "Fallback failed: {e}"),
                }
            }
        }

        bail!("All inference backends failed")
    }

    /// Generate with automatic routing (convenience wrapper over chat).
    pub async fn generate(
        &self,
        prompt: &str,
        complexity: Complexity,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        let messages = vec![ChatMessage::user(prompt)];
        self.chat(&messages, complexity, options).await
    }

    fn default_model_for(&self, backend: BackendKind) -> &str {
        match backend {
            BackendKind::Claude => "claude-sonnet-4-20250514",
            BackendKind::Ollama => "qwen2.5-coder:7b",
        }
    }
}
