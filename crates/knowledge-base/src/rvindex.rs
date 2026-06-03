//! Embedded ruvector ANN index for agent memory.
//!
//! Wraps `ruvector_core::AgenticDB` (HNSW + SIMD, MIT, pure-Rust — no Node)
//! as a process-global, rebuildable accelerator. SurrealDB `memory_entry`
//! stays the system-of-record; this index is rebuilt from it on startup and
//! kept in sync on store/delete. `MemoryStore::search` uses it when present
//! and falls back to SurrealDB cosine scan when not (e.g. CLI contexts that
//! never initialize it).

use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ruvector_core::types::{DbOptions, DistanceMetric, SearchQuery, VectorEntry};
use ruvector_core::AgenticDB;

/// Overfetch factor: ruvector filters are exact-match maps and we post-filter
/// by namespace set + project, so pull extra candidates before trimming to k.
const OVERFETCH: usize = 8;

/// A hit from the ANN index: enough to build a `MemorySearchHit` without a
/// SurrealDB round-trip (content rides in the vector metadata).
pub struct RvHit {
    pub id: String,
    pub namespace: String,
    pub key: String,
    pub content: String,
    pub similarity: f32,
}

/// Embedded ANN index over memory embeddings. `AgenticDB` methods take `&self`;
/// the `Mutex` guarantees `Sync` regardless of its internals and ops are sub-ms.
pub struct RvIndex {
    inner: Mutex<AgenticDB>,
}

static RV_INDEX: OnceLock<RvIndex> = OnceLock::new();

impl RvIndex {
    /// Initialize the process-global index at `path` for `dims`-dim vectors.
    /// `path` is treated as an ephemeral cache — it is recreated each boot and
    /// repopulated via `rebuild`. Idempotent: a second call is a no-op.
    pub fn init(path: &str, dims: usize) -> Result<()> {
        if RV_INDEX.get().is_some() {
            return Ok(());
        }
        // Fresh cache each boot — SurrealDB is authoritative, this is rebuilt.
        let _ = std::fs::remove_dir_all(path);
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let options = DbOptions {
            dimensions: dims,
            distance_metric: DistanceMetric::Cosine,
            storage_path: path.to_string(),
            ..DbOptions::default()
        };
        let db = AgenticDB::new(options).map_err(|e| anyhow::anyhow!("ruvector init: {e}"))?;
        let _ = RV_INDEX.set(RvIndex { inner: Mutex::new(db) });
        Ok(())
    }

    /// The global index, if initialized.
    pub fn global() -> Option<&'static RvIndex> {
        RV_INDEX.get()
    }

    fn entry(id: &str, namespace: &str, project: &str, key: &str, content: &str, vector: Vec<f32>) -> VectorEntry {
        let mut md = HashMap::new();
        md.insert("namespace".to_string(), serde_json::Value::String(namespace.to_string()));
        md.insert("project".to_string(), serde_json::Value::String(project.to_string()));
        md.insert("key".to_string(), serde_json::Value::String(key.to_string()));
        md.insert("content".to_string(), serde_json::Value::String(content.to_string()));
        VectorEntry { id: Some(id.to_string()), vector, metadata: Some(md) }
    }

    /// Insert or replace a memory vector. `id` should be stable per logical
    /// entry (namespace:project:key) so re-stores update in place.
    pub fn insert(&self, id: &str, namespace: &str, project: &str, key: &str, content: &str, vector: Vec<f32>) -> Result<()> {
        let db = self.inner.lock().expect("rvindex poisoned");
        // Replace-by-id: delete any prior, then insert.
        let _ = db.delete(id);
        db.insert(Self::entry(id, namespace, project, key, content, vector))
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("ruvector insert: {e}"))
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let db = self.inner.lock().expect("rvindex poisoned");
        db.delete(id).map(|_| ()).map_err(|e| anyhow::anyhow!("ruvector delete: {e}"))
    }

    /// ANN search, post-filtered to `namespaces` ∩ `project`, top `k`.
    pub fn search(&self, namespaces: &[&str], project: &str, query: &[f32], k: usize) -> Result<Vec<RvHit>> {
        let db = self.inner.lock().expect("rvindex poisoned");
        let results = db.search(SearchQuery {
            vector: query.to_vec(),
            k: (k * OVERFETCH).max(k),
            filter: None,
            ef_search: None,
        }).map_err(|e| anyhow::anyhow!("ruvector search: {e}"))?;

        let mut hits = Vec::new();
        for r in results {
            let md = match &r.metadata { Some(m) => m, None => continue };
            let get = |key: &str| md.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ns = get("namespace");
            if get("project") != project || !namespaces.contains(&ns.as_str()) {
                continue;
            }
            hits.push(RvHit {
                id: r.id,
                namespace: ns,
                key: get("key"),
                content: get("content"),
                // ruvector cosine score is a DISTANCE (1 - cos_sim); convert to
                // similarity so higher = better, matching MemorySearchHit.
                similarity: 1.0 - r.score,
            });
            if hits.len() >= k { break; }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_search_roundtrip_and_score_semantics() {
        let dir = std::env::temp_dir().join(format!("rvindex-test-{}", std::process::id()));
        let path = dir.to_str().unwrap();
        // direct (not global) instance for the test
        let _ = std::fs::remove_dir_all(path);
        std::fs::create_dir_all(dir.parent().unwrap()).ok();
        let db = AgenticDB::new(DbOptions {
            dimensions: 3,
            distance_metric: DistanceMetric::Cosine,
            storage_path: path.to_string(),
            ..DbOptions::default()
        }).unwrap();
        let idx = RvIndex { inner: Mutex::new(db) };

        idx.insert("a", "patterns", "p", "ka", "near", vec![1.0, 0.0, 0.0]).unwrap();
        idx.insert("b", "patterns", "p", "kb", "far", vec![0.0, 1.0, 0.0]).unwrap();
        idx.insert("c", "solutions", "other", "kc", "wrongproj", vec![1.0, 0.0, 0.0]).unwrap();

        let hits = idx.search(&["patterns", "solutions"], "p", &[1.0, 0.0, 0.0], 5).unwrap();
        // project filter drops c; namespace filter keeps patterns
        assert!(hits.iter().all(|h| h.id != "c"), "project filter failed");
        assert!(!hits.is_empty(), "no hits");
        // nearest ('a', identical vector) must rank first — proves score is
        // ordered best-first regardless of similarity/distance convention.
        assert_eq!(hits[0].id, "a", "nearest vector did not rank first: {:?}",
            hits.iter().map(|h| (&h.id, h.similarity)).collect::<Vec<_>>());
        assert_eq!(hits[0].content, "near");
        // identical vector → similarity ~1.0 (locks the distance→similarity flip)
        assert!(hits[0].similarity > 0.9, "similarity not high for exact match: {}", hits[0].similarity);
        let _ = std::fs::remove_dir_all(path);
    }
}
