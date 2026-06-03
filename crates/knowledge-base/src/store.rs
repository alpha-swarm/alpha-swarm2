use anyhow::{Context, Result};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use tracing::info;

use crate::schema::*;
use crate::queries::SimilarRun;

/// Knowledge store over SurrealDB. The connection is `engine::any`, so one
/// type serves both modes: embedded `surrealkv://` (daemon = sole DB owner)
/// and remote `ws://` (escape hatch while an external server exists).
pub struct KnowledgeStore {
    db: Surreal<Any>,
}

impl KnowledgeStore {
    /// Mode-aware constructor — the primary entry point.
    /// `SurrealMode::Nats` is rejected here: bridge consumers use
    /// `bridge_client::NatsDbClient`, not a `KnowledgeStore`.
    pub async fn connect_with(cfg: &swarm_config::SurrealConfig) -> Result<Self> {
        match cfg.mode {
            swarm_config::SurrealMode::Embedded => {
                Self::connect_embedded(&cfg.path, &cfg.namespace, &cfg.database).await
            }
            swarm_config::SurrealMode::Remote => {
                Self::connect(&cfg.url, &cfg.namespace, &cfg.database).await
            }
            swarm_config::SurrealMode::Nats => anyhow::bail!(
                "SurrealMode::Nats has no KnowledgeStore — use bridge_client::NatsDbClient"
            ),
        }
    }

    /// Open the embedded kv-surrealkv engine at `path` (no auth needed).
    pub async fn connect_embedded(path: &str, namespace: &str, database: &str) -> Result<Self> {
        let endpoint = format!("surrealkv://{path}");
        let db = surrealdb::engine::any::connect(endpoint.as_str())
            .await
            .context("Failed to open embedded SurrealDB (surrealkv)")?;

        db.use_ns(namespace).use_db(database)
            .await
            .context("Failed to select namespace/database")?;

        let store = Self { db };
        store.init_schema().await?;

        info!(path, namespace, database, "Opened embedded SurrealDB (surrealkv)");
        Ok(store)
    }

    /// Connect to an external SurrealDB server over WebSocket (legacy/remote
    /// mode) and initialize the schema.
    pub async fn connect(url: &str, namespace: &str, database: &str) -> Result<Self> {
        let endpoint = if url.contains("://") { url.to_string() } else { format!("ws://{url}") };
        let db = surrealdb::engine::any::connect(endpoint.as_str())
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
             DEFINE INDEX IF NOT EXISTS idx_project_status ON TABLE agent_run FIELDS project, status;
             DEFINE TABLE IF NOT EXISTS goal_plan SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_plan_run ON TABLE goal_plan FIELDS run_id;
             DEFINE TABLE IF NOT EXISTS file_embedding SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_file_project ON TABLE file_embedding FIELDS project, file_path;
             DEFINE TABLE IF NOT EXISTS project_index SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_project_index ON TABLE project_index FIELDS project;
             DEFINE TABLE IF NOT EXISTS workflow_def SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_wdef_name ON TABLE workflow_def FIELDS name, version;
             DEFINE TABLE IF NOT EXISTS workflow_run SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_wrun_runid ON TABLE workflow_run FIELDS run_id;
             DEFINE INDEX IF NOT EXISTS idx_wrun_proj ON TABLE workflow_run FIELDS project, state;
             DEFINE TABLE IF NOT EXISTS memory_entry SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_mem_ns ON TABLE memory_entry FIELDS namespace, project;
             DEFINE INDEX IF NOT EXISTS idx_mem_key ON TABLE memory_entry FIELDS namespace, project, key;
             DEFINE TABLE IF NOT EXISTS pattern_effectiveness SCHEMALESS;
             DEFINE INDEX IF NOT EXISTS idx_peff_project ON TABLE pattern_effectiveness FIELDS project;"
        )
        .await
        .context("Failed to initialize schema")?;

        // HNSW vector indexes: each in its OWN query and fault-tolerant —
        // unsupported syntax or a dimension mismatch degrades to full-scan
        // cosine (already correct) instead of bricking startup. NO index on
        // agent_run.embedding: legacy rows may carry mixed dimensions.
        let hnsw_ddl = [
            format!(
                "DEFINE INDEX IF NOT EXISTS idx_mem_hnsw ON TABLE memory_entry \
                 FIELDS embedding HNSW DIMENSION {EMBED_DIM} DIST COSINE;"
            ),
            format!(
                "DEFINE INDEX IF NOT EXISTS idx_file_embed_hnsw ON TABLE file_embedding \
                 FIELDS embedding HNSW DIMENSION {EMBED_DIM} DIST COSINE;"
            ),
        ];
        for ddl in &hnsw_ddl {
            if let Err(e) = self.db.query(ddl.as_str()).await {
                tracing::warn!(error = %e, ddl, "HNSW index DDL failed — falling back to full-scan cosine");
            }
        }

        info!("Schema initialized");
        Ok(())
    }

    /// Store a new agent run. Returns the record ID.
    pub async fn store_run(&self, run: &AgentRun) -> Result<String> {
        let mut json = serde_json::to_value(run)?;
        // Remove the id field so SurrealDB generates one
        if let serde_json::Value::Object(ref mut map) = json {
            map.remove("id");
        }

        let mut result = self.db
            .query("CREATE agent_run CONTENT $data RETURN id")
            .bind(("data", json))
            .await
            .context("Failed to store agent run")?;

        let created: Vec<serde_json::Value> = result.take(0)?;
        let id = created.first()
            .and_then(|v| v.get("id").map(|id| id.to_string().trim_matches('"').to_string()))
            .unwrap_or_else(|| "unknown".into());

        Ok(id)
    }

    /// Update an existing run by ID. Accepts both "agent_run:xyz" and "xyz" formats.
    pub async fn update_run(&self, id: &str, run: &AgentRun) -> Result<()> {
        let mut json = serde_json::to_value(run)?;
        // Remove the id field to avoid conflicts
        if let serde_json::Value::Object(ref mut map) = json {
            map.remove("id");
        }

        // Use the full record ID directly if it contains ":"
        let query = if id.contains(':') {
            format!("UPDATE {} CONTENT $data", id)
        } else {
            format!("UPDATE type::thing('agent_run', '{}') CONTENT $data", id)
        };

        self.db
            .query(query)
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

    /// Execute a raw SurrealQL query (for admin operations).
    pub async fn db_query_raw(&self, query: &str) -> Result<()> {
        self.db.query(query).await.context("Raw query failed")?;
        Ok(())
    }

    /// Execute a SurrealQL query with named JSON params and return the FIRST
    /// statement's rows as JSON values. Shared primitive for typed CRUD layers
    /// (workflow, memory) and the NATS DB bridge.
    pub async fn query_json(
        &self,
        query: &str,
        params: serde_json::Value,
    ) -> Result<Vec<serde_json::Value>> {
        let mut q = self.db.query(query);
        if let serde_json::Value::Object(map) = params {
            for (key, value) in map {
                q = q.bind((key, value));
            }
        }
        let mut result = q.await.context("query_json failed")?;
        let rows: Vec<serde_json::Value> = result.take(0).unwrap_or_default();
        Ok(rows)
    }

    /// Get all pending tasks across all projects.
    pub async fn list_pending(&self) -> Result<Vec<AgentRun>> {
        let mut result = self.db
            .query("SELECT * FROM agent_run WHERE status IN ['pending', 'planning', 'approved'] ORDER BY created_at ASC LIMIT 20")
            .await
            .context("Failed to list pending runs")?;

        let rows: Vec<serde_json::Value> = result.take(0)?;
        Ok(rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
    }

    /// Look up the repo_url for a project.
    pub async fn get_project_repo(&self, project_name: &str) -> Result<Option<String>> {
        let project_name = project_name.to_string();
        let mut result = self.db
            .query("SELECT repo_url FROM project WHERE name = $name LIMIT 1")
            .bind(("name", project_name))
            .await
            .context("Failed to query project")?;

        let rows: Vec<serde_json::Value> = result.take(0)?;
        Ok(rows.first()
            .and_then(|v| v.get("repo_url"))
            .and_then(|u| u.as_str())
            .map(String::from))
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

    // --- RAG: File-level embeddings for context retrieval ---

    /// Store or update an embedding for a file in a project.
    pub async fn store_file_embedding(&self, project: &str, file_path: &str, summary: &str, embedding: &[f32], content_hash: &str) -> Result<()> {
        let project = project.to_string();
        let file_path = file_path.to_string();
        let summary = summary.to_string();
        let embedding = serde_json::to_value(embedding)?;
        let content_hash = content_hash.to_string();

        self.db.query(
            "UPSERT file_embedding SET project = $project, file_path = $file_path, summary = $summary, embedding = $embedding, content_hash = $content_hash, updated_at = time::now() WHERE project = $project AND file_path = $file_path"
        )
        .bind(("project", project))
        .bind(("file_path", file_path))
        .bind(("summary", summary))
        .bind(("embedding", embedding))
        .bind(("content_hash", content_hash))
        .await
        .context("Failed to store file embedding")?;
        Ok(())
    }

    /// Get the content hash for a file embedding (for cache invalidation).
    pub async fn get_file_hash(&self, project: &str, file_path: &str) -> Result<Option<String>> {
        let mut result = self.db.query(
            "SELECT content_hash FROM file_embedding WHERE project = $project AND file_path = $file_path LIMIT 1"
        )
        .bind(("project", project.to_string()))
        .bind(("file_path", file_path.to_string()))
        .await
        .context("Failed to query file hash")?;

        let row: Option<serde_json::Value> = result.take(0)?;
        Ok(row.and_then(|v| v.get("content_hash").and_then(|h| h.as_str().map(String::from))))
    }

    /// Get the last indexed git commit SHA for a project.
    pub async fn get_last_indexed_commit(&self, project: &str) -> Result<Option<String>> {
        let mut result = self.db.query(
            "SELECT commit_sha FROM project_index WHERE project = $project LIMIT 1"
        )
        .bind(("project", project.to_string()))
        .await.context("Failed to query last indexed commit")?;
        let row: Option<serde_json::Value> = result.take(0)?;
        Ok(row.and_then(|v| v.get("commit_sha").and_then(|h| h.as_str().map(String::from))))
    }

    /// Store the last indexed git commit SHA for a project.
    pub async fn set_last_indexed_commit(&self, project: &str, commit_sha: &str) -> Result<()> {
        self.db.query(
            "UPSERT project_index SET project = $project, commit_sha = $sha, updated_at = time::now() WHERE project = $project"
        )
        .bind(("project", project.to_string()))
        .bind(("sha", commit_sha.to_string()))
        .await.context("Failed to store last indexed commit")?;
        Ok(())
    }

    /// Find the most relevant files for a task using vector similarity.
    /// Returns (file_path, summary, similarity_score).
    pub async fn find_relevant_files(
        &self,
        project: &str,
        task_embedding: &[f32],
        limit: usize,
        min_similarity: f32,
    ) -> Result<Vec<(String, String, f32)>> {
        let project = project.to_string();
        let embedding = serde_json::to_value(task_embedding)?;

        let mut result = self.db.query(
            "SELECT file_path, summary, vector::similarity::cosine(embedding, $embedding) AS similarity
             FROM file_embedding
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
        .context("Failed to find relevant files")?;

        let rows: Vec<serde_json::Value> = result.take(0)?;
        Ok(rows.iter().filter_map(|v| {
            let path = v.get("file_path")?.as_str()?.to_string();
            let summary = v.get("summary")?.as_str()?.to_string();
            let sim = v.get("similarity")?.as_f64()? as f32;
            Some((path, summary, sim))
        }).collect())
    }

    // --- Goal Plan CRUD ---

    /// Store a new plan version. Returns the record ID.
    pub async fn store_plan(&self, plan: &GoalPlan) -> Result<String> {
        let mut json = serde_json::to_value(plan)?;
        if let serde_json::Value::Object(ref mut map) = json {
            map.remove("id");
        }
        let mut result = self.db
            .query("CREATE goal_plan CONTENT $data RETURN id")
            .bind(("data", json))
            .await
            .context("Failed to store goal plan")?;

        let created: Vec<serde_json::Value> = result.take(0)?;
        let id = created.first()
            .and_then(|v| v.get("id").map(|id| id.to_string().trim_matches('"').to_string()))
            .unwrap_or_else(|| "unknown".into());
        Ok(id)
    }

    /// Get all plan versions for a run, ordered by version.
    pub async fn get_plans(&self, run_id: &str) -> Result<Vec<GoalPlan>> {
        let run_id = run_id.to_string();
        let mut result = self.db
            .query("SELECT * FROM goal_plan WHERE run_id = $run_id ORDER BY version ASC")
            .bind(("run_id", run_id))
            .await
            .context("Failed to get plans")?;

        let rows: Vec<serde_json::Value> = result.take(0)?;
        Ok(rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
    }

    /// Get the latest plan for a run.
    pub async fn get_latest_plan(&self, run_id: &str) -> Result<Option<GoalPlan>> {
        let run_id = run_id.to_string();
        let mut result = self.db
            .query("SELECT * FROM goal_plan WHERE run_id = $run_id ORDER BY version DESC LIMIT 1")
            .bind(("run_id", run_id))
            .await
            .context("Failed to get latest plan")?;

        let rows: Vec<serde_json::Value> = result.take(0)?;
        Ok(rows.into_iter().next().and_then(|v| serde_json::from_value(v).ok()))
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
