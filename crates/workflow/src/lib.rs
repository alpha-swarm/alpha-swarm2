//! Persisted, resumable workflow engine for alpha-swarm.
//!
//! Capabilities (ruflo-like): DAG workflows with a durable state machine
//! (`created→running↔paused→completed/cancelled/failed`), reusable templates,
//! and adaptive replanning on step failure.

// WASI-portable model (serde only).
pub mod model;
pub use model::{
    Condition, StepKind, StepState, WorkflowDef, WorkflowRun, WorkflowState, WorkflowStep,
    DEFAULT_STEP_MAX_ATTEMPTS, MAX_REPLAN_ATTEMPTS, WORKFLOW_SCHEMA_VERSION,
};

// Native-only: engine, persistence, templates.
#[cfg(feature = "native")]
mod engine;
#[cfg(feature = "native")]
mod repo;
#[cfg(feature = "native")]
mod templates;

#[cfg(feature = "native")]
pub use engine::{EngineContext, EngineOutcome, WorkflowEngine};
#[cfg(feature = "native")]
pub use repo::WorkflowRepo;
#[cfg(feature = "native")]
pub use templates::seed_templates;
