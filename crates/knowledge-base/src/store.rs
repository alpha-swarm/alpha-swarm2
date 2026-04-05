use anyhow::{Context, Result};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use tracing::info;

use crate::schema::*;
use crate::queries::SimilarRun;

pub struct KnowledgeStore {
    db: Surreal<Client>,
}

impl KnowledgeStore {
    /// Connect to SurrealDB and initialize the schema.
    pub async fn connect(url: &str, namespace: &str, database: &str) -> Result<Self> {
        let db = Surreal::new::<Ws>(url)
            .await
            .context("Failed to connect to SurrealDB")?;

        db.signin(surrealdb::opt::auth::Root {
            username: "root".to_string(),
            password: "root".to_string(),
        })
        .await
        .context("Failed to authenticate with SurrealDB")?;

        db.use_ns(namespace).use_db(database)
            .await
            .context("Failed to select namespace/database")?;

        let store = Self { db };
        store.init_schema().await?;

        info!(url, namespace, database, "Connected to SurrealDB");
        Ok(store)
    }

    async fn init_schema(&self) -> Result<()> {
        self.db.query(
            "DEFINE TABLE IF NOT EXISTS agent_run SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_project ON TABLE agent_run FIELDS project;
             DEFINE INDEX IF NOT EXISTS idx_status ON TABLE agent_run FIELDS status;
             DEFINE INDEX IF NOT EXISTS idx_project_status ON TABLE agent_run FIELDS project, status;"
        )
        .await
        .context("Failed to initialize schema")?;

        info!("Schema initialized");
        Ok(())
    }

    /// Store a new agent run. Returns the record ID.
    pub async fn store_run(&self, run: &AgentRun) -> Result<String> {
        let json = serde_json::to_value(run)?;

        let mut result = self.db
            .query("CREATE agent_run CONTENT $data RETURN id")
            .bind(("data", json))
            .await
            .context("Failed to store agent run")?;

        let created: Option<serde_json::Value> = result.take(0)?;
        let id = created
            .and_then(|v| v.get("id").and_then(|id| id.as_str().map(String::from)))
            .unwrap_or_else(|| "unknown".into());

        Ok(id)
    }

    /// Update an existing run by ID.
    pub async fn update_run(&self, id: &str, run: &AgentRun) -> Result<()> {
        let json = serde_json::to_value(run)?;
        let id = id.to_string();

        self.db
            .query("UPDATE type::thing('agent_run', $id) CONTENT $data")
            .bind(("id", id))
            .bind(("data", json))
            .await
            .context("Failed to update agent run")?;
        Ok(())
    }

    /// Get all runs for a project, optionally filtered by status.
    pub async fn list_runs(
        &self,
        project: &str,
        status: Option<RunStatus>,
    ) -> Result<Vec<AgentRun>> {
        let project = project.to_string();

        let mut result = match status {
            Some(s) => {
                let status_str = serde_json::to_value(&s)?;
                self.db
                    .query("SELECT * FROM agent_run WHERE project = $project AND status = $status ORDER BY created_at DESC")
                    .bind(("project", project))
                    .bind(("status", status_str))
                    .await
                    .context("Failed to list runs")?
            }
            None => {
                self.db
                    .query("SELECT * FROM agent_run WHERE project = $project ORDER BY created_at DESC")
                    .bind(("project", project))
                    .await
                    .context("Failed to list runs")?
            }
        };

        let rows: Vec<serde_json::Value> = result.take(0)?;
        let runs: Vec<AgentRun> = rows.into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        Ok(runs)
    }

    /// Get currently running agents for a project.
    pub async fn running_agents(&self, project: &str) -> Result<Vec<AgentRun>> {
        self.list_runs(project, Some(RunStatus::Running)).await
    }

    /// Find past runs with similar task descriptions (by embedding cosine similarity).
    pub async fn find_similar(
        &self,
        project: &str,
        embedding: &[f32],
        limit: usize,
        min_similarity: f32,
    ) -> Result<Vec<SimilarRun>> {
        let project = project.to_string();
        let embedding = serde_json::to_value(embedding)?;

        let mut result = self.db
            .query(
                "SELECT *, vector::similarity::cosine(embedding, $embedding) AS similarity
                 FROM agent_run
                 WHERE project = $project
                   AND embedding IS NOT NONE
                   AND vector::similarity::cosine(embedding, $embedding) >= $min_sim
                 ORDER BY similarity DESC
                 LIMIT $limit"
            )
            .bind(("project", project))
            .bind(("embedding", embedding))
            .bind(("min_sim", min_similarity))
            .bind(("limit", limit as i64))
            .await
            .context("Failed to find similar runs")?;

        let rows: Vec<serde_json::Value> = result.take(0)?;
        let similar: Vec<SimilarRun> = rows.into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        Ok(similar)
    }

    /// Check if a similar task has already been completed successfully.
    pub async fn task_already_done(
        &self,
        project: &str,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Option<AgentRun>> {
        let similar = self.find_similar(project, embedding, 1, threshold).await?;

        Ok(similar.into_iter()
            .find(|s| s.status == RunStatus::Passed)
            .map(|s| s.into_run()))
    }

    /// Find past errors similar to the current task.
    pub async fn find_past_errors(
        &self,
        project: &str,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SimilarRun>> {
        let project = project.to_string();
        let embedding = serde_json::to_value(embedding)?;

        let mut result = self.db
            .query(
                "SELECT *, vector::similarity::cosine(embedding, $embedding) AS similarity
                 FROM agent_run
                 WHERE project = $project
                   AND status = 'failed'
                   AND embedding IS NOT NONE
                   AND vector::similarity::cosine(embedding, $embedding) >= 0.5
                 ORDER BY similarity DESC
                 LIMIT $limit"
            )
            .bind(("project", project))
            .bind(("embedding", embedding))
            .bind(("limit", limit as i64))
            .await
            .context("Failed to find past errors")?;

        let rows: Vec<serde_json::Value> = result.take(0)?;
        let errors: Vec<SimilarRun> = rows.into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        Ok(errors)
    }

    /// Store an embedding for a run.
    pub async fn store_embedding(&self, id: &str, embedding: &[f32]) -> Result<()> {
        let id = id.to_string();
        let embedding = serde_json::to_value(embedding)?;

        self.db
            .query("UPDATE type::thing('agent_run', $id) SET embedding = $embedding")
            .bind(("id", id))
            .bind(("embedding", embedding))
            .await
            .context("Failed to store embedding")?;
        Ok(())
    }
}
