use serde::{Deserialize, Serialize};

/// NATS subject schema:
///   alpha-swarm.{project}.agent.started
///   alpha-swarm.{project}.agent.finished
///   alpha-swarm.{project}.agent.failed
///   alpha-swarm.{project}.swarm.planned
///   alpha-swarm.{project}.swarm.completed
///   alpha-swarm.{project}.quality.checked

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SwarmEvent {
    AgentStarted {
        project: String,
        agent_id: String,
        task: String,
        model: String,
        files: Vec<String>,
        timestamp: String,
    },
    AgentFinished {
        project: String,
        agent_id: String,
        status: String,
        edits: u32,
        tokens_input: u32,
        tokens_output: u32,
        duration_ms: u64,
        model: String,
        timestamp: String,
    },
    AgentFailed {
        project: String,
        agent_id: String,
        error: String,
        model: String,
        duration_ms: u64,
        timestamp: String,
    },
    SwarmPlanned {
        project: String,
        goal: String,
        task_count: u32,
        tasks: Vec<String>,
        timestamp: String,
    },
    SwarmCompleted {
        project: String,
        goal: String,
        quality_passed: bool,
        tasks_passed: u32,
        tasks_failed: u32,
        total_duration_ms: u64,
        timestamp: String,
    },
    QualityChecked {
        project: String,
        agent_id: String,
        check_name: String,
        passed: bool,
        duration_ms: u64,
        timestamp: String,
    },
    TaskSubmitted {
        project: String,
        task_id: String,
        goal: String,
        timestamp: String,
    },
    /// Real-time tool call event — published as each tool executes.
    ToolCallExecuted {
        project: String,
        run_id: String,
        agent_id: String,
        step: u32,
        tool: String,
        params_preview: String,
        result_preview: String,
        is_error: bool,
        duration_ms: u64,
        timestamp: String,
    },
    /// Real-time progress update — published on each agent step.
    AgentProgress {
        project: String,
        run_id: String,
        agent_id: String,
        step: u32,
        max_steps: u32,
        action: String,
        result_preview: String,
        tokens_in: u32,
        tokens_out: u32,
        edits_count: u32,
        timestamp: String,
    },
    /// A persisted workflow run changed state (started/paused/resumed/completed/cancelled/failed).
    WorkflowStateChanged {
        project: String,
        workflow_id: String,
        run_id: String,
        state: String,
        timestamp: String,
    },
    /// One workflow step finished (passed or failed).
    WorkflowStepDone {
        project: String,
        workflow_id: String,
        run_id: String,
        step_id: String,
        passed: bool,
        timestamp: String,
    },
    /// A failed step triggered adaptive replanning; remaining DAG was replaced.
    WorkflowReplanned {
        project: String,
        workflow_id: String,
        run_id: String,
        failed_step_id: String,
        new_step_count: u32,
        replan_attempt: u32,
        timestamp: String,
    },
}

impl SwarmEvent {
    pub fn nats_subject(&self) -> String {
        match self {
            Self::AgentStarted { project, .. } => format!("alpha-swarm.{project}.agent.started"),
            Self::AgentFinished { project, .. } => format!("alpha-swarm.{project}.agent.finished"),
            Self::AgentFailed { project, .. } => format!("alpha-swarm.{project}.agent.failed"),
            Self::SwarmPlanned { project, .. } => format!("alpha-swarm.{project}.swarm.planned"),
            Self::SwarmCompleted { project, .. } => format!("alpha-swarm.{project}.swarm.completed"),
            Self::TaskSubmitted { project, .. } => format!("alpha-swarm.{project}.task.submitted"),
            Self::QualityChecked { project, .. } => format!("alpha-swarm.{project}.quality.checked"),
            Self::ToolCallExecuted { project, .. } => format!("alpha-swarm.{project}.tool.executed"),
            Self::AgentProgress { project, .. } => format!("alpha-swarm.{project}.agent.progress"),
            Self::WorkflowStateChanged { project, .. } => format!("alpha-swarm.{project}.workflow.state"),
            Self::WorkflowStepDone { project, .. } => format!("alpha-swarm.{project}.workflow.step"),
            Self::WorkflowReplanned { project, .. } => format!("alpha-swarm.{project}.workflow.replanned"),
        }
    }

    pub fn project(&self) -> &str {
        match self {
            Self::AgentStarted { project, .. }
            | Self::AgentFinished { project, .. }
            | Self::AgentFailed { project, .. }
            | Self::SwarmPlanned { project, .. }
            | Self::SwarmCompleted { project, .. }
            | Self::QualityChecked { project, .. }
            | Self::TaskSubmitted { project, .. }
            | Self::ToolCallExecuted { project, .. }
            | Self::AgentProgress { project, .. }
            | Self::WorkflowStateChanged { project, .. }
            | Self::WorkflowStepDone { project, .. }
            | Self::WorkflowReplanned { project, .. } => project,
        }
    }

    pub fn timestamp() -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

/// Message passed between agents during execution via NATS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub from_agent: String,
    pub to_agent: Option<String>, // None = broadcast to all agents in run
    pub run_id: String,
    pub kind: AgentMessageKind,
    pub timestamp: String,
}

/// Types of inter-agent messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentMessageKind {
    /// Notify peers about a file modification.
    FileModified { path: String, content: String },
    /// Share task completion summary for dependent tasks.
    TaskCompleted {
        task_id: String,
        summary: String,
        modified_files: Vec<String>,
    },
    /// Free-form context sharing.
    Context { key: String, value: String },
}

impl AgentMessage {
    pub fn file_modified(from: &str, run_id: &str, path: &str, content: &str) -> Self {
        Self {
            from_agent: from.into(),
            to_agent: None,
            run_id: run_id.into(),
            kind: AgentMessageKind::FileModified {
                path: path.into(),
                content: content.into(),
            },
            timestamp: Self::now(),
        }
    }

    pub fn task_completed(
        from: &str,
        run_id: &str,
        task_id: &str,
        summary: &str,
        files: Vec<String>,
    ) -> Self {
        Self {
            from_agent: from.into(),
            to_agent: None,
            run_id: run_id.into(),
            kind: AgentMessageKind::TaskCompleted {
                task_id: task_id.into(),
                summary: summary.into(),
                modified_files: files,
            },
            timestamp: Self::now(),
        }
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    /// NATS subject for this message.
    pub fn nats_subject(&self, project: &str) -> String {
        match &self.to_agent {
            Some(target) => {
                format!("alpha-swarm.{}.agent.{}.inbox", project, target)
            }
            None => {
                format!(
                    "alpha-swarm.{}.run.{}.broadcast",
                    project, self.run_id
                )
            }
        }
    }
}
