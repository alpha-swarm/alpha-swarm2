use serde::Deserialize;

/// Central configuration for alpha-swarm.
///
/// Load order:
/// 1. Defaults (compiled in)
/// 2. Config file (alpha-swarm.toml) if present
/// 3. Environment variable overrides
///
/// WASI components use defaults only (no file/env access).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SwarmConfig {
    pub ollama: OllamaConfig,
    pub surrealdb: SurrealConfig,
    pub nats: NatsConfig,
    pub claude: ClaudeConfig,
    pub defaults: DefaultsConfig,
    pub tiers: TiersConfig,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SurrealConfig {
    pub url: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
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

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            ollama: OllamaConfig::default(),
            surrealdb: SurrealConfig::default(),
            nats: NatsConfig::default(),
            claude: ClaudeConfig::default(),
            defaults: DefaultsConfig::default(),
            tiers: TiersConfig::default(),
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
            url: "127.0.0.1:8001".into(),
            namespace: "alpha_swarm".into(),
            database: "swarm".into(),
            username: "root".into(),
            password: "root".into(),
        }
    }
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self { url: "nats://127.0.0.1:4222".into() }
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

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            embed_model: "qwen2.5-coder:7b".into(),
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
                if let Ok(config) = toml::from_str(&content) {
                    return Some(config);
                }
            }
        }
        None
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("ALPHA_SWARM_OLLAMA_URL") { self.ollama.url = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_URL") { self.surrealdb.url = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_NS") { self.surrealdb.namespace = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_DB") { self.surrealdb.database = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_USER") { self.surrealdb.username = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_SURREALDB_PASS") { self.surrealdb.password = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_NATS_URL") { self.nats.url = v; }
        if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") { self.claude.api_key = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_CLAUDE_MODEL") { self.claude.model = v; }
        if let Ok(v) = std::env::var("ALPHA_SWARM_EMBED_MODEL") { self.defaults.embed_model = v; }
    }

    /// SurrealDB basic auth header value (base64 of user:pass).
    pub fn surrealdb_auth_header(&self) -> String {
        use std::io::Write;
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
