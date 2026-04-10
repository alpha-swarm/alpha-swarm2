use serde::{Deserialize, Serialize};
use crate::schema::{AgentRun, RunStatus};

/// A past run found via vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarRun {
    pub project: String,
    pub task_description: String,
    pub agent_id: String,
    pub model_used: String,
    pub status: RunStatus,
    pub files_modified: Vec<String>,
    pub diff: Option<String>,
    pub error_message: Option<String>,
    pub quality_gate_passed: Option<bool>,
    pub similarity: f32,
}

impl SimilarRun {
    pub fn into_run(self) -> AgentRun {
        AgentRun {
            id: None,
            project: self.project,
            task_description: self.task_description,
            agent_id: self.agent_id,
            model_used: self.model_used,
            status: self.status,
            files_modified: self.files_modified,
            diff: self.diff,
            error_message: self.error_message,
            quality_gate_passed: self.quality_gate_passed,
            tokens_input: 0,
            tokens_output: 0,
            duration_ms: 0,
            created_at: String::new(),
            embedding: None,
            prompt_sent: None,
            response_text: None,
            attempts: Vec::new(),
            started_at: None,
            last_activity_at: None,
            parent_run_id: None,
            progress_message: None,
            tool_calls: Vec::new(),
            phase_timings: None,
        }
    }
}
