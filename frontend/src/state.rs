use std::collections::HashMap;
use leptos::prelude::*;
use crate::types::*;

/// Single source of truth for the entire application.
#[derive(Clone, Copy)]
pub struct AppState {
    // Data
    pub projects: RwSignal<Vec<Project>>,
    pub runs: RwSignal<HashMap<String, Vec<AgentRun>>>,
    pub goals: RwSignal<HashMap<String, Vec<Goal>>>,
    pub models: RwSignal<Vec<ModelInfo>>,
    pub model_roles: RwSignal<Vec<ModelRole>>,
    pub resources: RwSignal<Vec<ResourceSnapshot>>,
    pub metrics: RwSignal<HashMap<String, ProjectMetrics>>,

    // Live (from SSE)
    pub live_agents: RwSignal<Vec<AgentRun>>,
    pub recent_activity: RwSignal<Vec<AgentRun>>,
    pub active_count: RwSignal<u32>,

    // UI state
    pub health_online: RwSignal<bool>,
    pub selected_run: RwSignal<Option<AgentRun>>,
    pub selected_goal: RwSignal<Option<Goal>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            projects: RwSignal::new(Vec::new()),
            runs: RwSignal::new(HashMap::new()),
            goals: RwSignal::new(HashMap::new()),
            models: RwSignal::new(Vec::new()),
            model_roles: RwSignal::new(Vec::new()),
            resources: RwSignal::new(Vec::new()),
            metrics: RwSignal::new(HashMap::new()),
            live_agents: RwSignal::new(Vec::new()),
            recent_activity: RwSignal::new(Vec::new()),
            active_count: RwSignal::new(0),
            health_online: RwSignal::new(false),
            selected_run: RwSignal::new(None),
            selected_goal: RwSignal::new(None),
        }
    }

    /// Get metrics for a project (from cache).
    pub fn project_metrics(&self, project: &str) -> Option<ProjectMetrics> {
        self.metrics.get().get(project).cloned()
    }

    /// Get model role info by model name.
    pub fn model_role(&self, model_name: &str) -> Option<ModelRole> {
        self.model_roles.get().iter()
            .find(|r| model_name.contains(&r.name) || r.name.contains(model_name))
            .cloned()
    }
}
