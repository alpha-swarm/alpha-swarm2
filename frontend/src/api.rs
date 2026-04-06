use gloo_net::http::Request;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::types::*;

const BASE: &str = "/api";

async fn get<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    Request::get(&format!("{BASE}{path}"))
        .send()
        .await
        .map_err(|e| format!("Fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("JSON parse failed: {e}"))
}

async fn post<B: Serialize, R: DeserializeOwned>(path: &str, body: &B) -> Result<R, String> {
    Request::post(&format!("{BASE}{path}"))
        .json(body)
        .map_err(|e| format!("Serialize failed: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("JSON parse failed: {e}"))
}

async fn delete(path: &str) -> Result<(), String> {
    Request::delete(&format!("{BASE}{path}"))
        .send()
        .await
        .map_err(|e| format!("Fetch failed: {e}"))?;
    Ok(())
}

// --- Endpoints ---

pub async fn health() -> Result<serde_json::Value, String> {
    get("/health").await
}

pub async fn list_projects() -> Result<Vec<Project>, String> {
    get("/projects").await
}

pub async fn create_project(project: &Project) -> Result<serde_json::Value, String> {
    post("/projects", project).await
}

pub async fn delete_project(name: &str) -> Result<(), String> {
    delete(&format!("/projects/{name}")).await
}

pub async fn clear_all() -> Result<(), String> {
    delete("/clear").await
}

pub async fn get_sub_runs(parent_id: &str) -> Result<Vec<AgentRun>, String> {
    get(&format!("/sub-runs/{parent_id}")).await
}

// --- Planning API ---

pub async fn submit_plan(task: &SubmitTask) -> Result<serde_json::Value, String> {
    post("/plan", task).await
}

pub async fn get_plans(run_id: &str) -> Result<Vec<GoalPlan>, String> {
    get(&format!("/plans/{run_id}")).await
}

#[derive(serde::Serialize)]
struct FeedbackBody { feedback: String }

pub async fn send_plan_feedback(run_id: &str, feedback: &str) -> Result<serde_json::Value, String> {
    post(&format!("/plans/{run_id}/feedback"), &FeedbackBody { feedback: feedback.to_string() }).await
}

pub async fn approve_plan(run_id: &str) -> Result<serde_json::Value, String> {
    post(&format!("/plans/{run_id}/approve"), &serde_json::json!({})).await
}

pub async fn edit_plan(run_id: &str, sub_tasks: &[PlannedTask]) -> Result<serde_json::Value, String> {
    #[derive(serde::Serialize)]
    struct EditBody<'a> { sub_tasks: &'a [PlannedTask] }
    post(&format!("/plans/{run_id}/edit"), &EditBody { sub_tasks }).await
}

pub async fn list_runs(project: &str) -> Result<Vec<AgentRun>, String> {
    get(&format!("/runs/{project}")).await
}

pub async fn get_metrics(project: &str) -> Result<ProjectMetrics, String> {
    get(&format!("/metrics/{project}")).await
}

pub async fn get_goals(project: &str) -> Result<Vec<Goal>, String> {
    get(&format!("/goals/{project}")).await
}

pub async fn get_run_detail(id: &str) -> Result<AgentRun, String> {
    get(&format!("/run-detail/{id}")).await
}

pub async fn list_models() -> Result<Vec<ModelInfo>, String> {
    get("/models").await
}

pub async fn list_model_roles() -> Result<Vec<ModelRole>, String> {
    get("/model-roles").await
}

pub async fn get_resources() -> Result<Vec<ResourceSnapshot>, String> {
    get("/resources").await
}

#[derive(Serialize)]
pub struct SubmitTask {
    pub task: String,
    pub project: String,
    pub files: Vec<serde_json::Value>,
}

pub async fn submit_task(task: &SubmitTask) -> Result<serde_json::Value, String> {
    post("/run", task).await
}
