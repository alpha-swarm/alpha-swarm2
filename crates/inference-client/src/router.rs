use anyhow::{Result, bail};
use tracing::{info, warn};

use crate::backend::InferenceBackend;
use crate::types::*;

/// Routes inference requests to the best available backend.
/// Picks model by complexity tier, falls back across backends.
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
    ///
    /// Routing strategy:
    ///   Simple  → smallest Ollama code model (≤10B params)
    ///   Medium  → mid-size Ollama model (10-35B) or Claude Haiku
    ///   Complex → Claude Sonnet or largest available Ollama model
    pub async fn recommend_model(&self, complexity: Complexity) -> Result<ModelInfo> {
        let models = self.list_models().await?;
        if models.is_empty() {
            bail!("No models available on any backend");
        }

        let pick = match complexity {
            Complexity::Simple => {
                // Prefer smallest Ollama model
                best_ollama_by_size(&models, |size| size <= 10)
                    .or_else(|| any_ready(&models))
            }
            Complexity::Medium => {
                // Prefer mid-size Ollama, then Claude Haiku, then anything
                best_ollama_by_size(&models, |size| size > 10 && size <= 35)
                    .or_else(|| find_model(&models, BackendKind::Claude, "haiku"))
                    .or_else(|| best_ollama_by_size(&models, |_| true))
                    .or_else(|| any_ready(&models))
            }
            Complexity::Complex => {
                // Prefer Claude Sonnet, then largest Ollama, then anything
                find_model(&models, BackendKind::Claude, "sonnet")
                    .or_else(|| find_model(&models, BackendKind::Claude, ""))
                    .or_else(|| largest_ollama(&models))
                    .or_else(|| any_ready(&models))
            }
        };

        match pick {
            Some(model) => {
                info!(
                    complexity = ?complexity,
                    model = %model.name,
                    backend = ?model.backend,
                    params = %model.parameter_size,
                    "Model selected for complexity tier"
                );
                Ok(model)
            }
            None => bail!("No suitable model for complexity {:?}", complexity),
        }
    }

    /// Get the next-tier model for retry escalation.
    /// Returns a larger model than the one that failed.
    pub async fn escalate_model(&self, failed_model: &str, complexity: Complexity) -> Result<ModelInfo> {
        let models = self.list_models().await?;
        let failed_size = parse_param_size_b(
            models.iter()
                .find(|m| m.name == failed_model)
                .map(|m| m.parameter_size.as_str())
                .unwrap_or("0")
        );

        // Try next bigger Ollama model
        if let Some(bigger) = best_ollama_by_size(&models, |size| size > failed_size) {
            info!(from = failed_model, to = %bigger.name, "Escalating to larger model");
            return Ok(bigger);
        }

        // Try Claude
        if let Some(claude) = find_model(&models, BackendKind::Claude, "") {
            info!(from = failed_model, to = %claude.name, "Escalating to Claude");
            return Ok(claude);
        }

        bail!("No larger model available for escalation from {failed_model}")
    }

    /// Chat with automatic backend + model selection.
    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        complexity: Complexity,
        options: &InferenceOptions,
    ) -> Result<InferenceResponse> {
        // If user specified a backend/model, try that first
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

        // Auto-select via routing
        let recommended = self.recommend_model(complexity).await?;

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
                continue;
            }
            let models = match backend.list_models().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if let Some(model) = models.first() {
                match backend.chat(&model.name, messages, options).await {
                    Ok(resp) => {
                        info!(backend = ?backend.kind(), model = %model.name, "Fallback succeeded");
                        return Ok(resp);
                    }
                    Err(e) => warn!(backend = ?backend.kind(), "Fallback failed: {e}"),
                }
            }
        }

        bail!("All inference backends failed")
    }

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

// --- Model selection helpers ---

/// Parse parameter size string like "7.6B", "33B", "34B" into a numeric value in billions.
fn parse_param_size_b(s: &str) -> u32 {
    let s = s.trim().to_lowercase();
    let s = s.trim_end_matches('b');
    s.parse::<f32>().map(|v| v as u32).unwrap_or(0)
}

/// Find the best Ollama model matching a size predicate, preferring code-specialized models.
fn best_ollama_by_size(models: &[ModelInfo], size_ok: impl Fn(u32) -> bool) -> Option<ModelInfo> {
    let mut candidates: Vec<_> = models.iter()
        .filter(|m| m.backend == BackendKind::Ollama && m.ready)
        .filter(|m| size_ok(parse_param_size_b(&m.parameter_size)))
        .collect();

    // Prefer code-specialized models
    candidates.sort_by(|a, b| {
        let a_code = is_code_model(&a.name);
        let b_code = is_code_model(&b.name);
        b_code.cmp(&a_code)
            .then_with(|| parse_param_size_b(&a.parameter_size).cmp(&parse_param_size_b(&b.parameter_size)))
    });

    candidates.first().map(|m| (*m).clone())
}

/// Find the largest Ollama model available.
fn largest_ollama(models: &[ModelInfo]) -> Option<ModelInfo> {
    models.iter()
        .filter(|m| m.backend == BackendKind::Ollama && m.ready)
        .max_by_key(|m| parse_param_size_b(&m.parameter_size))
        .cloned()
}

/// Find a Claude model matching a name fragment.
fn find_model(models: &[ModelInfo], backend: BackendKind, name_contains: &str) -> Option<ModelInfo> {
    models.iter()
        .filter(|m| m.backend == backend && m.ready)
        .find(|m| name_contains.is_empty() || m.name.contains(name_contains))
        .cloned()
}

/// Any ready model.
fn any_ready(models: &[ModelInfo]) -> Option<ModelInfo> {
    models.iter().find(|m| m.ready).cloned()
}

/// Check if a model name suggests code specialization.
fn is_code_model(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("code") || n.contains("coder") || n.contains("starcoder") || n.contains("deepseek")
}
