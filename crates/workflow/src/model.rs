//! WASI-portable workflow types and pure state-machine logic.
//!
//! The DAG is stored as an embedded `steps` array inside one `workflow_run`
//! document (NOT `RELATE` graph edges): one document = one atomic checkpoint
//! update, matching the existing `goal_plan` persistence pattern and what the
//! NATS bridge consumers can parse.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use swarm_orchestrator::SubTask;

/// Version stamp written into every persisted workflow document.
pub const WORKFLOW_SCHEMA_VERSION: u32 = 1;
/// Max adaptive replans per workflow run before the run fails.
pub const MAX_REPLAN_ATTEMPTS: u32 = 3;
/// Default per-step attempt budget before the step is declared failed.
pub const DEFAULT_STEP_MAX_ATTEMPTS: u32 = 2;
/// Id prefix applied to steps spliced in by replan round `n`: `r{n}-{id}`.
pub const REPLAN_STEP_ID_PREFIX: &str = "r";

/// Workflow run lifecycle: `created→running↔paused→completed/cancelled/failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    Created,
    Running,
    Paused,
    Completed,
    Cancelled,
    Failed,
}

impl WorkflowState {
    /// Pure transition predicate. Terminal states absorb.
    pub fn can_transition(self, to: WorkflowState) -> bool {
        use WorkflowState::*;
        matches!(
            (self, to),
            (Created, Running)
                | (Created, Cancelled)
                | (Running, Paused)
                | (Running, Completed)
                | (Running, Cancelled)
                | (Running, Failed)
                | (Paused, Running)
                | (Paused, Cancelled)
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, WorkflowState::Completed | WorkflowState::Cancelled | WorkflowState::Failed)
    }
}

/// Per-step lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
}

/// How a step executes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepKind {
    /// Full chat-loop agent (the runner's standard path; direct-edit and
    /// graph-template fast paths still apply based on the embedded SubTask).
    AgentTask,
    /// Pause the workflow until a human approves (maps to the existing
    /// planned→approved gate). The paused run holds NO execution lock.
    HumanApproval,
}

/// GOAP-flavored, locally-checkable condition. No LLM-evaluated conditions —
/// local-model reliability rules them out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    FileExists { path: String },
    FilesChanged { paths: Vec<String> },
}

/// One node of the workflow DAG. `task.depends_on` holds the edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Step id; equals `task.id` for planner-generated runs.
    pub id: String,
    pub kind: StepKind,
    /// The executable payload — reused verbatim from the planner.
    pub task: SubTask,
    pub state: StepState,
    pub attempts: u32,
    pub max_attempts: u32,
    #[serde(default)]
    pub preconditions: Vec<Condition>,
    #[serde(default)]
    pub effects: Vec<Condition>,
    #[serde(default)]
    pub error: Option<String>,
    /// Link to the agent_run row that executed this step, when known.
    #[serde(default)]
    pub agent_run_id: Option<String>,
}

impl WorkflowStep {
    pub fn from_task(task: SubTask) -> Self {
        Self {
            id: task.id.clone(),
            kind: StepKind::AgentTask,
            task,
            state: StepState::Pending,
            attempts: 0,
            max_attempts: DEFAULT_STEP_MAX_ATTEMPTS,
            preconditions: Vec::new(),
            effects: Vec::new(),
            error: None,
            agent_run_id: None,
        }
    }

    /// A step is ready when every dependency id is in `completed`.
    pub fn is_ready(&self, completed: &HashSet<String>) -> bool {
        self.state == StepState::Pending
            && self.task.depends_on.iter().all(|d| completed.contains(d))
    }
}

/// Reusable, versioned workflow template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub version: u32,
    pub description: String,
    /// Template steps (state = Pending). Instantiation clones these.
    pub steps: Vec<WorkflowStep>,
    pub schema_version: u32,
    pub created_at: String,
}

/// A concrete, persisted workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    #[serde(default)]
    pub id: Option<String>,
    /// Template provenance; `None` for ad-hoc planner-generated runs.
    #[serde(default)]
    pub def_name: Option<String>,
    #[serde(default)]
    pub def_version: Option<u32>,
    pub project: String,
    pub goal: String,
    /// The agent_run id this workflow executes (the existing task_id).
    pub run_id: String,
    pub state: WorkflowState,
    pub steps: Vec<WorkflowStep>,
    pub replan_attempts: u32,
    pub schema_version: u32,
    pub created_at: String,
    pub updated_at: String,
}

impl WorkflowRun {
    /// Build an ad-hoc workflow run from planner output.
    pub fn from_tasks(
        project: impl Into<String>,
        goal: impl Into<String>,
        run_id: impl Into<String>,
        tasks: Vec<SubTask>,
        now_rfc3339: impl Into<String>,
    ) -> Self {
        let now = now_rfc3339.into();
        Self {
            id: None,
            def_name: None,
            def_version: None,
            project: project.into(),
            goal: goal.into(),
            run_id: run_id.into(),
            state: WorkflowState::Created,
            steps: tasks.into_iter().map(WorkflowStep::from_task).collect(),
            replan_attempts: 0,
            schema_version: WORKFLOW_SCHEMA_VERSION,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Ids of steps in `Passed` state.
    pub fn completed_ids(&self) -> HashSet<String> {
        self.steps.iter()
            .filter(|s| s.state == StepState::Passed)
            .map(|s| s.id.clone())
            .collect()
    }

    /// Pending steps whose dependencies are all satisfied.
    pub fn ready_steps(&self) -> Vec<&WorkflowStep> {
        let done = self.completed_ids();
        self.steps.iter().filter(|s| s.is_ready(&done)).collect()
    }

    /// Remaining (pending) tasks, cloned for execution.
    pub fn pending_tasks(&self) -> Vec<SubTask> {
        self.steps.iter()
            .filter(|s| s.state == StepState::Pending)
            .map(|s| s.task.clone())
            .collect()
    }

    /// First failed step, if any.
    pub fn failed_step(&self) -> Option<&WorkflowStep> {
        self.steps.iter().find(|s| s.state == StepState::Failed)
    }

    pub fn all_passed(&self) -> bool {
        self.steps.iter().all(|s| matches!(s.state, StepState::Passed | StepState::Skipped))
    }

    /// Apply a state transition, enforcing the state machine. Returns false
    /// (and leaves state unchanged) on an illegal transition.
    pub fn transition(&mut self, to: WorkflowState, now_rfc3339: impl Into<String>) -> bool {
        if self.state.can_transition(to) {
            self.state = to;
            self.updated_at = now_rfc3339.into();
            true
        } else {
            false
        }
    }

    /// Replace the failed step and every step that (transitively) depends on it
    /// with replan output. New step ids get the `r{round}-` prefix to avoid
    /// collisions. Returns the number of steps removed.
    pub fn splice_replan(
        &mut self,
        failed_id: &str,
        mut new_tasks: Vec<SubTask>,
        now_rfc3339: impl Into<String>,
    ) -> usize {
        // Collect the failed step + transitive dependents.
        let mut doomed: HashSet<String> = HashSet::new();
        doomed.insert(failed_id.to_string());
        loop {
            let before = doomed.len();
            for s in &self.steps {
                if s.task.depends_on.iter().any(|d| doomed.contains(d)) {
                    doomed.insert(s.id.clone());
                }
            }
            if doomed.len() == before {
                break;
            }
        }
        let removed = self.steps.iter().filter(|s| doomed.contains(&s.id)).count();
        self.steps.retain(|s| !doomed.contains(&s.id));

        self.replan_attempts += 1;
        let prefix = format!("{}{}-", REPLAN_STEP_ID_PREFIX, self.replan_attempts);

        // Re-id new tasks (and their internal depends_on references).
        let new_ids: HashSet<String> = new_tasks.iter().map(|t| t.id.clone()).collect();
        for t in &mut new_tasks {
            t.id = format!("{prefix}{}", t.id);
            for dep in &mut t.depends_on {
                if new_ids.contains(dep) {
                    *dep = format!("{prefix}{dep}");
                } else if doomed.contains(dep) {
                    // Dependency on a removed step is meaningless now.
                    *dep = String::new();
                }
            }
            t.depends_on.retain(|d| !d.is_empty());
        }

        self.steps.extend(new_tasks.into_iter().map(WorkflowStep::from_task));
        self.updated_at = now_rfc3339.into();
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inference_client::Complexity;

    fn task(id: &str, deps: &[&str]) -> SubTask {
        SubTask {
            id: id.into(),
            description: format!("do {id}"),
            files: vec!["src/lib.rs".into()],
            complexity: Complexity::Simple,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            edit: None,
            template: None,
        }
    }

    #[test]
    fn state_machine_transitions() {
        use WorkflowState::*;
        assert!(Created.can_transition(Running));
        assert!(Running.can_transition(Paused));
        assert!(Paused.can_transition(Running));
        assert!(Running.can_transition(Completed));
        assert!(Running.can_transition(Cancelled));
        assert!(Paused.can_transition(Cancelled));
        // Illegal
        assert!(!Completed.can_transition(Running));
        assert!(!Cancelled.can_transition(Running));
        assert!(!Failed.can_transition(Running));
        assert!(!Created.can_transition(Paused));
    }

    #[test]
    fn readiness_follows_dag() {
        let wf = WorkflowRun::from_tasks(
            "p", "g", "run-1",
            vec![task("a", &[]), task("b", &["a"]), task("c", &["a", "b"])],
            "2026-01-01T00:00:00Z",
        );
        let ready: Vec<&str> = wf.ready_steps().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ready, vec!["a"]);
    }

    #[test]
    fn readiness_after_completion() {
        let mut wf = WorkflowRun::from_tasks(
            "p", "g", "run-1",
            vec![task("a", &[]), task("b", &["a"])],
            "2026-01-01T00:00:00Z",
        );
        wf.steps[0].state = StepState::Passed;
        let ready: Vec<&str> = wf.ready_steps().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ready, vec!["b"]);
    }

    #[test]
    fn splice_replan_removes_failed_and_dependents() {
        let mut wf = WorkflowRun::from_tasks(
            "p", "g", "run-1",
            vec![task("a", &[]), task("b", &["a"]), task("c", &["b"])],
            "2026-01-01T00:00:00Z",
        );
        wf.steps[0].state = StepState::Passed;
        wf.steps[1].state = StepState::Failed;

        let removed = wf.splice_replan("b", vec![task("fix", &[])], "2026-01-01T00:01:00Z");
        assert_eq!(removed, 2); // b and c
        assert_eq!(wf.replan_attempts, 1);
        let ids: Vec<&str> = wf.steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "r1-fix"]);
    }

    #[test]
    fn splice_replan_rewrites_internal_deps() {
        let mut wf = WorkflowRun::from_tasks(
            "p", "g", "run-1",
            vec![task("a", &[])],
            "2026-01-01T00:00:00Z",
        );
        wf.steps[0].state = StepState::Failed;
        wf.splice_replan("a", vec![task("x", &[]), task("y", &["x"])], "t");
        let y = wf.steps.iter().find(|s| s.id == "r1-y").unwrap();
        assert_eq!(y.task.depends_on, vec!["r1-x".to_string()]);
    }

    #[test]
    fn illegal_transition_is_rejected() {
        let mut wf = WorkflowRun::from_tasks("p", "g", "r", vec![task("a", &[])], "t");
        assert!(wf.transition(WorkflowState::Running, "t2"));
        assert!(wf.transition(WorkflowState::Completed, "t3"));
        assert!(!wf.transition(WorkflowState::Running, "t4"));
        assert_eq!(wf.state, WorkflowState::Completed);
    }
}
