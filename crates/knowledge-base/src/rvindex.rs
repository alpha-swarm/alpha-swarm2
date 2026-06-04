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

use ruvector_core::advanced_features::sparse_vector::{
    fuse_rankings, FusionConfig, FusionStrategy, ScoredDoc, SparseIndex, SparseVector,
};
use ruvector_core::types::{DbOptions, DistanceMetric, SearchQuery, VectorEntry};
use ruvector_core::AgenticDB;

/// Overfetch factor: ruvector filters are exact-match maps and we post-filter
/// by namespace set + project, so pull extra candidates before trimming to k.
const OVERFETCH: usize = 8;
/// RRF rank-pressure for dense+sparse fusion (ruvector default).
const RRF_K: f32 = 60.0;

/// Tokenize text into a sparse term-frequency vector (lowercased alphanumeric
/// tokens, FNV-1a-hashed to u32 term ids, weight = sqrt(tf)). A pragmatic
/// BM25-lite signal for hybrid retrieval.
fn tokenize_sparse(text: &str) -> SparseVector {
    let mut tf: HashMap<u32, f32> = HashMap::new();
    let mut tok = String::new();
    let flush = |t: &mut String, tf: &mut HashMap<u32, f32>| {
        if t.len() >= 2 {
            *tf.entry(fnv1a32(t)).or_insert(0.0) += 1.0;
        }
        t.clear();
    };
    for c in text.chars() {
        if c.is_alphanumeric() {
            tok.extend(c.to_lowercase());
        } else {
            flush(&mut tok, &mut tf);
        }
    }
    flush(&mut tok, &mut tf);
    let pairs: Vec<(u32, f32)> = tf.into_iter().map(|(id, n)| (id, n.sqrt())).collect();
    SparseVector::new(pairs)
}

fn fnv1a32(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// A hit from the ANN index: enough to build a `MemorySearchHit` without a
/// SurrealDB round-trip (content rides in the vector metadata).
#[derive(Clone)]
pub struct RvHit {
    pub id: String,
    pub namespace: String,
    pub project: String,
    pub key: String,
    pub content: String,
    pub similarity: f32,
}

/// Embedded ANN index over memory embeddings. `AgenticDB` methods take `&self`;
/// the `Mutex` guarantees `Sync` regardless of its internals and ops are sub-ms.
pub struct RvIndex {
    inner: Mutex<AgenticDB>,
    /// Keyword (sparse BM25-lite) index, fused with the dense HNSW ranking.
    sparse: Mutex<SparseIndex>,
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
        let _ = RV_INDEX.set(RvIndex { inner: Mutex::new(db), sparse: Mutex::new(SparseIndex::new()) });
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
        {
            let db = self.inner.lock().expect("rvindex poisoned");
            // Replace-by-id: delete any prior, then insert.
            let _ = db.delete(id);
            db.insert(Self::entry(id, namespace, project, key, content, vector))
                .map_err(|e| anyhow::anyhow!("ruvector insert: {e}"))?;
        }
        // Mirror into the sparse keyword index (content tokens).
        let mut sp = self.sparse.lock().expect("sparse poisoned");
        sp.remove(&id.to_string());
        sp.insert(id.to_string(), tokenize_sparse(content));
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        {
            let mut sp = self.sparse.lock().expect("sparse poisoned");
            sp.remove(&id.to_string());
        }
        let db = self.inner.lock().expect("rvindex poisoned");
        db.delete(id).map(|_| ()).map_err(|e| anyhow::anyhow!("ruvector delete: {e}"))
    }

    /// Dense-only ANN search (cosine), post-filtered to `namespaces` ∩
    /// `project`, top `k`. Retained for callers without query text.
    pub fn search(&self, namespaces: &[&str], project: &str, query: &[f32], k: usize) -> Result<Vec<RvHit>> {
        let hits = self.dense_candidates(query, (k * OVERFETCH).max(k))?
            .into_iter()
            .filter(|h| h.matches(namespaces, project))
            .take(k)
            .collect();
        Ok(hits)
    }

    /// Hybrid search: dense HNSW + sparse keyword, fused with RRF for ordering.
    /// The reported `similarity` stays the dense cosine value so the caller's
    /// min-similarity threshold remains meaningful; fusion only reorders.
    /// `query_text` builds the sparse query; `query` is its embedding.
    pub fn search_hybrid(
        &self,
        namespaces: &[&str],
        project: &str,
        query_text: &str,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<RvHit>> {
        let overfetch = (k * OVERFETCH).max(k);
        // Dense candidates (id-keyed for rehydration + cosine for threshold).
        let dense = self.dense_candidates(query, overfetch)?;
        if dense.is_empty() {
            return Ok(Vec::new());
        }
        let dense_scored: Vec<ScoredDoc> = dense.iter()
            .map(|h| ScoredDoc { id: h.id.clone(), score: h.similarity })
            .collect();

        // Sparse keyword ranking.
        let sparse_q = tokenize_sparse(query_text);
        let sparse_scored: Vec<ScoredDoc> = if sparse_q.is_empty() {
            Vec::new()
        } else {
            self.sparse.lock().expect("sparse poisoned").search(&sparse_q, overfetch)
        };

        // Fuse for ORDER (RRF). Sparse-only ids not in the dense pool are
        // dropped — the 8x dense overfetch keeps recall high.
        let fused = fuse_rankings(&dense_scored, &sparse_scored,
            &FusionConfig { strategy: FusionStrategy::RRF { k: RRF_K }, top_k: overfetch });

        let by_id: HashMap<&str, &RvHit> = dense.iter().map(|h| (h.id.as_str(), h)).collect();
        let mut out = Vec::new();
        for sd in &fused {
            if let Some(h) = by_id.get(sd.id.as_str()) {
                if h.matches(namespaces, project) {
                    out.push((*h).clone());
                    if out.len() >= k { break; }
                }
            }
        }
        Ok(out)
    }

    /// Raw dense candidates with cosine similarity (no ns/project filter).
    fn dense_candidates(&self, query: &[f32], k: usize) -> Result<Vec<RvHit>> {
        let db = self.inner.lock().expect("rvindex poisoned");
        let results = db.search(SearchQuery {
            vector: query.to_vec(),
            k,
            filter: None,
            ef_search: None,
        }).map_err(|e| anyhow::anyhow!("ruvector search: {e}"))?;
        Ok(results.into_iter().filter_map(|r| {
            let md = r.metadata.as_ref()?;
            let get = |key: &str| md.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(RvHit {
                id: r.id,
                namespace: get("namespace"),
                project: get("project"),
                key: get("key"),
                content: get("content"),
                // ruvector cosine score is a DISTANCE (1 - cos_sim) → similarity.
                similarity: 1.0 - r.score,
            })
        }).collect())
    }
}

impl RvHit {
    fn matches(&self, namespaces: &[&str], project: &str) -> bool {
        self.project == project && namespaces.contains(&self.namespace.as_str())
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
        let idx = RvIndex { inner: Mutex::new(db), sparse: Mutex::new(SparseIndex::new()) };

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

    #[test]
    fn hybrid_promotes_keyword_match_within_dense_pool() {
        let dir = std::env::temp_dir().join(format!("rvhybrid-{}", std::process::id()));
        let path = dir.to_str().unwrap();
        let _ = std::fs::remove_dir_all(path);
        std::fs::create_dir_all(dir.parent().unwrap()).ok();
        let db = AgenticDB::new(DbOptions { dimensions: 3, distance_metric: DistanceMetric::Cosine, storage_path: path.to_string(), ..DbOptions::default() }).unwrap();
        let idx = RvIndex { inner: Mutex::new(db), sparse: Mutex::new(SparseIndex::new()) };

        // Two near-equidistant vectors; only B's text matches the keyword query.
        idx.insert("a", "patterns", "p", "ka", "alpha generic notes", vec![1.0, 0.1, 0.0]).unwrap();
        idx.insert("b", "patterns", "p", "kb", "rust async tokio runtime", vec![1.0, 0.0, 0.1]).unwrap();

        // Dense-only would order by vector proximity; hybrid should let the
        // keyword query "tokio runtime" lift B regardless.
        let hits = idx.search_hybrid(&["patterns"], "p", "tokio runtime", &[1.0, 0.05, 0.05], 2).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, "b", "keyword match not promoted: {:?}",
            hits.iter().map(|h| &h.id).collect::<Vec<_>>());
        // similarity stays cosine-scaled (threshold-comparable), not a tiny RRF score
        assert!(hits[0].similarity > 0.5, "similarity should be cosine-scale: {}", hits[0].similarity);
        let _ = std::fs::remove_dir_all(path);
    }
}
