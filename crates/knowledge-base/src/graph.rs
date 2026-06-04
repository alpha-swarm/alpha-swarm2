//! Code knowledge-graph storage + traversal over SurrealDB.
//!
//! Entities and relations are extracted elsewhere (`swarm_tools::codegraph`,
//! which owns tree-sitter and is multi-language); this module persists them
//! into `code_entity` / `code_rel` and answers entity / relation / neighbor
//! (pathfinder) queries. Generic JSON in/out so it carries no dependency on
//! the extractor's types.
//!
//! Storage note: relation endpoints are stored as `src`/`dst` (NOT `from`/`to`)
//! because `from`/`to` collide with SurrealQL keywords in a `SELECT`
//! projection. The extractor speaks `from`/`to`; we remap at both boundaries
//! so callers keep the natural vocabulary.

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::backend::KnowledgeBackend;

/// Max BFS depth for neighbor traversal (guards runaway graph walks).
pub const MAX_TRAVERSE_DEPTH: usize = 4;

/// Code knowledge-graph facade.
pub struct CodeGraphStore {
    store: Arc<dyn KnowledgeBackend>,
}

impl CodeGraphStore {
    pub fn new(store: Arc<dyn KnowledgeBackend>) -> Self {
        Self { store }
    }

    /// Replace a project's graph with freshly-extracted entities + relations.
    /// Entity JSON: `{kind,name,file,line,lang}`. Relation JSON (extractor
    /// vocabulary): `{from,kind,to,file}` — stored as `{src,kind,dst,file}`.
    /// Returns (entities, relations) counts.
    pub async fn rebuild(
        &self,
        project: &str,
        entities: &[serde_json::Value],
        relations: &[serde_json::Value],
    ) -> Result<(usize, usize)> {
        // Clear prior graph for this project.
        self.store.query_json(
            "DELETE code_entity WHERE project = $p; DELETE code_rel WHERE project = $p;",
            serde_json::json!({ "p": project }),
        ).await.context("graph clear failed")?;

        for e in entities {
            let mut row = e.clone();
            if let serde_json::Value::Object(m) = &mut row {
                m.insert("project".into(), serde_json::Value::String(project.into()));
            }
            self.store.query_json("CREATE code_entity CONTENT $d", serde_json::json!({ "d": row }))
                .await.context("entity insert failed")?;
        }
        for r in relations {
            // Remap from/to → src/dst (keyword-safe) and stamp project.
            let from = r.get("from").and_then(|v| v.as_str()).unwrap_or("");
            let to = r.get("to").and_then(|v| v.as_str()).unwrap_or("");
            let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let file = r.get("file").and_then(|v| v.as_str()).unwrap_or("");
            let row = serde_json::json!({
                "src": from, "dst": to, "kind": kind, "file": file, "project": project,
            });
            self.store.query_json("CREATE code_rel CONTENT $d", serde_json::json!({ "d": row }))
                .await.context("relation insert failed")?;
        }
        Ok((entities.len(), relations.len()))
    }

    /// Look up entities by exact name (a name may have several defs/impls).
    pub async fn entity(&self, project: &str, name: &str) -> Result<Vec<serde_json::Value>> {
        self.store.query_json(
            "SELECT kind, name, file, line, lang FROM code_entity WHERE project = $p AND name = $n",
            serde_json::json!({ "p": project, "n": name }),
        ).await.context("entity query failed")
    }

    /// Direct relations touching `name` (outgoing + incoming), in extractor
    /// vocabulary (`from`/`to`).
    pub async fn relations(&self, project: &str, name: &str) -> Result<Vec<serde_json::Value>> {
        let rows = self.store.query_json(
            "SELECT src, dst, kind, file FROM code_rel WHERE project = $p AND (src = $n OR dst = $n)",
            serde_json::json!({ "p": project, "n": name }),
        ).await.context("relations query failed")?;
        Ok(rows.into_iter().map(Self::to_extractor_shape).collect())
    }

    /// Pathfinder: BFS from `seed` over the relation graph up to `depth`,
    /// returning reachable node names with their hop distance. Edges are
    /// treated undirected for reachability (relevance, not call direction).
    pub async fn neighbors(&self, project: &str, seed: &str, depth: usize) -> Result<Vec<serde_json::Value>> {
        let depth = depth.min(MAX_TRAVERSE_DEPTH);
        // Load the project's edges once; BFS in-memory (graphs are small).
        let edges = self.store.query_json(
            "SELECT src, dst, kind FROM code_rel WHERE project = $p",
            serde_json::json!({ "p": project }),
        ).await.context("neighbors edge load failed")?;

        let mut adj: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for e in &edges {
            let f = e.get("src").and_then(|v| v.as_str()).unwrap_or("");
            let t = e.get("dst").and_then(|v| v.as_str()).unwrap_or("");
            let k = e.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if f.is_empty() || t.is_empty() { continue; }
            adj.entry(f.into()).or_default().push((t.into(), k.clone()));
            adj.entry(t.into()).or_default().push((f.into(), k));
        }

        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(seed.to_string());
        let mut q: VecDeque<(String, usize)> = VecDeque::new();
        q.push_back((seed.to_string(), 0));
        let mut out = Vec::new();
        while let Some((node, d)) = q.pop_front() {
            if d >= depth { continue; }
            if let Some(nbrs) = adj.get(&node) {
                for (next, kind) in nbrs {
                    if seen.insert(next.clone()) {
                        out.push(serde_json::json!({ "name": next, "via": kind, "hops": d + 1 }));
                        q.push_back((next.clone(), d + 1));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Remap a stored `{src,dst,...}` row back to extractor `{from,to,...}`.
    fn to_extractor_shape(mut v: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = v.as_object_mut() {
            if let Some(src) = obj.remove("src") { obj.insert("from".into(), src); }
            if let Some(dst) = obj.remove("dst") { obj.insert("to".into(), dst); }
        }
        v
    }
}
