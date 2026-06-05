//! Knowledge carry-over: export the learned brain to git-friendly,
//! human-readable files under `<repo>/.swarm`, and re-import (re-embedding)
//! into a fresh DB.
//!
//!   .swarm/memory/patterns/<key>.md   distilled goal→plan (frontmatter + body)
//!   .swarm/memory/errors/<key>.md     failure signatures
//!   .swarm/memory/trajectories.jsonl  raw run trajectories (append-only)
//!   .swarm/routing_stats.json         learned bandit stats
//!   .swarm/KNOWLEDGE.md               generated human digest (tables)
//!   .swarm/runs/<ts>-<slug>.md        agent transcripts (goal→plan→diff→gate)
//!
//! Embeddings are DERIVED (768-dim from nomic) and never written to git — the
//! text is canonical, `import` recreates the vectors via MemoryStore::store.
//! Optional `auto_commit` snapshots `.swarm` onto `commit_branch` via a
//! throwaway worktree (the live checkout is never touched).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use inference_client::OllamaBackend;
use knowledge_base::{KnowledgeBackend, MemoryEntry, MemoryStore};
use swarm_config::KnowledgeConfig;

const SWARM_DIR: &str = ".swarm";
const MAX_RUN_TRANSCRIPTS: usize = 60;
const MAX_DIFF_CHARS: usize = 4000;

fn field(v: &serde_json::Value, k: &str) -> String {
    match v.get(k) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Filesystem-safe slug for a memory key / run title.
fn safe(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    out.trim_matches('-').chars().take(80).collect()
}

pub fn spawn(
    cfg: KnowledgeConfig,
    store: Arc<dyn KnowledgeBackend>,
    ollama: Arc<OllamaBackend>,
    embed_model: String,
    repo: PathBuf,
) {
    if !cfg.export && !cfg.import_on_start {
        info!("Knowledge sync disabled ([knowledge] export/import_on_start both false)");
        return;
    }
    tokio::spawn(async move {
        let mem = MemoryStore::new(Arc::clone(&store), ollama, embed_model);
        if cfg.import_on_start {
            match import_if_empty(&mem, store.as_ref(), &repo).await {
                0 => {}
                n => info!(entries = n, "Knowledge import: seeded memory from .swarm (re-embedded)"),
            }
        }
        if !cfg.export {
            return;
        }
        let interval = Duration::from_secs(cfg.export_interval_secs.max(60));
        info!(interval_secs = interval.as_secs(), auto_commit = cfg.auto_commit, "Knowledge export loop started");
        loop {
            tokio::time::sleep(interval).await;
            match export(store.as_ref(), &repo).await {
                Ok(true) => {
                    info!(dir = %repo.join(SWARM_DIR).display(), "Knowledge exported");
                    if cfg.auto_commit && commit_snapshot(&repo, &cfg.commit_branch) {
                        info!(branch = %cfg.commit_branch, "Knowledge snapshot committed + pushed");
                    }
                }
                Ok(false) => {}
                Err(e) => warn!(error = %e, "Knowledge export failed"),
            }
        }
    });
}

/// Export the live DB to `<repo>/.swarm`. Returns Ok(true) if anything written.
async fn export(store: &dyn KnowledgeBackend, repo: &Path) -> std::io::Result<bool> {
    use std::fs;
    let root = repo.join(SWARM_DIR);
    let mem_dir = root.join("memory");
    fs::create_dir_all(mem_dir.join("patterns"))?;
    fs::create_dir_all(mem_dir.join("errors"))?;
    fs::create_dir_all(root.join("runs"))?;

    // patterns + errors → one markdown file each (frontmatter + body).
    let mut pattern_rows: Vec<serde_json::Value> = Vec::new();
    for (ns, sub) in [("patterns", "patterns"), ("errors", "errors")] {
        let rows = store.query_json(
            &format!("SELECT key, content, project, use_count FROM memory_entry WHERE namespace = '{ns}'"),
            serde_json::Value::Null,
        ).await.unwrap_or_default();
        for r in &rows {
            let key = field(r, "key");
            if key.is_empty() { continue; }
            let uc = r.get("use_count").and_then(|v| v.as_i64()).unwrap_or(0);
            let md = format!(
                "---\nkey: {key}\nproject: {}\nnamespace: {ns}\nuse_count: {uc}\n---\n\n{}\n",
                field(r, "project"), field(r, "content"),
            );
            fs::write(mem_dir.join(sub).join(format!("{}.md", safe(&key))), md)?;
        }
        if ns == "patterns" { pattern_rows = rows; }
    }

    // trajectories → JSONL (raw, append-only style).
    let trj = store.query_json(
        "SELECT key, content, project, created_at FROM memory_entry WHERE namespace = 'trajectories' ORDER BY created_at ASC",
        serde_json::Value::Null,
    ).await.unwrap_or_default();
    let mut jl = String::new();
    for r in &trj {
        let line = serde_json::json!({
            "key": field(r, "key"), "content": field(r, "content"),
            "project": field(r, "project"), "created_at": field(r, "created_at"),
        });
        jl.push_str(&line.to_string());
        jl.push('\n');
    }
    fs::write(mem_dir.join("trajectories.jsonl"), jl)?;

    // routing_stats → JSON.
    let rs = store.query_json(
        "SELECT shape, tier, attempts, successes FROM routing_stats ORDER BY shape",
        serde_json::Value::Null,
    ).await.unwrap_or_default();
    fs::write(root.join("routing_stats.json"), serde_json::to_string_pretty(&rs).unwrap_or_default())?;

    // KNOWLEDGE.md — generated human digest.
    fs::write(root.join("KNOWLEDGE.md"), knowledge_md(&pattern_rows, &rs, trj.len()))?;

    // Run transcripts (recent passed runs).
    let runs = store.query_json(
        "SELECT task_description, model_used, files_modified, diff, response_text, created_at \
             FROM agent_run WHERE status = 'passed' ORDER BY created_at DESC LIMIT 60",
        serde_json::Value::Null,
    ).await.unwrap_or_default();
    for r in runs.iter().take(MAX_RUN_TRANSCRIPTS) {
        fs::write(root.join("runs").join(format!(
            "{}-{}.md",
            field(r, "created_at").chars().take(19).filter(|c| *c != ':').collect::<String>(),
            safe(&field(r, "task_description")),
        )), run_transcript(r))?;
    }
    Ok(true)
}

fn knowledge_md(patterns: &[serde_json::Value], routing: &[serde_json::Value], n_trajectories: usize) -> String {
    let mut s = String::from("# Learned knowledge (SONA)\n\n_Generated snapshot — canonical source is `.swarm/memory/`._\n\n");
    s.push_str(&format!("- patterns: {}\n- routing shapes: {}\n- trajectories: {n_trajectories}\n\n", patterns.len(), routing.len()));
    s.push_str("## Distilled patterns (goal→plan that worked)\n\n| reuse | pattern |\n|---|---|\n");
    let mut ps: Vec<&serde_json::Value> = patterns.iter().collect();
    ps.sort_by_key(|p| -(p.get("use_count").and_then(|v| v.as_i64()).unwrap_or(0)));
    for p in ps.iter().take(40) {
        let uc = p.get("use_count").and_then(|v| v.as_i64()).unwrap_or(0);
        let c: String = field(p, "content").replace('\n', " ").chars().take(110).collect();
        s.push_str(&format!("| {uc}× | {c} |\n"));
    }
    s.push_str("\n## Learned routing (UCB1)\n\n| shape | tier | pass | n |\n|---|---|---|---|\n");
    for r in routing {
        let a = r.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
        let su = r.get("successes").and_then(|v| v.as_i64()).unwrap_or(0);
        let rate = if a > 0 { format!("{:.0}%", su as f64 / a as f64 * 100.0) } else { "—".into() };
        s.push_str(&format!("| {} | {} | {rate} | {a} |\n", field(r, "shape"), field(r, "tier")));
    }
    s
}

fn run_transcript(r: &serde_json::Value) -> String {
    let files = r.get("files_modified").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|f| f.as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let diff: String = field(r, "diff").chars().take(MAX_DIFF_CHARS).collect();
    format!(
        "# {}\n\n- model: {}\n- files: {files}\n- when: {}\n\n## Agent output\n\n```\n{}\n```\n\n## Diff\n\n```diff\n{diff}\n```\n",
        field(r, "task_description"), field(r, "model_used"), field(r, "created_at"),
        field(r, "response_text").chars().take(MAX_DIFF_CHARS).collect::<String>(),
    )
}

/// Seed an EMPTY memory table from `.swarm` (re-embedding via MemoryStore).
/// No-op if memory already has rows (never clobbers live learning).
async fn import_if_empty(mem: &MemoryStore, store: &dyn KnowledgeBackend, repo: &Path) -> usize {
    use std::fs;
    let cnt = store.query_json("SELECT count() FROM memory_entry GROUP ALL", serde_json::Value::Null).await.unwrap_or_default();
    let existing = cnt.first().and_then(|r| r.get("count")).and_then(|v| v.as_i64()).unwrap_or(0);
    if existing > 0 {
        return 0;
    }
    let mem_dir = repo.join(SWARM_DIR).join("memory");
    if !mem_dir.exists() {
        return 0;
    }
    let mut n = 0;
    for (ns, sub) in [("patterns", "patterns"), ("errors", "errors")] {
        let Ok(rd) = fs::read_dir(mem_dir.join(sub)) else { continue };
        for e in rd.flatten() {
            let Ok(txt) = fs::read_to_string(e.path()) else { continue };
            let Some((fm, body)) = parse_frontmatter(&txt) else { continue };
            let key = fm_get(&fm, "key");
            if key.is_empty() { continue; }
            let entry = MemoryEntry {
                id: None,
                namespace: ns.to_string(),
                key,
                content: body,
                embedding: Vec::new(),
                metadata: serde_json::Value::Null,
                project: { let p = fm_get(&fm, "project"); if p.is_empty() { "default".into() } else { p } },
                created_at: String::new(),
                last_used_at: String::new(),
                use_count: fm_get(&fm, "use_count").parse().unwrap_or(1),
                ttl_secs: None,
            };
            if mem.store(entry).await.is_ok() { n += 1; }
        }
    }
    if let Ok(txt) = fs::read_to_string(mem_dir.join("trajectories.jsonl")) {
        for line in txt.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let key = field(&v, "key");
            if key.is_empty() { continue; }
            let entry = MemoryEntry {
                id: None,
                namespace: "trajectories".to_string(),
                key,
                content: field(&v, "content"),
                embedding: Vec::new(),
                metadata: serde_json::Value::Null,
                project: { let p = field(&v, "project"); if p.is_empty() { "default".into() } else { p } },
                created_at: String::new(),
                last_used_at: String::new(),
                use_count: 1,
                ttl_secs: None,
            };
            if mem.store(entry).await.is_ok() { n += 1; }
        }
    }
    n
}

/// Minimal `--- k: v ---` frontmatter parser → (fields, body).
fn parse_frontmatter(txt: &str) -> Option<(Vec<(String, String)>, String)> {
    let rest = txt.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let (head, body) = rest.split_at(end);
    let fields = head.lines().filter_map(|l| {
        let (k, v) = l.split_once(':')?;
        Some((k.trim().to_string(), v.trim().to_string()))
    }).collect();
    let body = body.trim_start_matches("\n---").trim_start().to_string();
    Some((fields, body))
}

fn fm_get(fm: &[(String, String)], key: &str) -> String {
    fm.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default()
}

/// Commit `.swarm` onto `branch` via a throwaway worktree (live checkout
/// untouched). Returns true if a commit was made (skips when nothing changed).
fn commit_snapshot(repo: &Path, branch: &str) -> bool {
    use std::process::Command;
    let git = |dir: &Path, args: &[&str]| {
        Command::new("git").args(args).current_dir(dir).output().map(|o| o.status.success()).unwrap_or(false)
    };
    let wt = PathBuf::from("/tmp/alpha-swarm/knowledge-wt");
    let wts = wt.to_string_lossy().to_string();
    let _ = std::fs::remove_dir_all(&wt);
    let _ = git(repo, &["worktree", "prune"]);
    if !git(repo, &["worktree", "add", "--force", "-B", branch, &wts, "HEAD"]) {
        warn!(branch, "knowledge commit: worktree add failed");
        return false;
    }
    let copied = copy_dir(&repo.join(SWARM_DIR), &wt.join(SWARM_DIR)).is_ok();
    let mut committed = false;
    if copied {
        let _ = git(&wt, &["add", SWARM_DIR]);
        let changed = !Command::new("git").args(["diff", "--cached", "--quiet"]).current_dir(&wt)
            .status().map(|s| s.success()).unwrap_or(true);
        committed = changed && git(&wt, &[
            "-c", "user.email=swarm@local", "-c", "user.name=alpha-swarm",
            "commit", "-m", "swarm: knowledge snapshot", "--no-verify",
        ]);
    }
    let _ = git(repo, &["worktree", "remove", "--force", &wts]);
    if committed {
        let _ = git(repo, &["push", "--force-with-lease", "-u", "origin", branch]);
    }
    committed
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::fs;
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}
