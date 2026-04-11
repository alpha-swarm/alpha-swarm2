use swarm_events::SwarmEvent;

const MAX_EVENTS: usize = 500;
const MAX_LOG: usize = 100;

pub struct App {
    pub events: Vec<SwarmEvent>,
    pub log_lines: Vec<LogLine>,
    pub selected: usize,
    pub expanded: bool,
    pub tab: Tab,
    pub connected: bool,
    pub tick_count: u64,
}

pub struct LogLine {
    pub timestamp: String,
    pub kind: String,
    pub message: String,
    pub is_error: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Live,
    Log,
}

impl App {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            log_lines: Vec::new(),
            selected: 0,
            expanded: false,
            tab: Tab::Live,
            connected: false,
            tick_count: 0,
        }
    }

    pub fn push_event(&mut self, event: SwarmEvent) {
        self.connected = true;
        let line = event_to_log(&event);
        self.log_lines.insert(0, line);
        if self.log_lines.len() > MAX_LOG {
            self.log_lines.truncate(MAX_LOG);
        }
        self.events.insert(0, event);
        if self.events.len() > MAX_EVENTS {
            self.events.truncate(MAX_EVENTS);
        }
    }

    pub fn scroll_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        let max = match self.tab {
            Tab::Live => self.events.len(),
            Tab::Log => self.log_lines.len(),
        };
        if self.selected + 1 < max {
            self.selected += 1;
        }
    }

    pub fn toggle_expand(&mut self) {
        self.expanded = !self.expanded;
    }

    pub fn next_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Live => Tab::Log,
            Tab::Log => Tab::Live,
        };
        self.selected = 0;
    }

    pub fn tick(&mut self) {
        self.tick_count += 1;
    }
}

fn event_to_log(event: &SwarmEvent) -> LogLine {
    let now = chrono::Utc::now().format("%H:%M:%S").to_string();
    match event {
        SwarmEvent::AgentStarted { agent_id, task, .. } => LogLine {
            timestamp: now, kind: "STARTED".into(),
            message: format!("{agent_id}: {}", &task[..task.len().min(80)]),
            is_error: false,
        },
        SwarmEvent::AgentFinished { agent_id, status, duration_ms, .. } => LogLine {
            timestamp: now, kind: "DONE".into(),
            message: format!("{agent_id}: {status} ({duration_ms}ms)"),
            is_error: false,
        },
        SwarmEvent::AgentFailed { agent_id, error, .. } => LogLine {
            timestamp: now, kind: "FAIL".into(),
            message: format!("{agent_id}: {}", &error[..error.len().min(80)]),
            is_error: true,
        },
        SwarmEvent::AgentProgress { agent_id, step, max_steps, action, .. } => LogLine {
            timestamp: now, kind: "STEP".into(),
            message: format!("{agent_id}: {step}/{max_steps} {action}"),
            is_error: false,
        },
        SwarmEvent::ToolCallExecuted { tool, is_error, duration_ms, .. } => LogLine {
            timestamp: now, kind: if *is_error { "TOOL!" } else { "TOOL" }.into(),
            message: format!("{tool} ({duration_ms}ms)"),
            is_error: *is_error,
        },
        SwarmEvent::SwarmPlanned { goal, task_count, .. } => LogLine {
            timestamp: now, kind: "PLAN".into(),
            message: format!("{task_count} tasks: {}", &goal[..goal.len().min(60)]),
            is_error: false,
        },
        SwarmEvent::SwarmCompleted { goal, quality_passed, tasks_passed, tasks_failed, .. } => LogLine {
            timestamp: now, kind: "SWARM".into(),
            message: format!("QG:{quality_passed} pass:{tasks_passed} fail:{tasks_failed} {}", &goal[..goal.len().min(40)]),
            is_error: !quality_passed,
        },
        SwarmEvent::TaskSubmitted { goal, .. } => LogLine {
            timestamp: now, kind: "NEW".into(),
            message: goal[..goal.len().min(80)].to_string(),
            is_error: false,
        },
        SwarmEvent::QualityChecked { check_name, passed, .. } => LogLine {
            timestamp: now, kind: "QG".into(),
            message: format!("{check_name}: {}", if *passed { "pass" } else { "FAIL" }),
            is_error: !passed,
        },
    }
}
