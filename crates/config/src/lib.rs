use serde::Deserialize;

/// Central configuration for alpha-swarm.
///
/// Load order:
/// 1. Defaults (compiled in)
/// 2. Config file (alpha-swarm.toml) if present
/// 3. Environment variable overrides
///
/// WASI components use defaults only (no file/env access).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SwarmConfig {
    pub ollama: OllamaConfig,
    pub surrealdb: SurrealConfig,
    pub nats: NatsConfig,
    pub claude: ClaudeConfig,
    pub defaults: DefaultsConfig,
    pub tiers: TiersConfig,
    pub resources: ResourceConfig,
    /// SONA learning loop (trajectory distillation + retrieval-augmented planning).
    pub learning: LearningConfig,
    /// Inference providers (multiple Ollama hosts, etc.)
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

/// Default number of past proven plans injected into the planner prompt.
pub const DEFAULT_MAX_PROVEN_PLANS: usize = 3;
/// Default minimum similarity for a memory hit to be injected.
pub const DEFAULT_LEARNING_MIN_SIMILARITY: f32 = 0.5;
/// Default char budget for the injected past-plans block.
pub const DEFAULT_PROVEN_PLANS_CHAR_BUDGET: usize = 1200;

/// SONA learning loop configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LearningConfig {
    /// Master switch for trajectory recording, distillation, and retrieval.
    pub enabled: bool,
    /// Distill a pattern via the planner-tier LLM on successful runs.
    pub distill_on_success: bool,
    /// Max past proven plans injected into the planner prompt.
    pub max_proven_plans: usize,
    /// Minimum similarity for a memory hit to be injected.
    pub min_similarity: f32,
    /// Char budget for the injected past-plans block.
    pub proven_plans_char_budget: usize,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            distill_on_success: true,
            max_proven_plans: DEFAULT_MAX_PROVEN_PLANS,
            min_similarity: DEFAULT_LEARNING_MIN_SIMILARITY,
            proven_plans_char_budget: DEFAULT_PROVEN_PLANS_CHAR_BUDGET,
        }
    }
}

/// An inference provider configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    /// Provider type: "ollama", "openai", or "llamacpp"
    #[serde(rename = "type")]
    pub provider_type: String,
    /// Base URL for the provider
    pub url: String,
    /// Priority (lower = preferred)
    #[serde(default = "default_priority")]
    pub priority: u32,
    /// API key (for cloud providers like openai/together/deepinfra)
    #[serde(default)]
    pub api_key: String,
    /// Default model for this provider
    #[serde(default)]
    pub model: String,
    /// Specific models available on this host (optional, auto-discovered if empty)
    #[serde(default)]
    pub models: Vec<String>,
}

fn default_priority() -> u32 { 10 }

/// Resource usage thresholds for scheduling.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ResourceConfig {
    pub max_cpu_percent: f64,
    pub max_ram_percent: f64,
    pub check_interval_secs: u64,
    /// Max parallel sub-agents per swarm run (to avoid OOM with large models).
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: usize,
    /// Maximum depth for recursive sub-plan decomposition.
    #[serde(default = "default_max_sub_plan_depth")]
    pub max_sub_plan_depth: u32,
    /// Max retries in graph executor fix loop before escalating to full agent.
    #[serde(default = "default_max_graph_retries")]
    pub max_graph_retries: u32,
    /// Monitored hosts.
    #[serde(default = "default_hosts")]
    pub hosts: Vec<HostConfig>,
}

fn default_max_concurrent_agents() -> usize { 2 }
fn default_max_sub_plan_depth() -> u32 { 3 }
fn default_max_graph_retries() -> u32 { 3 }

#[derive(Debug, Clone, Deserialize)]
pub struct HostConfig {
    pub name: String,
    /// "local" (sysinfo) or "ollama" (query Ollama API)
    #[serde(rename = "type")]
    pub host_type: String,
    /// Ollama URL (for type=ollama)
    #[serde(default)]
    pub ollama_url: String,
}

fn default_hosts() -> Vec<HostConfig> {
    vec![
        HostConfig { name: "local".into(), host_type: "local".into(), ollama_url: String::new() },
    ]
}

/// Per-tier configuration for the agent hierarchy.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TiersConfig {
    pub orchestrator: TierConfig,
    pub agent: TierConfig,
    pub worker: TierConfig,
}

/// Configuration for a single agent tier.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TierConfig {
    /// Preferred model name for this tier.
    pub model: String,
    /// Context window size (num_ctx for Ollama).
    pub context_window: u32,
    /// Max wall-clock time in seconds.
    pub time_limit_secs: u64,
    /// Max total tokens across all iterations.
    pub token_limit: u32,
    /// Max retry iterations.
    pub max_iterations: u32,
    /// Max backoff between retries in seconds.
    pub max_backoff_secs: u64,
    /// Max files to include as context.
    pub max_context_files: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub url: String,
}

/// Default embedded SurrealDB data directory (kv-surrealkv).
/// NOTE: under /tmp for consistency with the existing DATA_DIR convention —
/// move off /tmp for reboot durability once validated.
pub const DEFAULT_SURREAL_PATH: &str = "/tmp/alpha-swarm/surrealdb/embedded";
/// Embedded ruvector ANN index path. Ephemeral cache — rebuilt from SurrealDB
/// (the system-of-record) on every daemon start, so /tmp is correct.
pub const DEFAULT_RUVECTOR_PATH: &str = "/tmp/alpha-swarm/ruvector/index";
/// Default request-reply timeout for the NATS DB bridge.
pub const DEFAULT_BRIDGE_TIMEOUT_SECS: u64 = 30;

/// How the process reaches SurrealDB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SurrealMode {
    /// In-process kv-surrealkv engine at `path` — the daemon is sole DB owner.
    #[default]
    Embedded,
    /// External SurrealDB server over WebSocket at `url` (escape hatch).
    Remote,
    /// No direct DB — consumers go through the daemon's NATS bridge
    /// (`swarm.db.>`); used by remote daemons and native tools.
    Nats,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SurrealConfig {
    pub mode: SurrealMode,
    /// Embedded engine data directory (mode = embedded).
    pub path: String,
    /// External server address (mode = remote).
    pub url: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
    /// NATS bridge request timeout (mode = nats consumers).
    pub bridge_timeout_secs: u64,
    /// Embedded ruvector ANN index path (ephemeral cache, rebuilt on startup).
    pub ruvector_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NatsConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub embed_model: String,
    pub simple_model: String,
    pub project: String,
}

/// Maps models to their roles/capabilities.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ModelRole {
    pub name: String,
    pub role: String,
    pub good_for: Vec<String>,
    pub complexity: String,
}

impl Default for ModelRole {
    fn default() -> Self {
        Self { name: String::new(), role: String::new(), good_for: vec![], complexity: "simple".into() }
    }
}

/// Known model role mappings (compiled in, overridable via config).
pub fn default_model_roles() -> Vec<ModelRole> {
    vec![
        ModelRole {
            name: "qwen2.5-coder:7b".into(),
            role: "Fast code edits".into(),
            good_for: vec!["lint fixes".into(), "rename".into(), "add simple function".into(), "fmt".into()],
            complexity: "simple".into(),
        },
        ModelRole {
            name: "deepseek-coder:33b".into(),
            role: "Medium complexity tasks".into(),
            good_for: vec!["refactoring".into(), "add features".into(), "write tests".into(), "error handling".into()],
            complexity: "medium".into(),
        },
        ModelRole {
            name: "codellama:34b".into(),
            role: "Complex reasoning".into(),
            good_for: vec!["architecture changes".into(), "algorithms".into(), "multi-file edits".into(), "debugging".into()],
            complexity: "complex".into(),
        },
        ModelRole {
            name: "claude-sonnet-4-20250514".into(),
            role: "Orchestration & planning".into(),
            good_for: vec!["task decomposition".into(), "code review".into(), "complex refactors".into(), "design decisions".into()],
            complexity: "complex".into(),
        },
    ]
}

/// Get the role info for a model by name.
pub fn model_role(name: &str) -> Option<ModelRole> {
    default_model_roles().into_iter().find(|r| name.contains(&r.name) || r.name.contains(name))
}

// --- Defaults ---


impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            max_cpu_percent: 75.0,
            max_ram_percent: 75.0,
            check_interval_secs: 10,
            max_concurrent_agents: default_max_concurrent_agents(),
            max_sub_plan_depth: default_max_sub_plan_depth(),
            max_graph_retries: default_max_graph_retries(),
            hosts: default_hosts(),
        }
    }
}

impl Default for TiersConfig {
    fn default() -> Self {
        Self {
            orchestrator: TierConfig::orchestrator(),
            agent: TierConfig::agent(),
            worker: TierConfig::worker(),
        }
    }
}

impl TierConfig {
    pub fn orchestrator() -> Self {
        Self {
            model: "codellama:34b".into(),
            context_window: 32768,
            time_limit_secs: 14400,     // 4 hours
            token_limit: 1_000_000,     // 1M tokens
            max_iterations: 20,
            max_backoff_secs: 60,
            max_context_files: 100,
        }
    }

    pub fn agent() -> Self {
        Self {
            model: "deepseek-coder:33b".into(),
            context_window: 16384,
            time_limit_secs: 1800,      // 30 min
            token_limit: 300_000,       // 300K tokens
            max_iterations: 10,
            max_backoff_secs: 30,
            max_context_files: 50,
        }
    }

    pub fn worker() -> Self {
        Self {
            model: "qwen2.5-coder:7b".into(),
            context_window: 8192,
            time_limit_secs: 300,       // 5 min
            token_limit: 100_000,       // 100K tokens
            max_iterations: 5,
            max_backoff_secs: 10,
            max_context_files: 20,
        }
    }
}

impl Default for TierConfig {
    fn default() -> Self {
        Self::agent() // sensible middle ground
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self { url: "http://100.81.10.8:11434".into() }
    }
}

impl Default for SurrealConfig {
    fn default() -> Self {
        Self {
            mode: SurrealMode::Embedded,
            path: DEFAULT_SURREAL_PATH.into(),
            url: "127.0.0.1:8001".into(),
            namespace: "alpha_swarm".into(),
            database: "swarm".into(),
            username: "root".into(),
            password: "root".into(),
            bridge_timeout_secs: DEFAULT_BRIDGE_TIMEOUT_SECS,
            ruvector_path: DEFAULT_RUVECTOR_PATH.into(),
        }
    }
}

impl Default for NatsConfig {
    fn default() -> Self {
        // Matches alpha-swarm.toml (the source of truth): the local system
        // NATS daemon ("picur", port 4223), clustered with csatapaci as
        // "alpha_swarm".
        Self { url: "nats://127.0.0.1:4223".into() }
    }
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "claude-sonnet-4-20250514".into(),
        }
    }
}

/// Default embedding model. Must be a dedicated embedding model whose output
/// dimension matches `knowledge_base::EMBED_DIM` (nomic-embed-text = 768).
/// Code models like qwen2.5-coder emit 3584-dim vectors which are incompatible
/// with the HNSW indexes.
pub const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            embed_model: DEFAULT_EMBED_MODEL.into(),
            simple_model: "qwen2.5-coder:7b".into(),
            project: "default".into(),
        }
    }
}

impl SwarmConfig {
    /// Load config: defaults → TOML file → env overrides.
    #[cfg(feature = "file")]
    pub fn load() -> Self {
        let mut config = Self::from_file().unwrap_or_default();
        config.apply_env();
        config
    }

    /// Load defaults + env only (no file). For WASI or tests.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        config.apply_env();
        config
    }

    #[cfg(feature = "file")]
    fn from_file() -> Option<Self> {
        let paths = ["alpha-swarm.toml", ".alpha-swarm.toml"];
        for path in paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                match toml::from_str::<Self>(&content) {
                    Ok(config) => {
                        eprintln!("[config] Loaded from {path} (nats={}, orchestrator.model={})", config.nats.url, config.tiers.orchestrator.model);
                        return Some(config);
                    }
                    Err(e) => eprintln!("[config] Failed to parse {path}: {e}"),
                }
            }
        }
        None
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("ALPHA_SWARM_OLLAMA_URL") { self.ollama.url = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_MODE") {
            match v.to_lowercase().as_str() {
                "embedded" => self.surrealdb.mode = SurrealMode::Embedded,
                "remote" => self.surrealdb.mode = SurrealMode::Remote,
                "nats" => self.surrealdb.mode = SurrealMode::Nats,
                _ => {}
            }
        }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_PATH") { self.surrealdb.path = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_URL") { self.surrealdb.url = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_NS") { self.surrealdb.namespace = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_DB") { self.surrealdb.database = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_USER") { self.surrealdb.username = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_PASS") { self.surrealdb.password = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_NATS_URL") { self.nats.url = v; }
        if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") { self.claude.api_key = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_CLAUDE_MODEL") { self.claude.model = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_EMBED_MODEL") { self.defaults.embed_model = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_LEARNING") {
            self.learning.enabled = matches!(v.to_lowercase().as_str(), "1" | "true" | "on");
        }
    }

    /// SurrealDB basic auth header value (base64 of user:pass).
    pub fn surrealdb_auth_header(&self) -> String {
        let credentials = format!("{}:{}", self.surrealdb.username, self.surrealdb.password);
        format!("Basic {}", base64_encode(credentials.as_bytes()))
    }

    /// SQL prefix that initializes namespace, database, and tables.
    pub fn surrealdb_init_sql(&self) -> String {
        format!(
            "USE NS {} DB {}; DEFINE TABLE IF NOT EXISTS agent_run SCHEMALESS; DEFINE TABLE IF NOT EXISTS project SCHEMALESS",
            self.surrealdb.namespace, self.surrealdb.database
        )
    }
}

fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((combined >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((combined >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 { result.push(CHARS[((combined >> 6) & 0x3F) as usize] as char); } else { result.push('='); }
        if chunk.len() > 2 { result.push(CHARS[(combined & 0x3F) as usize] as char); } else { result.push('='); }
    }
    result
}
