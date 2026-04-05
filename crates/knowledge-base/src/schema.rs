use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: Option<String>,
    pub project: String,
    pub task_description: String,
    pub agent_id: String,
    pub model_used: String,
    pub status: RunStatus,
    pub files_modified: Vec<String>,
    pub diff: Option<String>,
    pub error_message: Option<String>,
    pub quality_gate_passed: Option<bool>,
    pub tokens_input: u32,
    pub tokens_output: u32,
    pub duration_ms: u64,
    pub created_at: String,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
}

impl AgentRun {
    pub fn new(
        project: impl Into<String>,
        task_description: impl Into<String>,
        agent_id: impl Into<String>,
        model_used: impl Into<String>,
    ) -> Self {
        Self {
            id: None,
            project: project.into(),
            task_description: task_description.into(),
            agent_id: agent_id.into(),
            model_used: model_used.into(),
            status: RunStatus::Running,
            files_modified: Vec::new(),
            diff: None,
            error_message: None,
            quality_gate_passed: None,
            tokens_input: 0,
            tokens_output: 0,
            duration_ms: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            embedding: None,
        }
    }
}
