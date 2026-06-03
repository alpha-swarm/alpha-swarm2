//! SurrealQL persistence for workflow documents, via `KnowledgeStore::query_json`.
//!
//! Table DDL lives in `KnowledgeStore::init_schema()` (single combined edit —
//! see the workflow_def / workflow_run definitions there).

use anyhow::{Context, Result};
use knowledge_base::KnowledgeBackend;
use std::sync::Arc;

use crate::model::{WorkflowDef, WorkflowRun};

/// Persistence facade for `workflow_def` / `workflow_run` tables.
pub struct WorkflowRepo {
    store: Arc<dyn KnowledgeBackend>,
}

impl WorkflowRepo {
    pub fn new(store: Arc<dyn KnowledgeBackend>) -> Self {
        Self { store }
    }

    /// Persist a new workflow run. Returns the record id.
    pub async fn create_run(&self, wf: &WorkflowRun) -> Result<String> {
        let mut json = serde_json::to_value(wf)?;
        if let serde_json::Value::Object(ref mut map) = json {
            map.remove("id");
        }
        let rows = self.store.query_json(
            "CREATE workflow_run CONTENT $data RETURN id",
            serde_json::json!({ "data": json }),
        ).await.context("create workflow_run failed")?;
        Ok(rows.first()
            .and_then(|v| v.get("id").map(|id| id.to_string().trim_matches('"').to_string()))
            .unwrap_or_else(|| "unknown".into()))
    }

    /// Checkpoint the full run document (atomic single-document replace),
    /// addressed by its agent_run id.
    pub async fn update_run(&self, wf: &WorkflowRun) -> Result<()> {
        let mut json = serde_json::to_value(wf)?;
        if let serde_json::Value::Object(ref mut map) = json {
            map.remove("id");
        }
        self.store.query_json(
            "UPDATE workflow_run CONTENT $data WHERE run_id = $run_id",
            serde_json::json!({ "data": json, "run_id": wf.run_id }),
        ).await.context("update workflow_run failed")?;
        Ok(())
    }

    /// Load the workflow run for an agent_run id.
    pub async fn get_by_run_id(&self, run_id: &str) -> Result<Option<WorkflowRun>> {
        let rows = self.store.query_json(
            "SELECT * FROM workflow_run WHERE run_id = $run_id LIMIT 1",
            serde_json::json!({ "run_id": run_id }),
        ).await.context("get workflow_run failed")?;
        Ok(rows.into_iter().next().and_then(|v| serde_json::from_value(v).ok()))
    }

    /// Runs needing crash recovery or resumable display.
    pub async fn list_active(&self) -> Result<Vec<WorkflowRun>> {
        let rows = self.store.query_json(
            "SELECT * FROM workflow_run WHERE state IN ['running', 'paused'] ORDER BY created_at ASC",
            serde_json::json!({}),
        ).await.context("list active workflow_runs failed")?;
        Ok(rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
    }

    /// List all runs for a project (newest first).
    pub async fn list_runs(&self, project: &str) -> Result<Vec<WorkflowRun>> {
        let rows = self.store.query_json(
            "SELECT * FROM workflow_run WHERE project = $project ORDER BY created_at DESC",
            serde_json::json!({ "project": project }),
        ).await.context("list workflow_runs failed")?;
        Ok(rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
    }

    /// Upsert a reusable workflow definition (keyed by name + version).
    pub async fn store_def(&self, def: &WorkflowDef) -> Result<()> {
        let mut json = serde_json::to_value(def)?;
        if let serde_json::Value::Object(ref mut map) = json {
            map.remove("id");
        }
        self.store.query_json(
            "UPSERT workflow_def CONTENT $data WHERE name = $name AND version = $version",
            serde_json::json!({ "data": json, "name": def.name, "version": def.version }),
        ).await.context("store workflow_def failed")?;
        Ok(())
    }

    /// Fetch a definition by name; latest version when `version` is None.
    pub async fn get_def(&self, name: &str, version: Option<u32>) -> Result<Option<WorkflowDef>> {
        let rows = match version {
            Some(v) => self.store.query_json(
                "SELECT * FROM workflow_def WHERE name = $name AND version = $version LIMIT 1",
                serde_json::json!({ "name": name, "version": v }),
            ).await,
            None => self.store.query_json(
                "SELECT * FROM workflow_def WHERE name = $name ORDER BY version DESC LIMIT 1",
                serde_json::json!({ "name": name }),
            ).await,
        }.context("get workflow_def failed")?;
        Ok(rows.into_iter().next().and_then(|v| serde_json::from_value(v).ok()))
    }

    pub async fn list_defs(&self) -> Result<Vec<WorkflowDef>> {
        let rows = self.store.query_json(
            "SELECT * FROM workflow_def ORDER BY name ASC, version DESC",
            serde_json::json!({}),
        ).await.context("list workflow_defs failed")?;
        Ok(rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
    }
}
