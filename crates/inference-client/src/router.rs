use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, bail};
use tracing::{info, warn};

use crate::backend::InferenceBackend;
use crate::types::*;

/// Routes inference requests to the best available backend.
///
/// Model is picked by complexity tier; among the backends that actually host
/// that model the request goes to the LEAST-loaded one (cross-machine load
/// balancing), with a round-robin tiebreak so even sequential calls spread
/// across the Ollama hosts (picur / csatapaci / malna) instead of all piling
/// onto the first one. Other backends remain failure fallbacks.
#[derive(Default)]
pub struct InferenceRouter {
    backends: Vec<Box<dyn InferenceBackend>>,
    /// In-flight request count per backend (index-parallel to `backends`).
    in_flight: Vec<AtomicUsize>,
    /// Round-robin cursor used to break ties between equally-loaded backends.
    rr: AtomicUsize,
}

/// RAII counter: increments a backend's in-flight count for the duration of a
/// request and decrements on drop (covers early return / error paths).
struct InFlightGuard<'a>(&'a AtomicUsize);
impl<'a> InFlightGuard<'a> {
    fn enter(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}
impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl InferenceRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_backend(mut self, backend: impl InferenceBackend + 'static) -> Self {
        self.backends.push(Box::new(backend));
        self.in_flight.push(AtomicUsize::new(0));
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

    /// List every backend's models, tagged with the backend's index, so the
    /// router knows WHICH hosts carry each model (one extra entry per host that
    /// has it). One fan-out of `list_models` across backends.
    async fn list_models_indexed(&self) -> Vec<(usize, ModelInfo)> {
        let mut out = Vec::new();
        for (idx, backend) in self.backends.iter().enumerate() {
            match backend.list_models().await {
                Ok(models) => out.extend(models.into_iter().map(|m| (idx, m))),
                Err(e) => warn!(backend = ?backend.kind(), "Failed to list models: {e}"),
            }
        }
        out
    }

    /// Pick the best model for a complexity tier.
    ///
    /// Routing strategy (fast-loop):
    ///   Simple/Medium → SMALLEST ready code model (e.g. qwen2.5-coder:14b).
    ///   Picking the largest code model cold-loads a 20–40GB model that isn't
    ///   resident (qwen3:32b / llama3.3:70b), saturates the single Ollama host,
    ///   and times out every other request — the loop wedges. Quality comes from
    ///   the cargo gate + compile-feedback iteration, not a bigger planner model.
    ///   Complex → Claude if configured, else the largest local model.
    pub async fn recommend_model(&self, complexity: Complexity) -> Result<ModelInfo> {
        Ok(self.pick_model_and_hosts(complexity).await?.0)
    }

    /// Pick the model for a tier AND the set of backend indices that host it.
    /// The index set is what `chat` load-balances across.
    async fn pick_model_and_hosts(&self, complexity: Complexity) -> Result<(ModelInfo, Vec<usize>)> {
        let indexed = self.list_models_indexed().await;
        if indexed.is_empty() {
            bail!("No models available on any backend");
        }
        let models: Vec<ModelInfo> = indexed.iter().map(|(_, m)| m.clone()).collect();

        let pick = match complexity {
            // Simple + Medium → SMALLEST ready code model (fast, resident).
            // best_ollama_by_size sorts code-models first, then smallest size,
            // so this yields qwen2.5-coder:14b over the cold 32b/70b giants.
            Complexity::Simple | Complexity::Medium => best_ollama_by_size(&models, |_| true)
                .or_else(|| any_ready(&models)),
            Complexity::Complex => find_model(&models, BackendKind::Claude, "sonnet")
                .or_else(|| find_model(&models, BackendKind::Claude, ""))
                .or_else(|| largest_ollama(&models))
                .or_else(|| any_ready(&models)),
        };

        match pick {
            Some(model) => {
                // Every backend that lists this exact model is a routing candidate.
                let hosts: Vec<usize> = indexed
                    .iter()
                    .filter(|(_, m)| m.name == model.name && m.backend == model.backend)
                    .map(|(i, _)| *i)
                    .collect();
                info!(
                    complexity = ?complexity,
                    model = %model.name,
                    backend = ?model.backend,
                    params = %model.parameter_size,
                    hosts = hosts.len(),
                    "Model selected for complexity tier"
                );
                Ok((model, hosts))
            }
            None => bail!("No suitable model for complexity {:?}", complexity),
        }
    }

    /// Order candidate backend indices least-loaded first, rotating the
    /// equal-loaded leaders by a round-robin cursor so sequential calls spread
    /// across hosts instead of always hitting index 0.
    fn order_by_load(&self, candidates: &[usize]) -> Vec<usize> {
        let mut ordered: Vec<usize> = candidates.to_vec();
        ordered.sort_by_key(|&i| self.in_flight[i].load(Ordering::Relaxed));
        if ordered.len() > 1 {
            let min = self.in_flight[ordered[0]].load(Ordering::Relaxed);
            let tied = ordered
                .iter()
                .take_while(|&&i| self.in_flight[i].load(Ordering::Relaxed) == min)
                .count();
            if tied > 1 {
                let start = self.rr.fetch_add(1, Ordering::Relaxed) % tied;
                ordered[..tied].rotate_left(start);
            }
        }
        ordered
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
        // Preferred MODEL with MULTIPLE backends (e.g. one MLX server per model
        // on its own port): route to the backend that actually HOSTS the model,
        // not just the first of its kind. Single-backend setups skip this (no
        // extra /models round-trip) and use the fast path below.
        if let Some(ref pm) = options.preferred_model
            && self.backends.len() > 1 {
                let indexed = self.list_models_indexed().await;
                if let Some(&(idx, _)) = indexed.iter().find(|(_, m)| &m.name == pm) {
                    let _guard = InFlightGuard::enter(&self.in_flight[idx]);
                    match self.backends[idx].chat(pm, messages, options).await {
                        Ok(resp) => return Ok(resp),
                        Err(e) => warn!(model = %pm, host = idx, "preferred-model backend failed: {e}"),
                    }
                }
        }

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

        // Auto-select model + the backends that host it.
        let (recommended, hosts) = self.pick_model_and_hosts(complexity).await?;

        // Route to the least-loaded host carrying the model (cross-machine load
        // balancing); fall through to the next-least-loaded on failure.
        for idx in self.order_by_load(&hosts) {
            let _guard = InFlightGuard::enter(&self.in_flight[idx]);
            match self.backends[idx].chat(&recommended.name, messages, options).await {
                Ok(resp) => return Ok(resp),
                Err(e) => warn!(
                    host = idx,
                    backend = ?recommended.backend,
                    model = %recommended.name,
                    "Backend failed for recommended model: {e}, trying next host"
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
    async fn simple_task_selects_smallest_code_model() {
        // Fast-loop: Simple/Medium pick the SMALLEST code model (resident, fast),
        // not the largest (cold-load thrash on a single Ollama host).
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:7b", "7.6B")
                    .with_model("deepseek-coder:33b", "33B")
                    .with_model("codellama:34b", "34B")
            );

        let model = router.recommend_model(Complexity::Simple).await.unwrap();
        assert_eq!(model.name, "qwen2.5-coder:7b");
    }

    #[tokio::test]
    async fn medium_task_selects_smallest_code_model() {
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:7b", "7.6B")
                    .with_model("deepseek-coder:33b", "33B")
            );

        let model = router.recommend_model(Complexity::Medium).await.unwrap();
        assert_eq!(model.name, "qwen2.5-coder:7b");
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
    async fn distributes_across_hosts_round_robin() {
        // Two Ollama hosts carrying the SAME model — sequential requests must
        // fan out (one each), not both hit host 0.
        let h0 = MockBackend::new(BackendKind::Ollama)
            .with_model("qwen2.5-coder:7b", "7.6B")
            .with_response("from-h0");
        let h1 = MockBackend::new(BackendKind::Ollama)
            .with_model("qwen2.5-coder:7b", "7.6B")
            .with_response("from-h1");
        let c0 = h0.calls.clone();
        let c1 = h1.calls.clone();

        let router = InferenceRouter::new().add_backend(h0).add_backend(h1);
        let opts = InferenceOptions::default();
        router.chat(&[ChatMessage::user("a")], Complexity::Simple, &opts).await.unwrap();
        router.chat(&[ChatMessage::user("b")], Complexity::Simple, &opts).await.unwrap();

        assert_eq!(c0.lock().unwrap().len(), 1, "host 0 should serve exactly one request");
        assert_eq!(c1.lock().unwrap().len(), 1, "host 1 should serve exactly one request");
    }

    #[tokio::test]
    async fn fails_over_to_next_host_with_same_model() {
        // Host 0 has the model but no queued response (errors); host 1 serves it.
        let h0 = MockBackend::new(BackendKind::Ollama).with_model("qwen2.5-coder:7b", "7.6B");
        let h1 = MockBackend::new(BackendKind::Ollama)
            .with_model("qwen2.5-coder:7b", "7.6B")
            .with_response("ok")
            .with_response("ok"); // enough for either rr start order
        let router = InferenceRouter::new().add_backend(h0).add_backend(h1);
        let opts = InferenceOptions::default();
        let resp = router.chat(&[ChatMessage::user("x")], Complexity::Simple, &opts).await.unwrap();
        assert_eq!(resp.content, "ok");
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
    async fn simple_prefers_small_code_model_over_giant() {
        // Fast-loop: Simple must NOT cold-load the 32B giant. qwen2.5-coder:14b
        // is a code model and smaller, so it wins over the larger qwen3:32b.
        let router = InferenceRouter::new()
            .add_backend(
                MockBackend::new(BackendKind::Ollama)
                    .with_model("qwen2.5-coder:14b", "14.8B")
                    .with_model("qwen3:32b", "32B")
            );

        let model = router.recommend_model(Complexity::Simple).await.unwrap();
        assert_eq!(model.name, "qwen2.5-coder:14b");
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
