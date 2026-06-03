//! Storage abstraction: every persistence operation the swarm performs, as a
//! trait. `KnowledgeStore` (SurrealDB via `engine::any` — embedded surrealkv,
//! remote ws, and later distributed tikv) is the production implementation;
//! tests can mock it, and a future non-SurrealDB backend only has to
//! implement this trait.
//!
//! Layering note: connection-level portability within the SurrealDB family is
//! already handled by `surrealdb::engine::any` (swap `surrealkv://` for
//! `tikv://pd:2379` + the `kv-tikv` feature). This trait is the DOMAIN-level
//! seam for everything else.

use anyhow::Result;
use async_trait::async_trait;

use crate::queries::SimilarRun;
use crate::schema::{AgentRun, GoalPlan, RunStatus};

/// Full persistence surface used by the daemon, runner, agents, memory,
/// and workflow layers.
#[async_trait]
pub trait KnowledgeBackend: Send + Sync {
    // --- Agent runs ---
    async fn store_run(&self, run: &AgentRun) -> Result<String>;
    async fn update_run(&self, id: &str, run: &AgentRun) -> Result<()>;
    async fn list_runs(&self, project: &str, status: Option<RunStatus>) -> Result<Vec<AgentRun>>;
    async fn running_agents(&self, project: &str) -> Result<Vec<AgentRun>>;
    async fn list_pending(&self) -> Result<Vec<AgentRun>>;
    async fn store_embedding(&self, id: &str, embedding: &[f32]) -> Result<()>;

    // --- Raw query escape hatches (SurrealQL today; an alternative backend
    //     must translate or reject) ---
    async fn db_query_raw(&self, query: &str) -> Result<()>;
    async fn query_json(&self, query: &str, params: serde_json::Value) -> Result<Vec<serde_json::Value>>;

    // --- Projects ---
    async fn get_project_repo(&self, project_name: &str) -> Result<Option<String>>;

    // --- Similarity over past runs ---
    async fn find_similar(&self, project: &str, embedding: &[f32], limit: usize, min_similarity: f32) -> Result<Vec<SimilarRun>>;
    async fn task_already_done(&self, project: &str, embedding: &[f32], threshold: f32) -> Result<Option<AgentRun>>;
    async fn find_past_errors(&self, project: &str, embedding: &[f32], limit: usize) -> Result<Vec<SimilarRun>>;

    // --- RAG file embeddings ---
    async fn store_file_embedding(&self, project: &str, file_path: &str, summary: &str, embedding: &[f32], content_hash: &str) -> Result<()>;
    async fn get_file_hash(&self, project: &str, file_path: &str) -> Result<Option<String>>;
    async fn get_last_indexed_commit(&self, project: &str) -> Result<Option<String>>;
    async fn set_last_indexed_commit(&self, project: &str, commit_sha: &str) -> Result<()>;
    async fn find_relevant_files(&self, project: &str, task_embedding: &[f32], limit: usize, min_similarity: f32) -> Result<Vec<(String, String, f32)>>;

    // --- Goal plans ---
    async fn store_plan(&self, plan: &GoalPlan) -> Result<String>;
    async fn get_plans(&self, run_id: &str) -> Result<Vec<GoalPlan>>;
    async fn get_latest_plan(&self, run_id: &str) -> Result<Option<GoalPlan>>;
}

#[async_trait]
impl KnowledgeBackend for crate::store::KnowledgeStore {
    async fn store_run(&self, run: &AgentRun) -> Result<String> {
        Self::store_run(self, run).await
    }
    async fn update_run(&self, id: &str, run: &AgentRun) -> Result<()> {
        Self::update_run(self, id, run).await
    }
    async fn list_runs(&self, project: &str, status: Option<RunStatus>) -> Result<Vec<AgentRun>> {
        Self::list_runs(self, project, status).await
    }
    async fn running_agents(&self, project: &str) -> Result<Vec<AgentRun>> {
        Self::running_agents(self, project).await
    }
    async fn list_pending(&self) -> Result<Vec<AgentRun>> {
        Self::list_pending(self).await
    }
    async fn store_embedding(&self, id: &str, embedding: &[f32]) -> Result<()> {
        Self::store_embedding(self, id, embedding).await
    }
    async fn db_query_raw(&self, query: &str) -> Result<()> {
        Self::db_query_raw(self, query).await
    }
    async fn query_json(&self, query: &str, params: serde_json::Value) -> Result<Vec<serde_json::Value>> {
        Self::query_json(self, query, params).await
    }
    async fn get_project_repo(&self, project_name: &str) -> Result<Option<String>> {
        Self::get_project_repo(self, project_name).await
    }
    async fn find_similar(&self, project: &str, embedding: &[f32], limit: usize, min_similarity: f32) -> Result<Vec<SimilarRun>> {
        Self::find_similar(self, project, embedding, limit, min_similarity).await
    }
    async fn task_already_done(&self, project: &str, embedding: &[f32], threshold: f32) -> Result<Option<AgentRun>> {
        Self::task_already_done(self, project, embedding, threshold).await
    }
    async fn find_past_errors(&self, project: &str, embedding: &[f32], limit: usize) -> Result<Vec<SimilarRun>> {
        Self::find_past_errors(self, project, embedding, limit).await
    }
    async fn store_file_embedding(&self, project: &str, file_path: &str, summary: &str, embedding: &[f32], content_hash: &str) -> Result<()> {
        Self::store_file_embedding(self, project, file_path, summary, embedding, content_hash).await
    }
    async fn get_file_hash(&self, project: &str, file_path: &str) -> Result<Option<String>> {
        Self::get_file_hash(self, project, file_path).await
    }
    async fn get_last_indexed_commit(&self, project: &str) -> Result<Option<String>> {
        Self::get_last_indexed_commit(self, project).await
    }
    async fn set_last_indexed_commit(&self, project: &str, commit_sha: &str) -> Result<()> {
        Self::set_last_indexed_commit(self, project, commit_sha).await
    }
    async fn find_relevant_files(&self, project: &str, task_embedding: &[f32], limit: usize, min_similarity: f32) -> Result<Vec<(String, String, f32)>> {
        Self::find_relevant_files(self, project, task_embedding, limit, min_similarity).await
    }
    async fn store_plan(&self, plan: &GoalPlan) -> Result<String> {
        Self::store_plan(self, plan).await
    }
    async fn get_plans(&self, run_id: &str) -> Result<Vec<GoalPlan>> {
        Self::get_plans(self, run_id).await
    }
    async fn get_latest_plan(&self, run_id: &str) -> Result<Option<GoalPlan>> {
        Self::get_latest_plan(self, run_id).await
    }
}
