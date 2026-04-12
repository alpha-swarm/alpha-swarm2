use anyhow::{Result, bail};
use tracing::{info, warn};

use crate::backend::InferenceBackend;
use crate::types::*;

/// Routes inference requests to the best available backend.
/// Picks model by complexity tier, falls back across backends.
#[derive(Default)]
pub struct InferenceRouter {
    backends: Vec<Box<dyn InferenceBackend>>,
}

impl InferenceRouter {
    pub fn new() -> Self {
        Self::default()
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
    ///   All tiers → prefer largest code model for quality.
    ///   Running locally with 96GB RAM — no reason to be stingy.
    ///   Simple tasks still get routed to the largest model because
    ///   the quality improvement is worth the extra seconds.
    pub async fn recommend_model(&self, complexity: Complexity) -> Result<ModelInfo> {
        let models = self.list_models().await?;
        if models.is_empty() {
            bail!("No models available on any backend");
        }

        let pick = match complexity {
            Complexity::Simple => {
                // Use largest code model — quality over speed
                largest_ollama(&models)
                    .or_else(|| best_ollama_by_size(&models, |_| true))
                    .or_else(|| any_ready(&models))
            }
            Complexity::Medium => {
                // Largest Ollama code model
                largest_ollama(&models)
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
    pub async fn escalate_model(&self, failed_model: &str, _complexity: Complexity) -> Result<ModelInfo> {
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
        if let Some(preferred) = options.preferred_backend
            && let Some(backend) = self.backends.iter().find(|b| b.kind() == preferred) {
                let model = options.preferred_model.as_deref()
                    .unwrap_or_else(|| self.default_model_for(preferred));
                match backend.chat(model, messages, options).await {
                    Ok(resp) => return Ok(resp),
                    Err(e) => warn!(backend = ?preferred, "Preferred backend failed: {e}"),
                }
        }

        // Auto-select via routing
        let recommended = self.recommend_model(complexity).await?;

        // Try the recommended model on ALL backends that have it (not just the first)
        for backend in self.backends.iter().filter(|b| b.kind() == recommended.backend) {
            match backend.chat(&recommended.name, messages, options).await {
                Ok(resp) => return Ok(resp),
                Err(e) => warn!(
                    backend = ?recommended.backend,
                    model = %recommended.name,
                    "Backend failed for recommended model: {e}, trying next"
                ),
            }
        }

        // Fallback: try every backend with its best available model
        for backend in &self.backends {
            let models = match backend.list_models().await {
                Ok(m) if !m.is_empty() => m,
                _ => continue,
            };
            let model = largest_ollama(&models)
                .or_else(|| models.into_iter().next());
            if let Some(model) = model {
                if model.name == recommended.name { continue; } // Already tried above
                match backend.chat(&model.name, messages, options).await {
                    Ok(resp) => {
                        info!(backend = ?backend.kind(), model = %model.name, "Fallback succeeded");
                        return Ok(resp);
                    }
                    Err(e) => warn!(backend = ?backend.kind(), model = %model.name, "Fallback failed: {e}"),
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

/// Preferred model name prefixes — models known to follow structured output well.
/// Checked in order; first match wins over a larger but less capable model.
const PREFERRED_CODE_MODELS: &[&str] = &["deepseek-coder", "qwen2.5-coder", "qwen"];

/// Find the best Ollama model: prefer known-good code models, then largest by params.
/// Among preferred models, picks the largest across ALL prefixes (not first match).
fn largest_ollama(models: &[ModelInfo]) -> Option<ModelInfo> {
    let ollama_ready: Vec<&ModelInfo> = models.iter()
        .filter(|m| m.backend == BackendKind::Ollama && m.ready)
        .collect();

    // Collect the largest model from each preferred prefix, then pick the overall biggest
    let best_preferred = PREFERRED_CODE_MODELS.iter()
        .filter_map(|prefix| {
            ollama_ready.iter()
                .filter(|m| m.name.starts_with(prefix))
                .max_by_key(|m| parse_param_size_b(&m.parameter_size))
                .copied()
        })
        .max_by_key(|m| parse_param_size_b(&m.parameter_size));

    if let Some(m) = best_preferred {
        return Some(m.clone());
    }

    // Fallback: largest available
    ollama_ready.iter()
        .max_by_key(|m| parse_param_size_b(&m.parameter_size))
        .map(|m| (*m).clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockBackend;

    #[tokio::test]
    async fn simple_task_selects_largest_code_model() {
        // All tiers now prefer largest code model for quality
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:7b", "7.6B")
                    .with_model("deepseek-coder:33b", "33B")
                    .with_model("codellama:34b", "34B")
            );

        let model = router.recommend_model(Complexity::Simple).await.unwrap();
        assert_eq!(model.name, "deepseek-coder:33b");
    }

    #[tokio::test]
    async fn medium_task_selects_midsize_model() {
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:7b", "7.6B")
                    .with_model("deepseek-coder:33b", "33B")
            );

        let model = router.recommend_model(Complexity::Medium).await.unwrap();
        assert_eq!(model.name, "deepseek-coder:33b");
    }

    #[tokio::test]
    async fn complex_task_prefers_claude() {
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Claude)
                    .with_model("claude-sonnet-4-20250514", "unknown")
            )
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("codellama:34b", "34B")
            );

        let model = router.recommend_model(Complexity::Complex).await.unwrap();
        assert_eq!(model.backend, BackendKind::Claude);
    }

    #[tokio::test]
    async fn complex_falls_back_to_largest_code_model() {
        // Without Claude, picks largest preferred code model
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:7b", "7.6B")
                    .with_model("codellama:34b", "34B")
            );

        let model = router.recommend_model(Complexity::Complex).await.unwrap();
        // qwen2.5-coder is in PREFERRED_CODE_MODELS, codellama is not
        assert_eq!(model.name, "qwen2.5-coder:7b");
    }

    #[tokio::test]
    async fn no_models_returns_error() {
        let router = InferenceRouter::new()
            .add_backend(MockBackend::new(BackendKind::Ollama));

        assert!(router.recommend_model(Complexity::Simple).await.is_err());
    }

    #[tokio::test]
    async fn fallback_on_backend_failure() {
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Claude)
                    .with_model("claude-sonnet-4-20250514", "unknown")
                    .unhealthy()
            )
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:7b", "7.6B")
                    .with_response("hello")
            );

        let opts = InferenceOptions {
            preferred_backend: Some(BackendKind::Claude),
            ..Default::default()
        };
        let resp = router.chat(&[ChatMessage::user("test")], Complexity::Simple, &opts).await.unwrap();
        assert_eq!(resp.backend, BackendKind::Ollama);
    }

    #[tokio::test]
    async fn escalate_returns_larger_model() {
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:7b", "7.6B")
                    .with_model("deepseek-coder:33b", "33B")
            );

        let bigger = router.escalate_model("qwen2.5-coder:7b", Complexity::Simple).await.unwrap();
        assert_eq!(bigger.name, "deepseek-coder:33b");
    }

    #[tokio::test]
    async fn code_model_preferred_over_general() {
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("llama3:8b", "8B")
                    .with_model("qwen2.5-coder:7b", "7.6B")
            );

        let model = router.recommend_model(Complexity::Simple).await.unwrap();
        assert_eq!(model.name, "qwen2.5-coder:7b");
    }

    #[test]
    fn parse_param_size() {
        assert_eq!(parse_param_size_b("7.6B"), 7);
        assert_eq!(parse_param_size_b("33B"), 33);
        assert_eq!(parse_param_size_b("unknown"), 0);
        assert_eq!(parse_param_size_b(""), 0);
    }

    #[tokio::test]
    async fn larger_preferred_model_wins_across_prefixes() {
        // qwen3:32b should beat qwen2.5-coder:14b — both match PREFERRED_CODE_MODELS
        // ("qwen2.5-coder" and "qwen" prefixes), but 32B > 14B
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:14b", "14.8B")
                    .with_model("qwen3:32b", "32B")
            );

        let model = router.recommend_model(Complexity::Simple).await.unwrap();
        assert_eq!(model.name, "qwen3:32b");
    }

    #[test]
    fn code_model_detection() {
        assert!(is_code_model("qwen2.5-coder:7b"));
        assert!(is_code_model("deepseek-coder:33b"));
        assert!(is_code_model("codellama:34b"));
        assert!(is_code_model("starcoder2:15b"));
        assert!(!is_code_model("llama3:8b"));
        assert!(!is_code_model("mistral:7b"));
    }
}
