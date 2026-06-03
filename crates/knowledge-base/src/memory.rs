//! Namespaced semantic agent memory (AgentDB-like) over SurrealDB.
//!
//! Backs the SONA learning loop: trajectories and distilled patterns are
//! stored here and vector-retrieved into future planner prompts. Search is
//! cosine similarity over the `memory_entry` table; the HNSW index (defined
//! fault-tolerantly in `init_schema`) accelerates it when available and the
//! query degrades to a full scan when not.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

use inference_client::OllamaBackend;
use crate::schema::{MemoryEntry, MemorySearchHit, MEM_NS_TRAJECTORIES};
use crate::backend::KnowledgeBackend;

/// Default number of hits returned by a semantic search.
pub const DEFAULT_TOP_K: usize = 5;
/// Minimum cosine similarity for a hit to count.
pub const MIN_SIMILARITY: f32 = 0.5;
/// Entries used fewer times than this AND stale are pruned by `decay`.
pub const DECAY_MIN_USE_COUNT: u32 = 2;
/// Entries not used for this many days are decay candidates.
pub const DECAY_STALE_DAYS: i64 = 30;
/// Closed-loop reranking: floor on the similarity multiplier (a 0%-success
/// pattern keeps this fraction of its semantic similarity).
const EFFECTIVENESS_FLOOR: f32 = 0.5;
/// Weight given to a pattern with no recorded effectiveness yet.
const NEUTRAL_PATTERN_WEIGHT: f32 = 0.75;
/// Overfetch factor before effectiveness reranking.
const EFFECTIVENESS_OVERFETCH: usize = 3;

/// Namespaced semantic memory store. Reuses the daemon's existing Ollama
/// client + embed model — never creates a second embedding path.
pub struct MemoryStore {
    store: Arc<dyn KnowledgeBackend>,
    ollama: Arc<OllamaBackend>,
    embed_model: String,
}

impl MemoryStore {
    pub fn new(store: Arc<dyn KnowledgeBackend>, ollama: Arc<OllamaBackend>, embed_model: String) -> Self {
        Self { store, ollama, embed_model }
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    /// Embed arbitrary text with the configured embed model (e.g. to key a
    /// pattern by GOAL embedding rather than content embedding).
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.ollama.embed(&self.embed_model, text).await.context("embed failed")
    }

    /// Store an entry; embeds `content` when `embedding` is empty.
    /// Upserts by (namespace, project, key) so repeated learnings reinforce
    /// one entry instead of accumulating duplicates. Returns the record id.
    pub async fn store(&self, mut entry: MemoryEntry) -> Result<String> {
        if entry.embedding.is_empty() {
            entry.embedding = self.ollama.embed(&self.embed_model, &entry.content).await
                .context("memory embed failed")?;
        }
        if entry.created_at.is_empty() {
            entry.created_at = Self::now();
        }
        if entry.last_used_at.is_empty() {
            entry.last_used_at = entry.created_at.clone();
        }

        let mut json = serde_json::to_value(&entry)?;
        if let serde_json::Value::Object(ref mut map) = json {
            map.remove("id");
        }
        let rows = self.store.query_json(
            "UPSERT memory_entry CONTENT $data WHERE namespace = $ns AND project = $project AND key = $key RETURN id",
            serde_json::json!({
                "data": json,
                "ns": entry.namespace,
                "project": entry.project,
                "key": entry.key,
            }),
        ).await.context("memory store failed")?;

        // Mirror into the embedded ruvector ANN index (rebuildable accelerator;
        // SurrealDB row above is authoritative). Keyed by ns:project:key so
        // re-stores replace in place. Best-effort — never fails the store.
        if let Some(idx) = crate::rvindex::RvIndex::global() {
            let id = Self::rv_id(&entry.namespace, &entry.project, &entry.key);
            if let Err(e) = idx.insert(&id, &entry.namespace, &entry.project, &entry.key, &entry.content, entry.embedding.clone()) {
                warn!(error = %e, "ruvector index insert failed (degraded to SurrealDB search)");
            }
        }

        Ok(rows.first()
            .and_then(|v| v.get("id").map(|id| id.to_string().trim_matches('"').to_string()))
            .unwrap_or_else(|| "unknown".into()))
    }

    /// Stable ruvector id for a logical memory entry.
    fn rv_id(namespace: &str, project: &str, key: &str) -> String {
        format!("{namespace}:{project}:{key}")
    }

    /// Rebuild the global ruvector index from all `memory_entry` rows.
    /// Called once at daemon startup (the index is an ephemeral cache).
    pub async fn rebuild_index(&self) -> Result<usize> {
        let Some(idx) = crate::rvindex::RvIndex::global() else { return Ok(0) };
        let rows = self.store.query_json(
            "SELECT id, namespace, project, key, content, embedding FROM memory_entry WHERE embedding IS NOT NONE",
            serde_json::Value::Null,
        ).await.context("rebuild_index query failed")?;
        let mut n = 0;
        for row in rows {
            let ns = row.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
            let project = row.get("project").and_then(|v| v.as_str()).unwrap_or("");
            let key = row.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let content = row.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let embedding: Vec<f32> = row.get("embedding")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            if embedding.is_empty() || ns.is_empty() { continue; }
            let id = Self::rv_id(ns, project, key);
            if idx.insert(&id, ns, project, key, content, embedding).is_ok() { n += 1; }
        }
        Ok(n)
    }

    /// Namespace-scoped semantic search by a pre-computed query embedding.
    /// Uses the embedded ruvector HNSW index when available (O(log n)),
    /// falling back to a SurrealDB cosine scan otherwise. Hits get a
    /// fire-and-forget `touch` (use_count + recency bump).
    pub async fn search(
        &self,
        namespaces: &[&str],
        project: &str,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<MemorySearchHit>> {
        // Fast path: embedded ruvector ANN index.
        if let Some(idx) = crate::rvindex::RvIndex::global() {
            match idx.search(namespaces, project, query_embedding, top_k) {
                Ok(rv_hits) => {
                    let mut hits = Vec::new();
                    for h in rv_hits {
                        if h.similarity < MIN_SIMILARITY { continue; }
                        self.touch_by_key(&h.namespace, project, &h.key).await;
                        hits.push(MemorySearchHit {
                            entry: MemoryEntry {
                                id: Some(h.id),
                                namespace: h.namespace,
                                key: h.key,
                                content: h.content,
                                embedding: Vec::new(),
                                metadata: serde_json::Value::Null,
                                project: project.to_string(),
                                created_at: String::new(),
                                last_used_at: String::new(),
                                use_count: 0,
                                ttl_secs: None,
                            },
                            similarity: h.similarity,
                        });
                    }
                    debug!(hits = hits.len(), "memory search (ruvector)");
                    return Ok(hits);
                }
                Err(e) => warn!(error = %e, "ruvector search failed — falling back to cosine scan"),
            }
        }

        // Fallback: SurrealDB cosine scan.
        let rows = self.store.query_json(
            "SELECT *, vector::similarity::cosine(embedding, $embedding) AS similarity
             FROM memory_entry
             WHERE namespace IN $namespaces
               AND project = $project
               AND embedding IS NOT NONE
               AND vector::similarity::cosine(embedding, $embedding) >= $min_sim
             ORDER BY similarity DESC
             LIMIT $limit",
            serde_json::json!({
                "namespaces": namespaces,
                "project": project,
                "embedding": query_embedding,
                "min_sim": MIN_SIMILARITY,
                "limit": top_k as i64,
            }),
        ).await.context("memory search failed")?;

        let mut hits = Vec::new();
        for mut row in rows {
            let similarity = row.get("similarity").and_then(|s| s.as_f64()).unwrap_or(0.0) as f32;
            if let serde_json::Value::Object(ref mut map) = row {
                map.remove("similarity");
            }
            if let Ok(entry) = serde_json::from_value::<MemoryEntry>(row) {
                if let Some(id) = &entry.id {
                    self.touch(id).await;
                }
                hits.push(MemorySearchHit { entry, similarity });
            }
        }
        debug!(hits = hits.len(), "memory search");
        Ok(hits)
    }

    /// Convenience: embed `query_text`, then `search`.
    pub async fn search_text(
        &self,
        namespaces: &[&str],
        project: &str,
        query_text: &str,
        top_k: usize,
    ) -> Result<Vec<MemorySearchHit>> {
        let embedding = self.ollama.embed(&self.embed_model, query_text).await
            .context("memory query embed failed")?;
        self.search(namespaces, project, &embedding, top_k).await
    }

    /// Like `search_text`, but reranks by closed-loop effectiveness: a hit's
    /// semantic similarity is scaled by how often that pattern led to a
    /// successful run (from `pattern_effectiveness`). Proven patterns rise;
    /// patterns that consistently precede failures sink. Patterns with no
    /// history get a neutral weight. Pure stats — non-neural, honest.
    pub async fn search_text_weighted(
        &self,
        namespaces: &[&str],
        project: &str,
        query_text: &str,
        top_k: usize,
    ) -> Result<Vec<MemorySearchHit>> {
        let embedding = self.ollama.embed(&self.embed_model, query_text).await
            .context("memory query embed failed")?;
        // Overfetch so reranking can promote a lower-similarity-but-proven hit.
        let mut hits = self.search(namespaces, project, &embedding, top_k.saturating_mul(EFFECTIVENESS_OVERFETCH)).await?;
        let weights = self.pattern_weights(project).await.unwrap_or_default();
        for h in &mut hits {
            let w = h.entry.id.as_deref()
                .and_then(|id| weights.get(id).copied())
                .unwrap_or(NEUTRAL_PATTERN_WEIGHT);
            // Floor keeps a 0%-success pattern at EFFECTIVENESS_FLOOR of its
            // similarity (semantic relevance still counts); 100% → full.
            h.similarity *= EFFECTIVENESS_FLOOR + (1.0 - EFFECTIVENESS_FLOOR) * w;
        }
        hits.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(top_k);
        Ok(hits)
    }

    /// Per-pattern success rate (succeeded / total) from `pattern_effectiveness`.
    pub async fn pattern_weights(&self, project: &str) -> Result<HashMap<String, f32>> {
        let rows = self.store.query_json(
            "SELECT pattern_id, run_succeeded FROM pattern_effectiveness WHERE project = $project",
            serde_json::json!({ "project": project }),
        ).await.context("pattern_weights query failed")?;
        let mut tally: HashMap<String, (u32, u32)> = HashMap::new(); // id -> (succ, total)
        for row in &rows {
            let Some(id) = row.get("pattern_id").and_then(|v| v.as_str()) else { continue };
            let ok = row.get("run_succeeded").and_then(|v| v.as_bool()).unwrap_or(false);
            let e = tally.entry(id.to_string()).or_insert((0, 0));
            e.1 += 1;
            if ok { e.0 += 1; }
        }
        Ok(tally.into_iter().map(|(id, (s, t))| (id, if t > 0 { s as f32 / t as f32 } else { NEUTRAL_PATTERN_WEIGHT })).collect())
    }

    /// Exact-key lookup (no vector math).
    pub async fn recall(&self, namespace: &str, project: &str, key: &str) -> Result<Option<MemoryEntry>> {
        let rows = self.store.query_json(
            "SELECT * FROM memory_entry WHERE namespace = $ns AND project = $project AND key = $key LIMIT 1",
            serde_json::json!({ "ns": namespace, "project": project, "key": key }),
        ).await.context("memory recall failed")?;
        Ok(rows.into_iter().next().and_then(|v| serde_json::from_value(v).ok()))
    }

    /// Bump usage stats by logical key (used by the ruvector search path,
    /// whose hit ids are ns:project:key, not SurrealDB record ids).
    pub async fn touch_by_key(&self, namespace: &str, project: &str, key: &str) {
        let q = "UPDATE memory_entry SET use_count += 1, last_used_at = time::now() \
                 WHERE namespace = $ns AND project = $project AND key = $key";
        if let Err(e) = self.store.query_json(q, serde_json::json!({
            "ns": namespace, "project": project, "key": key,
        })).await {
            warn!(key, error = %e, "memory touch_by_key failed");
        }
    }

    /// Bump usage stats; best-effort.
    pub async fn touch(&self, id: &str) {
        let query = if id.contains(':') {
            format!("UPDATE {id} SET use_count += 1, last_used_at = time::now()")
        } else {
            format!("UPDATE type::thing('memory_entry', '{id}') SET use_count += 1, last_used_at = time::now()")
        };
        if let Err(e) = self.store.db_query_raw(&query).await {
            warn!(id, error = %e, "memory touch failed");
        }
    }

    /// SONA effectiveness: aggregate `pattern_effectiveness` rows for a project.
    /// Returns `{ runs_with_pattern, succeeded, failed, success_rate }`.
    pub async fn pattern_hit_rate(&self, project: &str) -> Result<serde_json::Value> {
        let rows = self.store.query_json(
            "SELECT run_succeeded, count() AS cnt FROM pattern_effectiveness
             WHERE project = $project GROUP BY run_succeeded",
            serde_json::json!({ "project": project }),
        ).await.context("pattern_hit_rate query failed")?;

        let mut succeeded: u64 = 0;
        let mut failed: u64 = 0;
        for row in &rows {
            let cnt = row.get("cnt").and_then(|c| c.as_u64()).unwrap_or(0);
            if row.get("run_succeeded").and_then(|s| s.as_bool()).unwrap_or(false) {
                succeeded += cnt;
            } else {
                failed += cnt;
            }
        }
        let total = succeeded + failed;
        let success_rate = if total > 0 { succeeded as f64 / total as f64 } else { 0.0 };
        Ok(serde_json::json!({
            "runs_with_pattern": total,
            "succeeded": succeeded,
            "failed": failed,
            "success_rate": success_rate,
        }))
    }

    /// Prune unused stale entries and TTL-expired entries for a project.
    /// Trajectories are exempt from the staleness rule (they feed distillation)
    /// but still honor explicit TTLs. Returns the number of pruned rows.
    pub async fn decay(&self, project: &str) -> Result<usize> {
        let stale_rows = self.store.query_json(
            "DELETE memory_entry
             WHERE project = $project
               AND namespace != $trajectories
               AND use_count < $min_use
               AND last_used_at < time::now() - duration::from::days($stale_days)
             RETURN BEFORE",
            serde_json::json!({
                "project": project,
                "trajectories": MEM_NS_TRAJECTORIES,
                "min_use": DECAY_MIN_USE_COUNT,
                "stale_days": DECAY_STALE_DAYS,
            }),
        ).await.context("memory decay (stale) failed")?;

        let ttl_rows = self.store.query_json(
            "DELETE memory_entry
             WHERE project = $project
               AND ttl_secs IS NOT NONE
               AND created_at < time::now() - duration::from::secs(ttl_secs)
             RETURN BEFORE",
            serde_json::json!({ "project": project }),
        ).await.context("memory decay (ttl) failed")?;

        // Evict the pruned entries from the ANN index too (keep it in sync).
        if let Some(idx) = crate::rvindex::RvIndex::global() {
            for row in stale_rows.iter().chain(ttl_rows.iter()) {
                let ns = row.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                let key = row.get("key").and_then(|v| v.as_str()).unwrap_or("");
                if !ns.is_empty() {
                    let _ = idx.delete(&Self::rv_id(ns, project, key));
                }
            }
        }

        Ok(stale_rows.len() + ttl_rows.len())
    }
}
