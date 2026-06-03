//! Execution lifecycle hooks.
//!
//! `ExecutionHook` is the formal extension point fired by `SwarmRunner` (per-task
//! events) and the daemon executor (run-level events). Hooks run SEQUENTIALLY in
//! registration order — correctness over speed; a hook can rely on earlier hooks
//! having finished.
//!
//! The trait lives in this crate (not agent-daemon) because the runner must fire
//! task-level hooks and agent-daemon already depends on orchestrator. Built-in
//! hooks with heavier dependencies (memory, metrics) live in agent-daemon.

use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Fired once after planning, before any task executes.
pub struct RunStartCtx<'a> {
    pub run_id: &'a str,
    pub project: &'a str,
    pub goal: &'a str,
    pub planned_task_count: usize,
}

/// Fired when a task is dispatched to an agent.
pub struct TaskStartCtx<'a> {
    pub run_id: &'a str,
    pub project: &'a str,
    pub task_id: &'a str,
    pub description: &'a str,
    pub files: &'a [String],
}

/// Fired when a task's agent finishes successfully.
pub struct TaskCompleteCtx<'a> {
    pub run_id: &'a str,
    pub project: &'a str,
    pub task_id: &'a str,
    pub description: &'a str,
    pub files: &'a [String],
    pub applied: bool,
    pub duration_ms: u64,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub model: &'a str,
}

/// Fired when a task's agent errors.
pub struct TaskFailCtx<'a> {
    pub run_id: &'a str,
    pub project: &'a str,
    pub task_id: &'a str,
    pub description: &'a str,
    pub error: &'a str,
}

/// Fired once by the daemon executor after the run result is persisted.
pub struct RunCompleteCtx<'a> {
    pub run_id: &'a str,
    pub project: &'a str,
    pub goal: &'a str,
    pub quality_passed: bool,
    pub tasks_passed: usize,
    pub tasks_failed: usize,
    pub total_duration_ms: u64,
    /// Memory pattern ids that were injected into this run's planner prompt
    /// (SONA effectiveness signal; empty when retrieval-augmented planning is off).
    pub retrieved_pattern_ids: &'a [String],
    /// Compact human-readable plan summary for trajectory recording.
    pub plan_summary: &'a str,
}

/// Lifecycle hook. All methods default to no-ops so implementors override
/// only what they need.
#[async_trait]
pub trait ExecutionHook: Send + Sync {
    fn name(&self) -> &str;
    async fn on_run_start(&self, _ctx: &RunStartCtx<'_>) {}
    async fn on_task_start(&self, _ctx: &TaskStartCtx<'_>) {}
    async fn on_task_complete(&self, _ctx: &TaskCompleteCtx<'_>) {}
    async fn on_task_fail(&self, _ctx: &TaskFailCtx<'_>) {}
    async fn on_run_complete(&self, _ctx: &RunCompleteCtx<'_>) {}
}

/// Ordered set of hooks. Emission is sequential and infallible — a hook must
/// handle its own errors; the execution path never fails because of a hook.
#[derive(Default, Clone)]
pub struct HookSet {
    hooks: Vec<Arc<dyn ExecutionHook>>,
}

impl HookSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, hook: Arc<dyn ExecutionHook>) {
        self.hooks.push(hook);
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub async fn emit_run_start(&self, ctx: &RunStartCtx<'_>) {
        for h in &self.hooks {
            h.on_run_start(ctx).await;
        }
    }

    pub async fn emit_task_start(&self, ctx: &TaskStartCtx<'_>) {
        for h in &self.hooks {
            h.on_task_start(ctx).await;
        }
    }

    pub async fn emit_task_complete(&self, ctx: &TaskCompleteCtx<'_>) {
        for h in &self.hooks {
            h.on_task_complete(ctx).await;
        }
    }

    pub async fn emit_task_fail(&self, ctx: &TaskFailCtx<'_>) {
        for h in &self.hooks {
            h.on_task_fail(ctx).await;
        }
    }

    pub async fn emit_run_complete(&self, ctx: &RunCompleteCtx<'_>) {
        for h in &self.hooks {
            h.on_run_complete(ctx).await;
        }
    }
}

/// Built-in hook: structured lifecycle logging.
pub struct TracingHook;

#[async_trait]
impl ExecutionHook for TracingHook {
    fn name(&self) -> &str {
        "tracing"
    }

    async fn on_run_start(&self, ctx: &RunStartCtx<'_>) {
        info!(run_id = ctx.run_id, project = ctx.project, tasks = ctx.planned_task_count, "hook: run started");
    }

    async fn on_task_start(&self, ctx: &TaskStartCtx<'_>) {
        info!(run_id = ctx.run_id, task_id = ctx.task_id, "hook: task started");
    }

    async fn on_task_complete(&self, ctx: &TaskCompleteCtx<'_>) {
        info!(
            run_id = ctx.run_id, task_id = ctx.task_id, applied = ctx.applied,
            duration_ms = ctx.duration_ms, "hook: task complete"
        );
    }

    async fn on_task_fail(&self, ctx: &TaskFailCtx<'_>) {
        info!(run_id = ctx.run_id, task_id = ctx.task_id, error = ctx.error, "hook: task failed");
    }

    async fn on_run_complete(&self, ctx: &RunCompleteCtx<'_>) {
        info!(
            run_id = ctx.run_id, project = ctx.project, quality_passed = ctx.quality_passed,
            passed = ctx.tasks_passed, failed = ctx.tasks_failed, "hook: run complete"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingHook {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ExecutionHook for CountingHook {
        fn name(&self) -> &str {
            "counting"
        }
        async fn on_task_complete(&self, _ctx: &TaskCompleteCtx<'_>) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn hooks_emit_sequentially() {
        let hook = Arc::new(CountingHook { calls: AtomicUsize::new(0) });
        let mut set = HookSet::new();
        set.register(hook.clone());
        set.register(hook.clone());

        let ctx = TaskCompleteCtx {
            run_id: "r1",
            project: "p",
            task_id: "t1",
            description: "d",
            files: &[],
            applied: true,
            duration_ms: 1,
            tokens_in: 0,
            tokens_out: 0,
            model: "m",
        };
        set.emit_task_complete(&ctx).await;
        assert_eq!(hook.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn empty_hookset_is_empty() {
        assert!(HookSet::new().is_empty());
    }
}
