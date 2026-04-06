use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Project {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub repo_url: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentRun {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub task_description: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub model_used: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tokens_input: u32,
    #[serde(default)]
    pub tokens_output: u32,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub quality_gate_passed: Option<bool>,
    #[serde(default)]
    pub files_modified: Vec<String>,
    #[serde(default)]
    pub diff: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Goal {
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub passed: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub running: u32,
    #[serde(default)]
    pub agents: Vec<AgentRun>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectMetrics {
    #[serde(default)]
    pub total_runs: u32,
    #[serde(default)]
    pub passed: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub pass_rate: f64,
    #[serde(default)]
    pub total_tokens_output: u64,
    #[serde(default)]
    pub avg_duration_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default, alias = "model")]
    pub model_name: String,
    #[serde(default)]
    pub parameter_size: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub details: Option<ModelDetails>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelDetails {
    #[serde(default)]
    pub parameter_size: String,
    #[serde(default)]
    pub family: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelRole {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub good_for: Vec<String>,
    #[serde(default)]
    pub complexity: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub fuel: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourceSnapshot {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub host_type: String,
    #[serde(default)]
    pub cpu_percent: f64,
    #[serde(default)]
    pub ram_total_mb: u64,
    #[serde(default)]
    pub ram_used_mb: u64,
    #[serde(default)]
    pub ram_percent: f64,
    #[serde(default)]
    pub disk_total_gb: f64,
    #[serde(default)]
    pub disk_free_gb: f64,
    #[serde(default)]
    pub disk_percent: f64,
    #[serde(default)]
    pub ollama_models: Vec<OllamaModel>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OllamaModel {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size_mb: u64,
    #[serde(default)]
    pub expires_at: String,
}

impl AgentRun {
    pub fn display_name(&self) -> &str {
        if !self.task_description.is_empty() { &self.task_description }
        else { &self.agent_id }
    }

    pub fn duration_secs(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }

    pub fn is_running(&self) -> bool { self.status == "running" }
    pub fn is_passed(&self) -> bool { self.status == "passed" }
    pub fn is_failed(&self) -> bool { self.status == "failed" }
    pub fn is_pending(&self) -> bool { self.status == "pending" }

    pub fn has_pr(&self) -> bool {
        self.diff.as_ref().is_some_and(|d| d.starts_with("PR: "))
    }

    pub fn pr_url(&self) -> Option<&str> {
        self.diff.as_ref()
            .filter(|d| d.starts_with("PR: "))
            .map(|d| &d[4..])
    }
}

impl ModelInfo {
    pub fn display_name(&self) -> &str {
        if !self.name.is_empty() { &self.name }
        else { &self.model_name }
    }

    pub fn display_params(&self) -> String {
        if !self.parameter_size.is_empty() {
            self.parameter_size.clone()
        } else if let Some(d) = &self.details {
            d.parameter_size.clone()
        } else {
            String::new()
        }
    }

    pub fn display_family(&self) -> String {
        if !self.family.is_empty() {
            self.family.clone()
        } else if let Some(d) = &self.details {
            d.family.clone()
        } else {
            String::new()
        }
    }
}
