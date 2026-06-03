//! Seed reusable workflow definitions mirroring the graph-executor templates
//! (`edit` / `create` / `refactor` / `doc` in `swarm_orchestrator::graph`).
//!
//! Seeding is idempotent (UPSERT keyed by name+version). A run instantiated
//! from a def clones its steps and fills description/files from the goal.

use anyhow::Result;
use inference_client::Complexity;
use swarm_orchestrator::SubTask;

use crate::model::{StepKind, WorkflowDef, WorkflowStep, WORKFLOW_SCHEMA_VERSION};
use crate::repo::WorkflowRepo;

/// Current version of all seed templates.
const SEED_TEMPLATE_VERSION: u32 = 1;

fn template_step(template: &str, description: &str) -> WorkflowStep {
    WorkflowStep {
        id: format!("{template}-1"),
        kind: StepKind::AgentTask,
        task: SubTask {
            id: format!("{template}-1"),
            description: description.into(),
            files: Vec::new(), // filled at instantiation
            complexity: Complexity::Simple,
            depends_on: Vec::new(),
            edit: None,
            template: Some(template.into()),
        },
        state: crate::model::StepState::Pending,
        attempts: 0,
        max_attempts: crate::model::DEFAULT_STEP_MAX_ATTEMPTS,
        preconditions: Vec::new(),
        effects: Vec::new(),
        error: None,
        agent_run_id: None,
    }
}

fn seed_defs(now: &str) -> Vec<WorkflowDef> {
    let mk = |name: &str, description: &str, steps: Vec<WorkflowStep>| WorkflowDef {
        id: None,
        name: name.into(),
        version: SEED_TEMPLATE_VERSION,
        description: description.into(),
        steps,
        schema_version: WORKFLOW_SCHEMA_VERSION,
        created_at: now.to_string(),
    };

    vec![
        mk("edit-file", "Modify one existing code file via the graph executor",
            vec![template_step("edit", "Edit the target file")]),
        mk("create-file", "Create a new file via the graph executor (modest expectations for local models)",
            vec![template_step("create", "Create the target file")]),
        mk("refactor", "Modify multiple files together via the graph executor",
            vec![template_step("refactor", "Refactor the target files")]),
        mk("doc", "Edit docs/config files (no build check) via the graph executor",
            vec![template_step("doc", "Update the documentation")]),
    ]
}

/// Idempotently seed the built-in workflow definitions.
pub async fn seed_templates(repo: &WorkflowRepo) -> Result<usize> {
    let now = chrono::Utc::now().to_rfc3339();
    let defs = seed_defs(&now);
    let count = defs.len();
    for def in &defs {
        repo.store_def(def).await?;
    }
    Ok(count)
}
