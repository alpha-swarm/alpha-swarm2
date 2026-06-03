use serde::{Deserialize, Serialize};
use swarm_events::SwarmEvent;

const MAX_LOG: usize = 200;

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct Goal {
    pub id: String,
    pub project: String,
    pub task_description: String,
    pub status: String,
    pub model_used: String,
    pub duration_ms: u64,
    pub progress_message: Option<String>,
    pub error_message: Option<String>,
    pub diff: Option<String>,
    pub phase_timings: Option<PhaseTiming>,
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    pub files_modified: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct PhaseTiming {
    pub embedding_ms: u64,
    pub rag_ms: u64,
    pub planning_ms: u64,
    pub agent_execution_ms: u64,
    pub quality_gate_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ToolCall {
    pub tool: String,
    pub params_preview: String,
    pub result_preview: String,
    pub is_error: bool,
    pub duration_ms: u64,
}

pub struct LogLine {
    pub timestamp: String,
    pub kind: String,
    pub message: String,
    pub is_error: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tab { Goals, Log }

#[derive(Clone, Copy, PartialEq)]
pub enum InputMode { Normal, Input }

pub struct App {
    pub goals: Vec<Goal>,
    pub log_lines: Vec<LogLine>,
    pub selected: usize,
    pub expanded: Vec<String>,
    pub tab: Tab,
    pub input_mode: InputMode,
    pub input_buf: String,
    pub project: String,
    pub connected: bool,
    pub tick_count: u64,
}

impl App {
    pub fn new() -> Self {
        Self {
            goals: Vec::new(),
            log_lines: Vec::new(),
            selected: 0,
            expanded: Vec::new(),
            tab: Tab::Goals,
            input_mode: InputMode::Normal,
            input_buf: String::new(),
            project: "alpha-swarm2".into(),
            connected: false,
            tick_count: 0,
        }
    }

    pub fn set_goals(&mut self, goals: Vec<Goal>) {
        self.goals = goals;
    }

    pub fn push_event(&mut self, event: SwarmEvent) {
        self.connected = true;
        self.log_lines.insert(0, event_to_log(&event));
        if self.log_lines.len() > MAX_LOG { self.log_lines.truncate(MAX_LOG); }
        update_goal_from_event(&mut self.goals, &event);
    }

    pub fn scroll_up(&mut self) { self.selected = self.selected.saturating_sub(1); }

    pub fn scroll_down(&mut self) {
        let max = self.visible_count();
        if self.selected + 1 < max { self.selected += 1; }
    }

    pub fn visible_count(&self) -> usize {
        match self.tab { Tab::Goals => self.goals.len(), Tab::Log => self.log_lines.len() }
    }

    pub fn toggle_expand(&mut self) {
        if let Some(goal) = self.goals.get(self.selected) {
            let id = goal.id.clone();
            if self.expanded.contains(&id) {
                self.expanded.retain(|x| x != &id);
            } else {
                self.expanded.push(id);
            }
        }
    }

    pub fn is_expanded(&self, id: &str) -> bool { self.expanded.contains(&id.to_string()) }

    pub fn next_tab(&mut self) {
        self.tab = match self.tab { Tab::Goals => Tab::Log, Tab::Log => Tab::Goals };
        self.selected = 0;
    }

    pub fn tick(&mut self) { self.tick_count += 1; }
}

fn update_goal_from_event(goals: &mut [Goal], event: &SwarmEvent) {
    match event {
        SwarmEvent::AgentProgress { run_id, action, step, max_steps, .. } => {
            if let Some(g) = goals.iter_mut().find(|g| g.id == *run_id) {
                g.progress_message = Some(format!("Step {step}/{max_steps}: {action}"));
                g.status = "running".into();
            }
        }
        SwarmEvent::AgentFinished { .. } | SwarmEvent::SwarmCompleted { .. } => {
            // Will be refreshed by next SurrealDB poll
        }
        _ => {}
    }
}

fn event_to_log(event: &SwarmEvent) -> LogLine {
    let now = chrono::Utc::now().format("%H:%M:%S").to_string();
    match event {
        SwarmEvent::AgentStarted { task, .. } => LogLine { timestamp: now, kind: "START".into(), message: task[..task.len().min(70)].into(), is_error: false },
        SwarmEvent::AgentFinished { status, duration_ms, .. } => LogLine { timestamp: now, kind: "DONE".into(), message: format!("{status} ({duration_ms}ms)"), is_error: false },
        SwarmEvent::AgentFailed { error, .. } => LogLine { timestamp: now, kind: "FAIL".into(), message: error[..error.len().min(70)].into(), is_error: true },
        SwarmEvent::AgentProgress { step, max_steps, action, .. } => LogLine { timestamp: now, kind: "STEP".into(), message: format!("{step}/{max_steps} {action}"), is_error: false },
        SwarmEvent::ToolCallExecuted { tool, is_error, duration_ms, .. } => LogLine { timestamp: now, kind: "TOOL".into(), message: format!("{tool} ({duration_ms}ms)"), is_error: *is_error },
        SwarmEvent::SwarmPlanned { task_count, goal, .. } => LogLine { timestamp: now, kind: "PLAN".into(), message: format!("{task_count} tasks: {}", &goal[..goal.len().min(50)]), is_error: false },
        SwarmEvent::SwarmCompleted { quality_passed, .. } => LogLine { timestamp: now, kind: "SWARM".into(), message: format!("QG: {quality_passed}"), is_error: !quality_passed },
        SwarmEvent::TaskSubmitted { goal, .. } => LogLine { timestamp: now, kind: "NEW".into(), message: goal[..goal.len().min(70)].into(), is_error: false },
        SwarmEvent::QualityChecked { check_name, passed, .. } => LogLine { timestamp: now, kind: "QG".into(), message: format!("{check_name}: {passed}"), is_error: !passed },
        SwarmEvent::WorkflowStateChanged { run_id, state, .. } => LogLine { timestamp: now, kind: "WFLOW".into(), message: format!("{run_id} → {state}"), is_error: state == "failed" },
        SwarmEvent::WorkflowStepDone { step_id, passed, .. } => LogLine { timestamp: now, kind: "WSTEP".into(), message: format!("{step_id}: {}", if *passed { "passed" } else { "failed" }), is_error: !passed },
        SwarmEvent::WorkflowReplanned { failed_step_id, new_step_count, replan_attempt, .. } => LogLine { timestamp: now, kind: "RPLAN".into(), message: format!("after {failed_step_id}: {new_step_count} new steps (attempt {replan_attempt})"), is_error: false },
    }
}
