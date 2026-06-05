use std::sync::Arc;

use tracing::{info, warn, error};

use inference_client::{InferenceRouter, OllamaBackend};
use knowledge_base::{AgentRun, AttemptRecord, KnowledgeBackend, RunStatus};
use swarm_config::SwarmConfig;

/// Max chars for attempt preview fields
const ATTEMPT_PREVIEW_CHARS: usize = 500;
/// Persistent cargo target dir kept warm across gate runs (incremental builds).
const GATE_TARGET_DIR: &str = "/tmp/alpha-swarm/gate-target";
/// Keywords marking an edit-shaped, low-reasoning goal that the agent tier can
/// plan without the heavier orchestrator (70b) model.
const TRIVIAL_GOAL_KEYWORDS: &[&str] = &[
    "doc comment", "/// ", "add a comment", "rename", "typo", "unused",
    "remove the", "add a doc", "docstring", "spelling", "whitespace",
];
/// Max goal length still eligible for the cheaper planner tier.
const TRIVIAL_GOAL_MAX_CHARS: usize = 240;

/// Conservative heuristic: short + edit-shaped goals plan on the agent tier
/// (qwen2.5-coder:32b) instead of the 70b orchestrator, saving the big-model
/// prefill. Anything ambiguous stays on the big model — and the quality gate is
/// the backstop either way, so a misroute costs a run, never correctness.
fn is_trivial_goal(goal: &str) -> bool {
    if goal.len() > TRIVIAL_GOAL_MAX_CHARS {
        return false;
    }
    let lower = goal.to_lowercase();
    TRIVIAL_GOAL_KEYWORDS.iter().any(|k| lower.contains(k))
}

// --- Learned planner-tier routing (UCB1 contextual bandit over goal shape) ---
/// UCB1 exploration weight: higher = more exploration. ~1.0 is slightly more
/// exploitative than the textbook sqrt(2); fine for a 2-arm tier choice.
const ROUTING_UCB_C: f64 = 1.0;
const ROUTING_TIER_AGENT: &str = "agent";
const ROUTING_TIER_ORCH: &str = "orchestrator";

/// Goal shape — the bucket over which routing outcomes accumulate. Finer than
/// trivial/complex so the bandit can learn e.g. that doc goals never need 70b
/// while some "simple" edits do.
fn goal_shape(goal: &str) -> &'static str {
    let lower = goal.to_lowercase();
    if lower.contains("doc comment") || lower.contains("docstring") || lower.contains("/// ") || lower.contains("add a comment") {
        "doc"
    } else if is_trivial_goal(goal) {
        "simple"
    } else {
        "complex"
    }
}

/// Map a planner model name back to its tier label (for recording outcomes).
fn tier_label_for_model(model: &str, config: &SwarmConfig) -> Option<&'static str> {
    if model == config.tiers.agent.model { Some(ROUTING_TIER_AGENT) }
    else if model == config.tiers.orchestrator.model { Some(ROUTING_TIER_ORCH) }
    else { None }
}

/// Pick the planner tier for a goal via UCB1 over past gate outcomes for the
/// goal's shape. Cold start (no history for the shape) → the heuristic default
/// (complex→orchestrator, else agent). With history, score each tier by
/// `success_rate + C·sqrt(ln(N)/n)`; an untried tier scores +inf so it's tried
/// once — principled explore/exploit, no RNG. Advisory — the gate is the
/// backstop, so a wrong pick costs a run, not correctness.
async fn recommend_planner_tier<'a>(
    store: &dyn KnowledgeBackend,
    project: &str,
    goal: &str,
    config: &'a SwarmConfig,
) -> &'a swarm_config::TierConfig {
    let shape = goal_shape(goal);
    let default_label = if shape == "complex" { ROUTING_TIER_ORCH } else { ROUTING_TIER_AGENT };
    let tier_for = |label: &str| if label == ROUTING_TIER_AGENT { &config.tiers.agent } else { &config.tiers.orchestrator };

    let rows = store.query_json(
        "SELECT tier, attempts, successes FROM routing_stats WHERE project = $p AND shape = $s",
        serde_json::json!({ "p": project, "s": shape }),
    ).await.unwrap_or_default();

    // (label, attempts, successes) for each tier.
    let stats: Vec<(&str, i64, i64)> = [ROUTING_TIER_AGENT, ROUTING_TIER_ORCH].iter().map(|&label| {
        let row = rows.iter().find(|r| r.get("tier").and_then(|v| v.as_str()) == Some(label));
        let a = row.and_then(|r| r.get("attempts")).and_then(|v| v.as_i64()).unwrap_or(0);
        let s = row.and_then(|r| r.get("successes")).and_then(|v| v.as_i64()).unwrap_or(0);
        (label, a, s)
    }).collect();
    let total: i64 = stats.iter().map(|(_, a, _)| a).sum();

    let chosen = if total == 0 {
        default_label
    } else {
        let mut best = default_label;
        let mut best_score = f64::NEG_INFINITY;
        for &(label, a, s) in &stats {
            let score = if a == 0 {
                f64::INFINITY // explore an untried tier first
            } else {
                (s as f64 / a as f64) + ROUTING_UCB_C * ((total as f64).ln() / a as f64).sqrt()
            };
            if score > best_score {
                best_score = score;
                best = label;
            }
        }
        best
    };
    info!(task = %project, goal_shape = shape, chosen_tier = chosen, total_attempts = total, "learned routing: planner tier (UCB1)");
    tier_for(chosen)
}

/// Record a run's outcome against the tier that planned it (best-effort, never
/// fails the run). Read-modify-write upsert keyed by (project, shape, tier).
async fn record_routing(store: &dyn KnowledgeBackend, project: &str, shape: &str, tier: &str, success: bool, ms: u64) {
    let succ = if success { 1 } else { 0 };
    let existing = store.query_json(
        "SELECT id, attempts, successes, total_ms FROM routing_stats WHERE project = $p AND shape = $s AND tier = $t LIMIT 1",
        serde_json::json!({ "p": project, "s": shape, "t": tier }),
    ).await.unwrap_or_default();
    if let Some(row) = existing.into_iter().next() {
        let id = row.get("id").map(|v| v.to_string().trim_matches('"').to_string()).unwrap_or_default();
        let a = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0) + 1;
        let s = row.get("successes").and_then(|v| v.as_i64()).unwrap_or(0) + succ;
        let m = row.get("total_ms").and_then(|v| v.as_i64()).unwrap_or(0) + ms as i64;
        let _ = store.db_query_raw(&format!("UPDATE {id} SET attempts = {a}, successes = {s}, total_ms = {m}")).await;
    } else {
        let _ = store.db_query_raw(&format!(
            "CREATE routing_stats SET project = '{}', shape = '{}', tier = '{}', attempts = 1, successes = {}, total_ms = {}",
            project.replace('\'', ""), shape, tier, succ, ms,
        )).await;
    }
}
use swarm_events::{EventPublisher, SwarmEvent};

// --- Co-edit suggestions (files that historically change together) ---
/// Min co-occurrence count for a file to be suggested.
const COEDIT_MIN_COOCCURRENCE: usize = 2;
/// Max co-edit suggestions injected into the planner prompt.
const COEDIT_MAX_SUGGESTIONS: usize = 6;
/// Max seed files (named in the goal) to expand from.
const COEDIT_MAX_SEEDS: usize = 5;

/// Files named in the goal that match a known repo path (the co-edit seeds).
fn extract_goal_files(goal: &str, repo_files: &[String]) -> Vec<String> {
    repo_files.iter()
        .filter(|f| goal.contains(f.as_str()))
        .take(COEDIT_MAX_SEEDS)
        .cloned()
        .collect()
}

/// Suggest files that historically co-changed with the goal's named files, from
/// passed-run history (`agent_run.files_modified`). Pure co-occurrence stats —
/// surfaces the "edit compiles but breaks the caller" file the plan forgot.
/// Returns a planner-prompt block, or None when there's no seed/signal.
async fn coedit_hint(store: &dyn KnowledgeBackend, project: &str, goal: &str, repo_files: &[String]) -> Option<String> {
    let seeds = extract_goal_files(goal, repo_files);
    if seeds.is_empty() {
        return None;
    }
    let rows = store.query_json(
        "SELECT files_modified FROM agent_run WHERE project = $p AND status = 'passed'",
        serde_json::json!({ "p": project }),
    ).await.ok()?;
    let mut tally: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in &rows {
        let files: Vec<String> = row.get("files_modified")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if files.iter().any(|f| seeds.contains(f)) {
            for f in &files {
                if !seeds.contains(f) {
                    *tally.entry(f.clone()).or_default() += 1;
                }
            }
        }
    }
    let mut suggestions: Vec<(String, usize)> = tally.into_iter()
        .filter(|(_, c)| *c >= COEDIT_MIN_COOCCURRENCE)
        .collect();
    suggestions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    suggestions.truncate(COEDIT_MAX_SUGGESTIONS);
    if suggestions.is_empty() {
        return None;
    }
    let list = suggestions.iter()
        .map(|(f, c)| format!("- {f} (co-changed {c}×)"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("FILES THAT HISTORICALLY CHANGE TOGETHER with the named file(s) — include them if the change needs them:\n{list}"))
}

// --- Graph-expanded scoping (structural neighbors via the code graph) ---
/// Max structurally-related files surfaced to the planner.
const GRAPH_EXPAND_MAX_NEIGHBORS: usize = 8;

/// Expand the goal's named files by one hop over the code knowledge graph
/// (code_entity defines name→file; code_rel relates names), surfacing files
/// that are structurally coupled (callers, trait impls, type users) but may not
/// lexically resemble the goal — the classic "edit compiles but breaks a
/// caller" file embedding RAG misses. Returns a planner-prompt hint, or None.
async fn graph_expand_hint(store: &dyn KnowledgeBackend, project: &str, goal: &str, repo_files: &[String]) -> Option<String> {
    let seeds = extract_goal_files(goal, repo_files);
    if seeds.is_empty() {
        return None;
    }
    // 1. Entity names defined in the seed files.
    let name_rows = store.query_json(
        "SELECT name FROM code_entity WHERE project = $p AND file IN $files",
        serde_json::json!({ "p": project, "files": seeds }),
    ).await.ok()?;
    let seed_names: Vec<String> = name_rows.iter()
        .filter_map(|r| r.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();
    if seed_names.is_empty() {
        return None;
    }
    // 2. Entity names related to the seed entities (1 hop, undirected).
    let rel_rows = store.query_json(
        "SELECT src, dst FROM code_rel WHERE project = $p AND (src IN $names OR dst IN $names)",
        serde_json::json!({ "p": project, "names": seed_names }),
    ).await.ok()?;
    let seed_set: std::collections::HashSet<&str> = seed_names.iter().map(String::as_str).collect();
    let mut related: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in &rel_rows {
        for k in ["src", "dst"] {
            if let Some(n) = r.get(k).and_then(|v| v.as_str())
                && !seed_set.contains(n)
            {
                related.insert(n.to_string());
            }
        }
    }
    if related.is_empty() {
        return None;
    }
    // 3. Files defining those related entities (excluding the seeds).
    let related_vec: Vec<String> = related.into_iter().collect();
    let file_rows = store.query_json(
        "SELECT file FROM code_entity WHERE project = $p AND name IN $names",
        serde_json::json!({ "p": project, "names": related_vec }),
    ).await.ok()?;
    let mut neighbor_files: Vec<String> = file_rows.iter()
        .filter_map(|r| r.get("file").and_then(|v| v.as_str()).map(String::from))
        .filter(|f| !seeds.contains(f))
        .collect();
    neighbor_files.sort();
    neighbor_files.dedup();
    neighbor_files.truncate(GRAPH_EXPAND_MAX_NEIGHBORS);
    if neighbor_files.is_empty() {
        return None;
    }
    let list = neighbor_files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n");
    Some(format!("STRUCTURALLY RELATED FILES (code-graph neighbors of the named file(s) — check if the change affects them):\n{list}"))
}

// --- Adversarial verify (2nd-model semantic review of a passed diff) ---
/// Max chars of the diff fed to the verifier (keeps the prompt bounded).
const VERIFY_DIFF_MAX_CHARS: usize = 4000;
/// Max tokens for the verifier's verdict line.
const VERIFY_MAX_TOKENS: u32 = 256;
/// Max chars of the reject reason carried into the run record.
const VERIFY_REASON_MAX_CHARS: usize = 200;
const VERIFY_SYSTEM: &str = "You are a rigorous, skeptical code reviewer. You are given a GOAL and a unified DIFF. The diff already compiles and passes EXISTING tests, but those tests may be thin or absent — passing is NOT proof of correctness, do not treat it as such. Decide whether the diff DEMONSTRABLY and COMPLETELY accomplishes the goal. REJECT if ANY of: it does not actually do what the goal asks; it is a no-op or only cosmetic when a real change was expected; it removes or weakens existing logic, validation, error handling, or replaces good documentation with worse; it is incomplete (e.g. changes one of several call sites that should all change); it adds no test for new/changed behaviour that warrants one; or it merely looks plausible and you cannot confirm correctness from the diff alone. A clean compile is NOT sufficient. When in doubt, REJECT. Reply with EXACTLY ONE line: 'VERDICT: ACCEPT' or 'VERDICT: REJECT <one short reason>'.";

enum VerifyVerdict {
    Accept,
    Reject(String),
}

/// Run a 2nd-model semantic critique of a gate-passed diff. Can only flag a
/// rejection; on any inference error or ambiguous output it ACCEPTS (the cargo
/// gate already passed — never block on a flaky second opinion). Uses the agent
/// tier (cheaper than the 70b orchestrator).
/// Does the diff add a test (a unit test or an assertion)?
fn diff_adds_test(diff: &str) -> bool {
    diff.lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .any(|l| {
            let t = l[1..].trim();
            t.contains("#[test]") || t.contains("#[cfg(test)]")
                || t.starts_with("assert") || t.contains("assert_eq!") || t.contains("assert!(")
        })
}

/// True only if EVERY changed (+/-) line is a comment/doc/blank — i.e. the diff
/// touches no real code. Trivial + safe → the critic's bar can be lenient.
fn diff_is_doc_only(diff: &str) -> bool {
    let mut any = false;
    for l in diff.lines() {
        let body = if l.starts_with('+') && !l.starts_with("+++") {
            &l[1..]
        } else if l.starts_with('-') && !l.starts_with("---") {
            &l[1..]
        } else {
            continue;
        };
        let t = body.trim();
        if t.is_empty() {
            continue;
        }
        any = true;
        let is_doc = t.starts_with("///") || t.starts_with("//!") || t.starts_with("//")
            || t.starts_with("/*") || t.starts_with('*');
        if !is_doc {
            return false;
        }
    }
    any
}

async fn adversarial_verify(router: &InferenceRouter, config: &SwarmConfig, goal: &str, diff: &str) -> VerifyVerdict {
    if diff.trim().is_empty() {
        return VerifyVerdict::Accept;
    }
    let doc_only = diff_is_doc_only(diff);
    let cov = if diff_adds_test(diff) {
        "The diff INCLUDES a test exercising the change."
    } else {
        "The diff does NOT include any test."
    };
    let diff_snippet: String = diff.chars().take(VERIFY_DIFF_MAX_CHARS).collect();
    let user = format!("GOAL: {goal}\n\n{cov}\n\nDIFF:\n{diff_snippet}");
    let messages = vec![
        inference_client::ChatMessage::system(VERIFY_SYSTEM),
        inference_client::ChatMessage::user(user),
    ];
    let options = inference_client::InferenceOptions {
        max_tokens: Some(VERIFY_MAX_TOKENS),
        // Use the WARM fast tier (planner/14b), NOT the agent escalation tier:
        // verify runs on every gate-passing run, so a cold-loading 32b here would
        // re-thrash the limited Ollama slots. The rigorous reject-on-doubt prompt
        // carries the judgement; a warm 14b critic stays fast + keeps the loop stable.
        preferred_model: Some(config.tiers.orchestrator.model.clone()),
        preferred_backend: Some(inference_client::BackendKind::Ollama),
        ..Default::default()
    };
    match router.chat(&messages, inference_client::Complexity::Medium, &options).await {
        Ok(resp) => {
            let upper = resp.content.to_uppercase();
            let rejected = upper.contains("VERDICT: REJECT") || upper.contains("VERDICT:REJECT");
            let accepted = upper.contains("VERDICT: ACCEPT") || upper.contains("VERDICT:ACCEPT");
            if rejected {
                let reason: String = resp.content.lines()
                    .find(|l| l.to_uppercase().contains("REJECT"))
                    .unwrap_or("semantic verify rejected")
                    .chars().take(VERIFY_REASON_MAX_CHARS).collect();
                VerifyVerdict::Reject(reason)
            } else if accepted || doc_only {
                // Explicit accept, OR an ambiguous verdict on a trivial doc-only
                // change (don't over-reject safe comment edits).
                VerifyVerdict::Accept
            } else {
                // Reject-on-doubt: a substantive change the critic could not
                // explicitly confirm correct is NOT trusted (gate already passed,
                // so this only downgrades).
                VerifyVerdict::Reject("verify inconclusive — correctness not confirmed".into())
            }
        }
        Err(e) => {
            // Infra failure ≠ bad code — never block on a flaky second opinion.
            warn!(error = %e, "adversarial verify inference failed — accepting (gate already passed)");
            VerifyVerdict::Accept
        }
    }
}

fn format_update(task_id: &str, set_clause: &str) -> String {
    if task_id.contains(':') {
        format!("UPDATE {} {}", task_id, set_clause)
    } else {
        format!("UPDATE type::thing('agent_run', '{}') {}", task_id, set_clause)
    }
}

fn format_update_where(task_id: &str, set_clause: &str, where_clause: &str) -> String {
    if task_id.contains(':') {
        format!("UPDATE {} {} WHERE {}", task_id, set_clause, where_clause)
    } else {
        format!("UPDATE type::thing('agent_run', '{}') {} WHERE {}", task_id, set_clause, where_clause)
    }
}

/// Real quality gate: materialize the run's changed files in a throwaway
/// worktree (the per-task workspaces are gone by now) and run `cargo check` +
/// `cargo test` on the changed crates. Returns Ok(()) only if all pass. This
/// is the gate the runner's disk-mode path stubs out (always-true) — without
/// it the loop merges unverified (even non-compiling) code.
fn run_quality_gate(repo_path: &std::path::Path, gate: &std::path::Path, files: &[(String, Vec<u8>)], sec: &swarm_config::SecurityConfig) -> Result<(), String> {
    use std::process::Command;
    if files.is_empty() {
        return Ok(());
    }
    let g = gate.to_string_lossy().to_string();
    let git = |args: &[&str]| Command::new("git").args(args).current_dir(repo_path).output()
        .map(|o| o.status.success()).unwrap_or(false);
    let _ = git(&["worktree", "prune"]);
    let _ = git(&["worktree", "remove", "--force", &g]);
    if !git(&["worktree", "add", "--force", "--detach", &g, "HEAD"]) {
        return Err("quality gate: worktree add failed".into());
    }
    // Keep a persistent cargo target dir warm across gate runs (incremental
    // builds) via CARGO_TARGET_DIR — NOT a worktree symlink: a dangling symlink
    // makes cargo's create_dir_all fail with ENOTDIR (os error 20).
    let _ = std::fs::create_dir_all(GATE_TARGET_DIR);

    // Write the run's modified files + collect the changed crate packages.
    let mut pkgs: Vec<String> = Vec::new();
    for (p, content) in files {
        let full = gate.join(p);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&full, content);
        if let Some(dir) = p.strip_prefix("crates/").and_then(|r| r.split('/').next()) {
            if let Ok(toml) = std::fs::read_to_string(gate.join("crates").join(dir).join("Cargo.toml")) {
                if let Some(name) = toml.lines().find_map(|l| {
                    l.trim().strip_prefix("name = \"").and_then(|s| s.strip_suffix('"'))
                }) {
                    if !pkgs.iter().any(|x| x == name) { pkgs.push(name.to_string()); }
                }
            }
        }
    }

    let cargo = |args: &[&str]| -> Result<(), String> {
        let out = Command::new("cargo").args(args).current_dir(&gate)
            .env("CARGO_TARGET_DIR", GATE_TARGET_DIR).output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) }
        else { Err(format!("`cargo {}` failed: {}", args.join(" "),
            String::from_utf8_lossy(&out.stderr).lines().rev().take(4).collect::<Vec<_>>().join(" "))) }
    };

    let mut verdict = Ok(());
    if pkgs.is_empty() {
        verdict = cargo(&["check"]); // non-crate edits → workspace check
    } else {
        // Fast-fail: cheap `cargo check -p` first (most rejected edits are
        // non-compiling — E0433 etc.); skips the test-harness build/link cost
        // on the common failure. Only on a clean check do we build + run tests.
        for pkg in &pkgs {
            if verdict.is_ok() { verdict = cargo(&["check", "-p", pkg]); }
        }
        for pkg in &pkgs {
            if verdict.is_ok() { verdict = cargo(&["test", "-p", pkg]); } // compiles incl. #[cfg(test)] + runs
        }
    }
    // Final tier: deterministic security scan over the run's added lines. Runs
    // only on a clean check+test (so it strictly raises the bar) and before the
    // worktree is torn down (so it can read HEAD via `git show`).
    if verdict.is_ok() && sec.rules_enabled {
        verdict = crate::security_scan::scan(files, gate, sec);
    }
    let _ = git(&["worktree", "remove", "--force", &g]);
    verdict
}

/// Force the managed clone onto the project's configured branch. `ensure_repo`
/// only tracks the repo's default branch, so without this agents edit the wrong
/// (stale) branch and landing clobbers. Best-effort; logged on failure.
async fn sync_repo_to_branch(store: &dyn knowledge_base::KnowledgeBackend, project: &str, repo_path: &std::path::Path) {
    let branch = store
        .query_json("SELECT branch FROM project WHERE name = $p", serde_json::json!({ "p": project }))
        .await
        .ok()
        .and_then(|rows| rows.first().and_then(|r| r.get("branch")).and_then(|b| b.as_str()).map(String::from))
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".to_string());
    let git = |args: &[&str]| {
        std::process::Command::new("git").args(args).current_dir(repo_path).output()
            .map(|o| o.status.success()).unwrap_or(false)
    };
    let _ = git(&["fetch", "origin", &branch]);
    // Prefer the accumulating loop branch (swarm/auto) when it exists upstream,
    // so serial edits to the same file build on each other instead of
    // clobbering. Falls back to the project branch (and the lander creates
    // swarm/auto from it on the first pass).
    let _ = git(&["fetch", "origin", "swarm/auto"]);
    let branch = if git(&["rev-parse", "--verify", "--quiet", "origin/swarm/auto"]) {
        "swarm/auto".to_string()
    } else {
        branch
    };
    let origin_ref = format!("origin/{branch}");
    if git(&["checkout", "-B", &branch, &origin_ref]) || git(&["checkout", &branch]) {
        let _ = git(&["reset", "--hard", &origin_ref]);
        let _ = git(&["clean", "-fd"]);
        info!(project, %branch, "synced managed clone");
    } else {
        warn!(project, %branch, "could not checkout branch; using clone default");
    }
}

/// Land a passed run's changes in the source repo via a throwaway git worktree
/// (the live checkout is never touched). Two modes:
///   - `issue = Some(n)` (GitHub ticket) → a FRESH per-issue branch
///     `swarm/issue-n` reset to base, with a PR whose body says `Fixes #n` so
///     merging auto-closes the issue (true 1:1).
///   - `issue = None` (manual goal) → accumulate on the shared `swarm/auto`
///     branch + one rolling PR.
/// Local-path repos only (remote URLs left to PR mode). Best-effort; logs on.
fn land_to_branch(repo_url: &str, base_branch: &str, run_id: &str, goal: &str, files: &[(String, Vec<u8>)], issue: Option<i64>) {
    if files.is_empty() || repo_url.contains("://") {
        return;
    }
    let repo = std::path::Path::new(repo_url);
    if !repo.join(".git").exists() {
        return;
    }
    let safe_id = run_id.replace([':', '/'], "_");
    let land_dir = std::path::PathBuf::from(format!("/tmp/alpha-swarm/land/{safe_id}"));
    let ld = land_dir.to_string_lossy().to_string();
    let _ = std::fs::remove_dir_all(&land_dir);
    let git = |dir: &std::path::Path, args: &[&str]| {
        std::process::Command::new("git").args(args).current_dir(dir).output()
            .map(|o| o.status.success()).unwrap_or(false)
    };
    let _ = git(repo, &["worktree", "prune"]);

    let branch = match issue {
        Some(n) => format!("swarm/issue-{n}"),
        None => "swarm/auto".to_string(),
    };
    let added = match issue {
        // Per-issue: fresh branch reset to base each run (one ticket = one PR).
        Some(_) => git(repo, &["worktree", "add", "--force", "-B", &branch, &ld, "HEAD"]),
        // Aggregate: reuse swarm/auto (accumulate), else create from HEAD.
        None => git(repo, &["worktree", "add", "--force", &ld, &branch])
            || git(repo, &["worktree", "add", "--force", "-b", &branch, &ld, "HEAD"]),
    };
    if !added {
        warn!(run_id, %branch, "land: could not create worktree");
        return;
    }
    for (path, content) in files {
        let full = land_dir.join(path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&full, content);
    }
    let _ = git(&land_dir, &["add", "-A"]);
    let goal_short: String = goal.chars().take(60).collect();
    let msg = match issue {
        Some(n) => format!("swarm: {goal_short} (#{n}) [{run_id}]"),
        None => format!("swarm: {goal_short} [{run_id}]"),
    };
    let committed = git(&land_dir, &[
        "-c", "user.email=swarm@local", "-c", "user.name=alpha-swarm",
        "commit", "-m", &msg, "--no-verify",
    ]);
    let _ = git(repo, &["worktree", "remove", "--force", &ld]);
    if !committed {
        return;
    }
    info!(run_id, %branch, files = files.len(), "landed changes");
    // Push + ensure a PR exists (needs an 'origin' on a hosting service + gh
    // auth — silently skipped otherwise). Per-issue branches are reset each run,
    // so force-with-lease; the aggregate branch accumulates with a plain push.
    let pushed = match issue {
        Some(_) => git(repo, &["push", "--force-with-lease", "-u", "origin", &branch]),
        None => git(repo, &["push", "-u", "origin", &branch]),
    };
    if !pushed {
        return;
    }
    let (title, body) = match issue {
        Some(n) => (
            format!("swarm: {goal_short}"),
            format!("Fixes #{n}\n\n{}\n\n🤖 Autonomous change by alpha-swarm — quality-gated (cargo check/test). Review before merge.",
                goal.chars().take(500).collect::<String>()),
        ),
        None => (
            "swarm: autonomous loop changes".to_string(),
            "Quality-gated changes accumulated by the autopilot loop. Review before merge.".to_string(),
        ),
    };
    let pr = std::process::Command::new("gh")
        .args(["pr", "create", "--base", base_branch, "--head", &branch, "--title", &title, "--body", &body])
        .current_dir(repo)
        .output();
    if matches!(pr, Ok(ref o) if o.status.success()) {
        info!(run_id, base = base_branch, %branch, "opened PR");
    } // else: PR already exists — the push updated it.
}

fn discover_source_files(repo_path: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    let extensions = ["rs", "ts", "js", "go", "py", "md", "toml", "json", "yaml", "yml"];
    fn walk(dir: &std::path::Path, base: &std::path::Path, ext: &[&str], out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with('.') || name == "target" || name == "node_modules" || name == "dist" { continue; }
                walk(&path, base, ext, out);
            } else if let Some(e) = path.extension().and_then(|e| e.to_str())
                && ext.contains(&e)
                && let Ok(rel) = path.strip_prefix(base) {
                    out.push(rel.to_string_lossy().to_string());
            }
        }
    }
    walk(repo_path, repo_path, &extensions, &mut files);
    files.sort();
    files
}

/// Dispatch a task based on its status: planning, approved, or pending (legacy).
#[allow(clippy::too_many_arguments)]
pub async fn handle_task(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    publisher: Option<Arc<EventPublisher>>,
    engine: Arc<swarm_workflow::WorkflowEngine>,
    task_id: &str,
    project: &str,
    goal: &str,
    status: &str,
) {
    match status {
        "planning" => handle_planning(config, router, ollama, store, task_id, project, goal).await,
        "approved" => handle_approved(config, router, ollama, store, publisher, engine, task_id, project, goal).await,
        _ => handle_execute(config, router, ollama, store, publisher, engine, task_id, project, goal).await,
    }
}

/// Convert a persisted `PlannedTask` back into the runner's `SubTask`.
fn planned_to_subtask(t: &knowledge_base::PlannedTask) -> swarm_orchestrator::SubTask {
    let complexity = match t.complexity.to_lowercase().as_str() {
        "medium" => inference_client::Complexity::Medium,
        "complex" => inference_client::Complexity::Complex,
        _ => inference_client::Complexity::Simple,
    };
    swarm_orchestrator::SubTask {
        id: t.id.clone(),
        description: t.description.clone(),
        files: t.files.clone(),
        complexity,
        depends_on: t.depends_on.clone(),
        edit: t.edit.as_ref().map(|e| swarm_orchestrator::planner_types::DirectEdit {
            path: e.path.clone(),
            old: e.old.clone(),
            new: e.new.clone(),
        }),
        template: t.template.clone(),
    }
}

/// Planning-only: generate plan, store it, set status to 'planned', STOP.
async fn handle_planning(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    task_id: &str,
    project: &str,
    goal: &str,
) {
    info!(task_id, project, goal, "Planning only (no execution)");

    // Claim
    let now = chrono::Utc::now().to_rfc3339();
    let claim = format_update(task_id, &format!("SET status = 'running', agent_id = 'planner', last_activity_at = '{}', progress_message = 'Planning goal decomposition...'", now));
    if store.db_query_raw(&claim).await.is_err() { return; }

    // Look up repo
    let repo_url = match store.get_project_repo(project).await {
        Ok(Some(url)) => url,
        _ => {
            let _ = store.db_query_raw(&format_update(task_id, "SET status = 'failed', error_message = 'No repo URL for project'")).await;
            return;
        }
    };

    let git = crate::provider_client::GitProviderClient::new(&config.nats.url).await;
    let base_path = match git.ensure_repo(project, &repo_url).await {
        Ok(p) => std::path::PathBuf::from(p),
        Err(e) => {
            let _ = store.db_query_raw(&format_update(task_id, &format!("SET status = 'failed', error_message = 'Git clone failed: {}'", e.replace('\'', "")))).await;
            return;
        }
    };
    // Per-run isolated workspace (reused by this run's later execution) so
    // concurrent planning never races on the shared base clone.
    let repo_path = crate::repo::isolate_run_workspace(&base_path, task_id, &repo_url).unwrap_or(base_path);
    sync_repo_to_branch(store.as_ref(), project, &repo_path).await;

    // Discover files and plan
    let repo_files = discover_source_files(&repo_path);
    let start = std::time::Instant::now();

    // Check for previous plans and feedback (for iterative re-planning)
    let previous_plans = store.get_plans(task_id).await.unwrap_or_default();
    let version = previous_plans.last().map(|p| p.version + 1).unwrap_or(1);

    // Build planning prompt with full conversation history
    let plan_goal = if previous_plans.is_empty() {
        goal.to_string()
    } else {
        let mut context = format!("{}\n\n", goal);
        for plan in &previous_plans {
            // Show previous plan
            let task_list: Vec<String> = plan.sub_tasks.iter()
                .map(|t| format!("  - {}: {} (files: {:?}, {})", t.id, t.description, t.files, t.complexity))
                .collect();
            context.push_str(&format!("PREVIOUS PLAN (v{}):\n{}\n\n", plan.version, task_list.join("\n")));

            // Show user feedback if any
            if let Some(fb) = &plan.user_feedback {
                context.push_str(&format!("USER FEEDBACK:\n{}\n\n", fb));
            }
        }
        context.push_str("Generate an improved plan addressing all feedback. Output ONLY the JSON array.");
        context
    };

    // Update progress with iteration info
    let progress = if version > 1 {
        format!("Re-planning (v{}) with user feedback...", version)
    } else {
        "Planning goal decomposition...".to_string()
    };
    let _ = store.db_query_raw(&format_update(task_id, &format!("SET progress_message = '{}'", progress.replace('\'', "")))).await;

    // SONA: feedback capture + retrieval of past proven plans.
    let memory = config.learning.enabled.then(|| knowledge_base::MemoryStore::new(
        Arc::clone(&store), Arc::clone(&ollama), config.defaults.embed_model.clone(),
    ));

    // User feedback on a previous plan version is high-signal — persist it
    // into the feedback namespace keyed by goal shape.
    if let Some(memory) = &memory
        && let Some(fb) = previous_plans.last().and_then(|p| p.user_feedback.clone()) {
            let now = chrono::Utc::now().to_rfc3339();
            let entry = knowledge_base::MemoryEntry {
                id: None,
                namespace: knowledge_base::MEM_NS_FEEDBACK.into(),
                key: task_id.to_string(),
                content: format!("GOAL: {goal}\nFEEDBACK: {fb}"),
                embedding: Vec::new(), // embedded from content on store
                metadata: serde_json::json!({ "run_id": task_id }),
                project: project.to_string(),
                created_at: now.clone(),
                last_used_at: now,
                use_count: 0,
                ttl_secs: None,
            };
            if let Err(e) = memory.store(entry).await {
                warn!(task_id, error = %e, "feedback memory store failed");
            }
    }

    // Retrieve past proven plans for similar goals and inject as guidance.
    // Weighted by closed-loop effectiveness — proven patterns rank higher.
    let (past_plans_block, retrieved_pattern_ids) = if let Some(memory) = &memory {
        let budget = config.learning.proven_plans_char_budget;
        let min_sim = config.learning.min_similarity;
        let top_k = config.learning.max_proven_plans;
        let mut block = String::new();
        let mut ids = Vec::new();

        // Proven plans (effectiveness-weighted) — reuse what worked.
        let proven = [knowledge_base::MEM_NS_PATTERNS, knowledge_base::MEM_NS_SOLUTIONS];
        if let Ok(hits) = memory.search_text_weighted(&proven, project, goal, top_k).await {
            for hit in &hits {
                if hit.similarity < min_sim { continue; }
                if block.len() + hit.entry.content.len() > budget { break; }
                if block.is_empty() { block.push_str("WORKING PLANS (reuse / adapt to current files):\n"); }
                block.push_str(&format!("- (sim {:.2}) {}\n", hit.similarity, hit.entry.content));
                if let Some(id) = &hit.entry.id { ids.push(id.clone()); }
            }
        }

        // Recent failures for similar goals — negative guidance (plain cosine;
        // error entries have no effectiveness history to weight by). Only the
        // proven `ids` feed effectiveness tracking — errors are not patterns.
        if let Ok(fails) = memory.search_text(&[knowledge_base::MEM_NS_ERRORS], project, goal, top_k).await {
            let mut wrote_header = false;
            for hit in &fails {
                if hit.similarity < min_sim { continue; }
                if block.len() + hit.entry.content.len() > budget { break; }
                if !wrote_header {
                    block.push_str("\nRECENT FAILED ATTEMPTS (do NOT repeat these approaches):\n");
                    wrote_header = true;
                }
                block.push_str(&format!("- (sim {:.2}) {}\n", hit.similarity, hit.entry.content));
            }
        }

        if block.is_empty() { (None, Vec::new()) } else {
            info!(task_id, patterns = ids.len(), "SONA: injecting prior run memory into planner");
            (Some(block), ids)
        }
    } else {
        (None, Vec::new())
    };

    // Planner context hints, each best-effort: prior-run memory (SONA) +
    // historical co-edit files + structural code-graph neighbors. All steer the
    // plan toward the files a change actually needs (fewer gate rejections from
    // a half-change that breaks a caller).
    let coedit = coedit_hint(store.as_ref(), project, goal, &repo_files).await;
    let graph = graph_expand_hint(store.as_ref(), project, goal, &repo_files).await;
    let blocks: Vec<String> = [past_plans_block, coedit, graph].into_iter().flatten().collect();
    let planner_block = if blocks.is_empty() { None } else { Some(blocks.join("\n\n")) };

    // Learned routing: pick the planner tier from past gate outcomes for this
    // goal shape (falls back to the trivial-goal heuristic with no history).
    let planner_tier = recommend_planner_tier(store.as_ref(), project, goal, config).await;
    match swarm_orchestrator::plan_goal(&router, &plan_goal, &repo_files, planner_tier, None, planner_block.as_deref()).await {
        Ok(tasks) => {
            let duration_ms = start.elapsed().as_millis() as u64;

            // Convert SubTasks to PlannedTasks (lossless: DAG edges, template, and
            // direct-edit payload are persisted so approved plans can be executed
            // without re-planning).
            let sub_tasks: Vec<knowledge_base::PlannedTask> = tasks.iter().map(|t| {
                knowledge_base::PlannedTask {
                    id: t.id.clone(),
                    description: t.description.clone(),
                    files: t.files.clone(),
                    complexity: format!("{:?}", t.complexity),
                    rationale: String::new(),
                    depends_on: t.depends_on.clone(),
                    template: t.template.clone(),
                    edit: t.edit.as_ref().map(|e| knowledge_base::PlannedEdit {
                        path: e.path.clone(),
                        old: e.old.clone(),
                        new: e.new.clone(),
                    }),
                }
            }).collect();

            let plan = knowledge_base::GoalPlan {
                id: None,
                run_id: task_id.to_string(),
                project: project.to_string(),
                goal: goal.to_string(),
                version,
                sub_tasks,
                model_used: config.tiers.orchestrator.model.clone(),
                tokens_input: 0,
                tokens_output: 0,
                duration_ms,
                user_feedback: previous_plans.last().and_then(|p| p.user_feedback.clone()),
                status: "draft".to_string(),
                context_files: repo_files,
                web_searches: vec![],
                reasoning: format!("Decomposed into {} sub-tasks", tasks.len()),
                created_at: chrono::Utc::now().to_rfc3339(),
                retrieved_pattern_ids,
            };

            let _ = store.store_plan(&plan).await;
            let msg = format!("Plan v{} ready — {} sub-tasks. Waiting for approval.", version, tasks.len());
            let _ = store.db_query_raw(&format_update(task_id, &format!("SET status = 'planned', progress_message = '{}'", msg.replace('\'', "")))).await;
            info!(task_id, version, tasks = tasks.len(), "Plan generated, awaiting approval");
        }
        Err(e) => {
            let _ = store.db_query_raw(&format_update(task_id, &format!("SET status = 'failed', error_message = 'Planning failed: {}'", e.to_string().replace('\'', "")))).await;
        }
    }
}

/// Execute with an approved plan. `handle_execute` detects the approved plan
/// and routes through the persisted workflow engine (no re-planning).
#[allow(clippy::too_many_arguments)]
async fn handle_approved(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    publisher: Option<Arc<EventPublisher>>,
    engine: Arc<swarm_workflow::WorkflowEngine>,
    task_id: &str,
    project: &str,
    goal: &str,
) {
    info!(task_id, project, "Executing approved plan");
    handle_execute(config, router, ollama, store, publisher, engine, task_id, project, goal).await;
}

/// Standard execution: claim → plan → execute → PR.
#[allow(clippy::too_many_arguments)]
async fn handle_execute(
    config: &SwarmConfig,
    router: Arc<InferenceRouter>,
    ollama: Arc<OllamaBackend>,
    store: Arc<dyn KnowledgeBackend>,
    publisher: Option<Arc<EventPublisher>>,
    engine: Arc<swarm_workflow::WorkflowEngine>,
    task_id: &str,
    project: &str,
    goal: &str,
) {
    info!(task_id, project, goal, "Starting task execution");

    // 1. Claim the task atomically
    let now = chrono::Utc::now().to_rfc3339();
    let claim_query = format_update_where(task_id, &format!("SET status = 'running', agent_id = 'daemon', last_activity_at = '{}'", now), "status IN ['pending', 'approved']");
    match store.db_query_raw(&claim_query).await {
        Ok(_) => {}
        Err(e) => {
            warn!(task_id, "Failed to claim task: {e}");
            return;
        }
    }

    // Emit agent started event
    if let Some(pub_) = &publisher {
        let _ = pub_.publish(&SwarmEvent::AgentStarted {
            project: project.into(),
            agent_id: format!("daemon-{}", &task_id[..task_id.len().min(8)]),
            task: goal.into(),
            model: "auto".into(),
            files: vec![],
            timestamp: SwarmEvent::timestamp(),
        }).await;
    }

    // 2. Look up repo URL
    let repo_url = match store.get_project_repo(project).await {
        Ok(Some(url)) => url,
        Ok(None) => {
            fail_task(store.as_ref(), &publisher, task_id, project, goal, "No repo URL configured for project").await;
            return;
        }
        Err(e) => {
            fail_task(store.as_ref(), &publisher, task_id, project, goal, &format!("Failed to query project: {e}")).await;
            return;
        }
    };

    // 3. Clone/update repo (via git-provider NATS service, local fallback)
    let git = crate::provider_client::GitProviderClient::new(&config.nats.url).await;
    let repo_path_str = match git.ensure_repo(project, &repo_url).await {
        Ok(p) => p,
        Err(e) => {
            fail_task(store.as_ref(), &publisher, task_id, project, goal, &format!("Git clone failed: {e}")).await;
            return;
        }
    };
    // Per-run isolated working copy (git clone --local off the shared base) so
    // parallel runs never share mutable git state (sync/reset/edit). Falls back
    // to the shared base if isolation fails.
    let base_path = std::path::PathBuf::from(&repo_path_str);
    let repo_path = crate::repo::isolate_run_workspace(&base_path, task_id, &repo_url)
        .unwrap_or_else(|e| {
            warn!(task_id, error = %e, "run workspace isolation failed — using shared base");
            base_path
        });
    sync_repo_to_branch(store.as_ref(), project, &repo_path).await;

    info!(task_id, repo = %repo_path.display(), "Repo ready, executing swarm");

    // Lifecycle hooks: fired by the runner (per-task) and below (run-level).
    let hooks = {
        let mut hs = swarm_orchestrator::hooks::HookSet::new();
        hs.register(Arc::new(swarm_orchestrator::hooks::TracingHook));
        if config.learning.enabled {
            let memory = Arc::new(knowledge_base::MemoryStore::new(
                Arc::clone(&store), Arc::clone(&ollama), config.defaults.embed_model.clone(),
            ));
            hs.register(Arc::new(crate::hooks::TrajectoryRecorder::new(
                memory,
                Arc::clone(&store),
                Arc::clone(&router),
                config.tiers.orchestrator.clone(),
                config.learning.clone(),
            )));
        }
        Arc::new(hs)
    };

    // === PHASE TIMING ===
    let phase_start = std::time::Instant::now();
    let embed_ms: u64;

    // Phase 1: Embeddings
    let embed_model = config.defaults.embed_model.clone();
    let emb_manager = std::sync::Arc::new(knowledge_base::embedding_manager::EmbeddingManager::new(
        Arc::clone(&store), Arc::clone(&ollama), embed_model,
    ));
    {
        let indexed = emb_manager.on_agent_start(project, &repo_path).await;
        embed_ms = phase_start.elapsed().as_millis() as u64;
        if indexed > 0 {
            info!(indexed, duration_ms = embed_ms, "Phase 1: Embeddings (indexed)");
        } else {
            info!(duration_ms = embed_ms, "Phase 1: Embeddings (cached)");
        }
        update_progress(store.as_ref(), task_id, &format!("Phase 1: Embeddings done ({}ms)", embed_ms)).await;
    }

    // Helper: update progress on the running task
    async fn update_progress(store: &dyn KnowledgeBackend, task_id: &str, msg: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        let safe_msg = msg.replace('\'', "");
        let query = if task_id.contains(':') {
            format!("UPDATE {} SET last_activity_at = '{}', progress_message = '{}'", task_id, now, safe_msg)
        } else {
            format!("UPDATE type::thing('agent_run', '{}') SET last_activity_at = '{}', progress_message = '{}'", task_id, now, safe_msg)
        };
        let _ = store.db_query_raw(&query).await;
    }

    update_progress(store.as_ref(), task_id, "Planning goal decomposition...").await;

    // 4. Run the swarm orchestrator with retry loop (orchestrator tier fuel)
    let tier = &config.tiers.orchestrator;
    let start = std::time::Instant::now();
    let max_iterations = tier.max_iterations;
    let time_limit_ms: u64 = tier.time_limit_secs * 1000;
    let token_limit: u32 = tier.token_limit;
    let max_backoff = tier.max_backoff_secs;
    info!(task_id, model = %tier.model, time_limit = tier.time_limit_secs, token_limit, max_iterations, "Using orchestrator tier");
    let mut total_tokens_used: u32 = 0;
    let mut iteration = 0;
    let mut last_errors = String::new();
    let mut final_result = None;
    // Pattern ids injected into the approved plan's prompt (SONA signal).
    let mut plan_pattern_ids: Vec<String> = Vec::new();

    loop {
        iteration += 1;
        let elapsed = start.elapsed().as_millis() as u64;

        if elapsed > time_limit_ms {
            warn!(task_id, iteration, "Time fuel exhausted ({elapsed}ms > {time_limit_ms}ms)");
            break;
        }
        if total_tokens_used > token_limit {
            warn!(task_id, iteration, total_tokens_used, "Token fuel exhausted");
            break;
        }
        if iteration > max_iterations {
            warn!(task_id, iteration, "Max iterations reached");
            break;
        }

        // Exponential backoff between retries
        if iteration > 1 {
            let backoff = std::cmp::min(2u64.pow(iteration - 1), max_backoff);
            info!(task_id, iteration, backoff_secs = backoff, errors = %last_errors.chars().take(100).collect::<String>(), "Retrying after backoff");
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        }

        // Build goal with error context from previous iteration
        let augmented_goal = if last_errors.is_empty() {
            goal.to_string()
        } else {
            format!("{}\n\nPREVIOUS ATTEMPT FAILED:\n{}\n\nFix the issues from the previous attempt.", goal, last_errors)
        };

        let progress_msg = if iteration > 1 {
            format!("Retry {}/{} — replanning...", iteration, max_iterations)
        } else {
            "Running agents...".to_string()
        };
        update_progress(store.as_ref(), task_id, &progress_msg).await;

        let wf_control = engine.control_for(task_id).await;
        let mut runner = swarm_orchestrator::SwarmRunner::new(Arc::clone(&router), Arc::clone(&ollama), &repo_path, project);
        runner = runner
            .with_store(Arc::clone(&store))
            .with_parent_run_id(task_id)
            .with_max_concurrent(config.resources.max_concurrent_agents)
            .with_planner_tier(config.tiers.orchestrator.clone())
            .with_agent_tier(config.tiers.agent.clone())
            .with_depth(config.resources.max_sub_plan_depth)
            .with_embed_model(config.defaults.embed_model.clone())
            .with_hooks(Arc::clone(&hooks))
            .with_control(wf_control)
            .with_learning(config.learning.clone());

        // Zero-disk mode: opt-in via ZERO_DISK=1 (not just GITHUB_TOKEN)
        // GITHUB_TOKEN is used for PR creation regardless
        if std::env::var("ZERO_DISK").is_ok() {
            if let Ok(token) = std::env::var("GITHUB_TOKEN") {
                let gh_repo = std::env::var("GITHUB_REPO").unwrap_or_else(|_| "alpha-swarm/alpha-swarm2".into());
                let parts: Vec<&str> = gh_repo.splitn(2, '/').collect();
                if parts.len() == 2 {
                    runner = runner.with_github(swarm_orchestrator::GitHubRepo {
                        owner: parts[0].into(),
                        repo: parts[1].into(),
                        token,
                        branch: "main".into(),
                    });
                    info!("Zero-disk mode enabled (ZERO_DISK=1)");
                }
            }
        }

        // Connect to NATS for distributed tool dispatch (best-effort)
        if let Ok(nats_client) = async_nats::connect(&config.nats.url).await {
            runner = runner.with_nats_client(nats_client);
        }

        let run_start = std::time::Instant::now();
        update_progress(store.as_ref(), task_id, "Phase 2: Planning + Agent execution...").await;

        // Workflow path: an approved plan with persisted steps executes through
        // the workflow engine (resumable, replans on step failure — never
        // re-plans from scratch). Legacy goals fall back to runner.run().
        // NOTE: approval is recorded on agent_run.status (the approve route
        // does not touch goal_plan.status) — a run reaching execution with a
        // persisted plan means that plan IS the approved plan.
        let approved_tasks: Option<Vec<swarm_orchestrator::SubTask>> = if iteration == 1 {
            match store.get_latest_plan(task_id).await {
                Ok(Some(plan)) if !plan.sub_tasks.is_empty() => {
                    plan_pattern_ids = plan.retrieved_pattern_ids.clone();
                    Some(plan.sub_tasks.iter().map(planned_to_subtask).collect())
                }
                _ => None,
            }
        } else {
            None
        };

        let exec_result = if let Some(tasks) = approved_tasks {
            info!(task_id, steps = tasks.len(), "Executing via workflow engine (approved plan)");
            match run_workflow(&engine, store.as_ref(), &runner, &router, config, task_id, project, goal, &repo_path, tasks).await {
                Ok(Some(result)) => Ok(result),
                Ok(None) => {
                    // Paused or cancelled — workflow_run row is the durable
                    // state; this task releases its locks and exits.
                    info!(task_id, "Workflow paused/cancelled — exiting executor");
                    return;
                }
                Err(e) => Err(e),
            }
        } else {
            runner.run(&augmented_goal).await
        };

        match exec_result {
            Ok(result) => {
                let run_ms = run_start.elapsed().as_millis() as u64;
                let total_ms = phase_start.elapsed().as_millis() as u64;
                let pt = &result.phase_timings;
                info!(task_id, run_ms, total_ms, quality = result.quality_passed,
                    tasks = result.tasks.len(),
                    rag_ms = pt.rag_ms, planning_ms = pt.planning_ms,
                    agent_ms = pt.agent_execution_ms, qg_ms = pt.quality_gate_ms,
                    "Phase 2+3: Plan + Execute + QG complete");
                info!(task_id, summary = %pt.summary(), "Phase timing breakdown");
                // Track token usage
                let iter_tokens: u32 = result.results.iter()
                    .filter_map(|r| r.agent_result.as_ref())
                    .map(|a| a.inference_response.tokens_input + a.inference_response.tokens_output)
                    .sum();
                total_tokens_used += iter_tokens;

                if result.quality_passed {
                    let tasks_done = result.results.iter().filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied)).count();
                    update_progress(store.as_ref(), task_id, &format!(
                        "Quality passed — {} tasks done, creating PR... [{}]",
                        tasks_done, pt.summary()
                    )).await;
                    info!(task_id, iteration, total_tokens_used, "Quality gate passed!");
                    final_result = Some(result);
                    break;
                } else {
                    // Collect errors for next iteration
                    last_errors = result.results.iter()
                        .filter_map(|r| r.error.as_ref())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    info!(task_id, iteration, total_tokens_used, "Quality gate failed, will retry");
                    final_result = Some(result);
                    // Reset repo to clean state for next attempt
                    let _ = std::process::Command::new("git")
                        .args(["checkout", "."])
                        .current_dir(&repo_path)
                        .output();
                }
            }
            Err(e) => {
                last_errors = e.to_string();
                warn!(task_id, iteration, error = %e, "Swarm execution failed");
                final_result = None;
            }
        }
    }

    let duration = start.elapsed().as_millis() as u64;

    let start_time_rfc3339 = chrono::Utc::now().checked_sub_signed(chrono::Duration::milliseconds(duration as i64))
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    match final_result {
        Some(result) => {
            let tasks_passed = result.results.iter().filter(|r| r.agent_result.as_ref().is_some_and(|a| a.applied)).count();
            let tasks_failed = result.results.iter().filter(|r| r.error.is_some()).count();
            // Check if any agent produced work — either via edits or via tool-based file writes
            // A run "did work" only if it produced REAL captured file changes
            // (git diff in the workspace, cargo's own Cargo.lock churn stripped
            // upstream). The agent CLAIMING tasks passed (tasks_passed > 0) is
            // not enough: a failed-to-apply edit reports success but leaves no
            // diff, and gating an empty change set trivially "passes" because the
            // unchanged baseline still compiles. Require an actual diff.
            let any_work_done = !result.modified_files.is_empty();

            // Real quality gate: materialize the changed files + `cargo
            // check/test` the changed crates. The runner's disk-mode
            // quality_passed is a stub (always true), so the actual verification
            // happens HERE before anything lands.
            let gate = if any_work_done {
                run_quality_gate(&repo_path, &crate::repo::run_gate_path(task_id), &result.modified_files, &config.security)
            } else {
                Err("no changes produced".to_string())
            };
            if let Err(ref e) = gate {
                warn!(task_id, reason = %e, "quality gate FAILED — not landing");
            }
            let status = if gate.is_ok() { RunStatus::Passed } else { RunStatus::Failed };
            // The REAL gate verdict (not the runner's always-true disk stub). Feed
            // this into learning + events so distillation/effectiveness are gated on
            // "compiled + tests passed", not "an edit applied".
            let gate_passed = matches!(status, RunStatus::Passed);

            // Adversarial verify (opt-in): a 2nd-model semantic critique that can
            // only DOWNGRADE a passed run to Failed (never upgrade a failed gate),
            // so it strictly raises the bar; the cargo gate stays the backstop.
            let mut verify_reason: Option<String> = None;
            let (status, gate_passed) = if gate_passed && config.learning.verify_diffs {
                match adversarial_verify(&router, config, goal, result.merged_diff.as_deref().unwrap_or("")).await {
                    VerifyVerdict::Reject(reason) => {
                        warn!(task_id, reason = %reason, "adversarial verify REJECTED — downgrading to Failed");
                        verify_reason = Some(format!("verify-reject: {reason}"));
                        (RunStatus::Failed, false)
                    }
                    VerifyVerdict::Accept => (status, gate_passed),
                }
            } else {
                (status, gate_passed)
            };

            // Land verified changes onto swarm/auto (+ push + PR) for review/merge.
            if matches!(status, RunStatus::Passed) {
                let base_branch = store
                    .query_json("SELECT branch FROM project WHERE name = $p", serde_json::json!({ "p": project }))
                    .await.ok()
                    .and_then(|rows| rows.first().and_then(|r| r.get("branch")).and_then(|b| b.as_str()).map(String::from))
                    .filter(|b| !b.is_empty())
                    .unwrap_or_else(|| "main".to_string());
                // If this run came from a GitHub ticket ("owner/repo#N"), land on
                // a per-issue branch + PR ("Fixes #N"); else accumulate on swarm/auto.
                let issue = store
                    .query_json(&format!("SELECT external_id FROM {task_id}"), serde_json::Value::Null)
                    .await.ok()
                    .and_then(|rows| rows.first().and_then(|r| r.get("external_id")).and_then(|v| v.as_str()).map(String::from))
                    .filter(|s| !s.is_empty())
                    .and_then(|ext| ext.rsplit('#').next().and_then(|n| n.parse::<i64>().ok()));
                land_to_branch(&repo_url, &base_branch, task_id, goal, &result.modified_files, issue);
            }

            // Learned routing: attribute this run's gate outcome to the tier that
            // planned it (best-effort; advisory stats only).
            if let Ok(plans) = store.get_plans(task_id).await
                && let Some(model) = plans.last().map(|p| p.model_used.clone())
                && let Some(tier_label) = tier_label_for_model(&model, config)
            {
                record_routing(store.as_ref(), project, goal_shape(goal), tier_label, gate_passed, duration).await;
            }

            info!(
                task_id, project,
                quality_passed = gate_passed,
                any_work_done,
                tasks = result.tasks.len(),
                tasks_passed,
                tasks_failed,
                duration_ms = duration,
                "Swarm completed"
            );

            // Collect actual models used by sub-agents
            let models_used: Vec<String> = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .map(|a| a.inference_response.model.clone())
                .filter(|m| !m.is_empty())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let model_str = if models_used.is_empty() { "unknown".to_string() } else { models_used.join(", ") };

            // Aggregate token counts from sub-agents
            let (total_in, total_out) = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .fold((0u32, 0u32), |(i, o), a| (i + a.inference_response.tokens_input, o + a.inference_response.tokens_output));

            // Store the diff regardless of PR outcome
            let captured_diff = result.merged_diff.clone();

            // Build run record with full tracking data
            let mut final_run = AgentRun::new(project, goal, "daemon", &model_str);
            final_run.status = status.clone();
            final_run.duration_ms = duration;
            final_run.quality_gate_passed = Some(matches!(status, RunStatus::Passed));
            final_run.diff = captured_diff;
            final_run.tokens_input = total_in;
            final_run.tokens_output = total_out;
            final_run.started_at = Some(start_time_rfc3339);
            final_run.last_activity_at = Some(chrono::Utc::now().to_rfc3339());

            // Store phase timing breakdown
            let pt = &result.phase_timings;
            final_run.phase_timings = Some(knowledge_base::PhaseTimingRecord {
                embedding_ms: embed_ms,
                rag_ms: pt.rag_ms,
                planning_ms: pt.planning_ms,
                agent_execution_ms: pt.agent_execution_ms,
                quality_gate_ms: pt.quality_gate_ms,
            });
            let total_profiled = embed_ms + pt.rag_ms + pt.planning_ms + pt.agent_execution_ms + pt.quality_gate_ms;
            info!(task_id, embed_ms, rag_ms = pt.rag_ms, planning_ms = pt.planning_ms,
                agent_ms = pt.agent_execution_ms, qg_ms = pt.quality_gate_ms,
                total_profiled, total_wall = duration,
                "Full phase timing (embed + runner)");

            // Build attempts from sub-agent results (one per sub-task)
            final_run.attempts = result.results.iter().enumerate().map(|(i, r)| {
                AttemptRecord {
                    attempt: (i + 1) as u32,
                    model: r.agent_result.as_ref().map(|a| a.inference_response.model.clone()).unwrap_or_default(),
                    prompt_preview: r.task.description.chars().take(ATTEMPT_PREVIEW_CHARS).collect(),
                    response_preview: r.agent_result.as_ref()
                        .map(|a| a.inference_response.content.chars().take(ATTEMPT_PREVIEW_CHARS).collect())
                        .unwrap_or_default(),
                    tokens_input: r.agent_result.as_ref().map(|a| a.inference_response.tokens_input).unwrap_or(0),
                    tokens_output: r.agent_result.as_ref().map(|a| a.inference_response.tokens_output).unwrap_or(0),
                    duration_ms: r.agent_result.as_ref().map(|a| a.inference_response.duration_ms).unwrap_or(0),
                    quality_passed: r.agent_result.as_ref().map(|a| a.applied),
                    error: r.error.clone(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                }
            }).collect();

            // Aggregate response text from all sub-agents
            let responses: Vec<String> = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .map(|a| a.inference_response.content.clone())
                .collect();
            if !responses.is_empty() {
                final_run.response_text = Some(responses.join("\n---\n"));
            }

            // Files ACTUALLY modified (real git-diff capture, Cargo.lock churn
            // stripped) — NOT r.task.files, which is the planner's CLAIM and is
            // recorded even when the edit never applied. Keeps files_modified
            // consistent with `diff` so reviews + co-edit stats reflect reality.
            final_run.files_modified = result.modified_files.iter()
                .map(|(p, _)| p.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            // Collect tool calls from all sub-agents
            final_run.tool_calls = result.results.iter()
                .filter_map(|r| r.agent_result.as_ref())
                .flat_map(|a| a.tool_calls.iter().cloned())
                .collect();

            // Collect errors
            let agent_errors: Vec<String> = result.results.iter()
                .filter_map(|r| r.error.as_ref())
                .cloned()
                .collect();

            if let Some(vr) = verify_reason {
                // Adversarial verify downgraded a gate-passing run — record WHY,
                // so verify-rejections are distinguishable from cargo-gate fails.
                final_run.error_message = Some(vr);
            } else if !any_work_done {
                final_run.error_message = Some(if agent_errors.is_empty() {
                    "All agents completed without making changes".into()
                } else {
                    format!("All agents failed:\n{}", agent_errors.join("\n"))
                });
            } else if !result.quality_passed && !agent_errors.is_empty() {
                final_run.error_message = Some(agent_errors.join("\n"));
            }

            // 5. Apply captured files to main repo for git CLI PR
            if any_work_done {
                for (path, content) in &result.modified_files {
                    let full_path = repo_path.join(path);
                    if let Some(parent) = full_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&full_path, content);
                }
            }

            // 6. Create PR if there are actual changes
            if any_work_done {
                let _sub_tasks_info: Vec<(String, String, String)> = result.results.iter()
                    .map(|r| {
                        let model = r.agent_result.as_ref().map(|a| a.inference_response.model.clone()).unwrap_or_default();
                        let st = if r.error.is_some() { "failed" } else if r.agent_result.as_ref().is_some_and(|a| a.applied) { "passed" } else { "skipped" };
                        (r.task.description.clone(), model, st.to_string())
                    })
                    .collect();

                // Create PR via GitHub API (no git CLI, no disk writes)
                let gh_token = std::env::var("GITHUB_TOKEN").ok();
                let gh_repo = std::env::var("GITHUB_REPO").unwrap_or_else(|_| "alpha-swarm/alpha-swarm2".into());

                if let Some(token) = gh_token {
                    if !result.modified_files.is_empty() {
                        let parts: Vec<&str> = gh_repo.splitn(2, '/').collect();
                        let (owner, repo_name) = (parts[0], parts.get(1).unwrap_or(&""));

                        let gh_config = virt_git::GitHubConfig {
                            owner: owner.into(),
                            repo: repo_name.to_string(),
                            token: token.clone(),
                            base_branch: "main".into(),
                        };

                        // Build workspace from captured files
                        let mut blob_store = virt_git::MemoryBlobStore::new();
                        let mut ws = virt_git::VirtWorkspace::new();

                        // Load original files from repo
                        for (path, _) in &result.modified_files {
                            if let Ok(original) = std::fs::read_to_string(repo_path.join(path)) {
                                ws.load_file(&mut blob_store, path, &original);
                            }
                        }
                        // Apply modified versions
                        for (path, content) in &result.modified_files {
                            if let Ok(text) = std::str::from_utf8(content) {
                                ws.write_file(&mut blob_store, path, text);
                            }
                        }

                        let branch = format!("agent/{}", task_id.replace(':', "-").chars().take(40).collect::<String>());
                        let diff_text = ws.diff_text(&blob_store);
                        let pr_body = format!("Generated by alpha-swarm agent.\n\n```diff\n{}\n```\n\n🤖 alpha-swarm", &diff_text[..diff_text.len().min(3000)]);

                        let http_client = reqwest::Client::new();
                        match virt_git::create_pr(
                            &gh_config, &ws, &blob_store,
                            &format!("agent: {}", &goal[..goal.len().min(60)]),
                            &format!("agent: {}", &goal[..goal.len().min(60)]),
                            &pr_body, &branch,
                            &|method, url, body, token| {
                                tokio::task::block_in_place(|| {
                                    tokio::runtime::Handle::current().block_on(async {
                                        let mut req = match method {
                                            "GET" => http_client.get(url),
                                            "POST" => http_client.post(url),
                                            _ => return Err(format!("Unknown method: {method}")),
                                        };
                                        req = req.header("Authorization", format!("Bearer {token}"))
                                            .header("Accept", "application/vnd.github+json")
                                            .header("User-Agent", "alpha-swarm");
                                        if !body.is_empty() {
                                            req = req.header("Content-Type", "application/json").body(body.to_string());
                                        }
                                        let resp = req.send().await.map_err(|e| format!("HTTP: {e}"))?;
                                        let status = resp.status();
                                        let text = resp.text().await.map_err(|e| format!("Read: {e}"))?;
                                        if !status.is_success() {
                                            return Err(format!("GitHub {status}: {}", &text[..text.len().min(200)]));
                                        }
                                        Ok(text)
                                    })
                                })
                            },
                        ) {
                            Ok(pr) => {
                                info!(pr_url = %pr.pr_url, "PR created via GitHub API (no git CLI)");
                                final_run.diff = Some(format!("PR: {}", pr.pr_url));
                            }
                            Err(e) => {
                                warn!("GitHub API PR failed: {e}, falling back to git CLI");
                                // Fallback to git CLI
                                match git.create_pr(&repo_path_str, goal, result.quality_passed, duration, total_in, total_out).await {
                                    Ok(pr_url) => {
                                        info!(pr_url = %pr_url, "PR created via git CLI fallback");
                                        final_run.diff = Some(format!("PR: {}", pr_url));
                                    }
                                    Err(e) => warn!("Git CLI PR also failed: {e}"),
                                }
                            }
                        }
                    } else {
                        warn!("No modified files captured, falling back to git CLI PR");
                        match git.create_pr(&repo_path_str, goal, result.quality_passed, duration, total_in, total_out).await {
                            Ok(pr_url) => { final_run.diff = Some(format!("PR: {}", pr_url)); }
                            Err(e) => { warn!("PR creation failed: {e}"); }
                        }
                    }
                } else {
                    // No GITHUB_TOKEN, use git CLI
                    match git.create_pr(&repo_path_str, goal, result.quality_passed, duration, total_in, total_out).await {
                        Ok(pr_url) => { final_run.diff = Some(format!("PR: {}", pr_url)); }
                        Err(e) => { warn!("PR creation failed: {e}"); }
                    }
                }
            }

            let _ = store.update_run(task_id, &final_run).await;

            // Run-level hook: fired after the run record is persisted.
            {
                let plan_summary: String = result.results.iter()
                    .map(|r| {
                        let st = if r.error.is_some() { "failed" }
                            else if r.agent_result.as_ref().is_some_and(|a| a.applied) { "passed" }
                            else { "noop" };
                        format!("[{}] {}: {}", st, r.task.id, r.task.description)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut pattern_ids = result.retrieved_pattern_ids.clone();
                pattern_ids.extend(plan_pattern_ids.iter().cloned());
                pattern_ids.dedup();
                hooks.emit_run_complete(&swarm_orchestrator::hooks::RunCompleteCtx {
                    run_id: task_id,
                    project,
                    goal,
                    quality_passed: gate_passed,
                    tasks_passed,
                    tasks_failed,
                    total_duration_ms: duration,
                    retrieved_pattern_ids: &pattern_ids,
                    plan_summary: &plan_summary,
                    diff: result.merged_diff.as_deref().unwrap_or(""),
                }).await;
            }

            // Lifecycle: on_agent_done — update embeddings for modified files only
            if !final_run.files_modified.is_empty() {
                emb_manager.on_agent_done(project, &repo_path, &final_run.files_modified).await;
            }

            // Emit completion event
            if let Some(pub_) = &publisher {
                let _ = pub_.publish(&SwarmEvent::SwarmCompleted {
                    project: project.into(),
                    goal: goal.into(),
                    quality_passed: gate_passed,
                    tasks_passed: tasks_passed as u32,
                    tasks_failed: tasks_failed as u32,
                    total_duration_ms: duration,
                    timestamp: SwarmEvent::timestamp(),
                }).await;
            }
        }
        None => {
            fail_task_with_duration(store.as_ref(), &publisher, task_id, project, goal,
                &format!("All {} iterations failed. Last error: {}", iteration, last_errors), duration).await;
        }
    }

    // Drop the run's isolated workspace (best-effort; /tmp is wiped on reboot
    // anyway). The verified changes already landed via land_to_branch.
    crate::repo::cleanup_run_workspace(task_id);
}

/// Execute (or resume) a workflow run through the engine.
/// Returns `Ok(Some(result))` on completion, `Ok(None)` when the run was
/// paused/cancelled (agent_run status already updated), `Err` on failure.
#[allow(clippy::too_many_arguments)]
async fn run_workflow(
    engine: &swarm_workflow::WorkflowEngine,
    store: &dyn KnowledgeBackend,
    runner: &swarm_orchestrator::SwarmRunner,
    router: &InferenceRouter,
    config: &SwarmConfig,
    task_id: &str,
    project: &str,
    goal: &str,
    repo_path: &std::path::Path,
    tasks: Vec<swarm_orchestrator::SubTask>,
) -> anyhow::Result<Option<swarm_orchestrator::SwarmResult>> {
    use swarm_workflow::{EngineContext, EngineOutcome, WorkflowRun};

    // Resume an existing non-terminal run, else create one from the plan.
    let mut wf = match engine.repo().get_by_run_id(task_id).await? {
        Some(existing) if !existing.state.is_terminal() => {
            info!(task_id, state = ?existing.state, "Resuming persisted workflow run");
            existing
        }
        _ => {
            let wf = WorkflowRun::from_tasks(
                project, goal, task_id, tasks,
                chrono::Utc::now().to_rfc3339(),
            ).with_trailing_quality_gate();
            engine.repo().create_run(&wf).await?;
            wf
        }
    };

    let ctx = EngineContext {
        runner,
        router,
        planner_tier: &config.tiers.orchestrator,
        repo_files: discover_source_files(repo_path),
        repo_path: repo_path.to_path_buf(),
    };

    match engine.execute(&mut wf, &ctx).await? {
        EngineOutcome::Completed(result) => Ok(Some(result)),
        EngineOutcome::Failed { result: _, error } => Err(anyhow::anyhow!(error)),
        EngineOutcome::Paused => {
            let _ = store.db_query_raw(&format_update(
                task_id,
                "SET status = 'paused', progress_message = 'Workflow paused — awaiting resume'",
            )).await;
            Ok(None)
        }
        EngineOutcome::Cancelled => {
            let _ = store.db_query_raw(&format_update(
                task_id,
                "SET status = 'failed', error_message = 'Workflow cancelled by user'",
            )).await;
            Ok(None)
        }
    }
}

async fn fail_task(
    store: &dyn KnowledgeBackend,
    publisher: &Option<Arc<EventPublisher>>,
    task_id: &str, project: &str, goal: &str, error: &str,
) {
    fail_task_with_duration(store, publisher, task_id, project, goal, error, 0).await;
}

async fn fail_task_with_duration(
    store: &dyn KnowledgeBackend,
    publisher: &Option<Arc<EventPublisher>>,
    task_id: &str, project: &str, goal: &str, error_msg: &str, duration_ms: u64,
) {
    error!(task_id, project, error = error_msg, "Task failed");

    let mut run = AgentRun::new(project, goal, "daemon", "error");
    run.status = RunStatus::Failed;
    run.error_message = Some(error_msg.to_string());
    run.duration_ms = duration_ms;
    let _ = store.update_run(task_id, &run).await;

    if let Some(pub_) = &publisher {
        let _ = pub_.publish(&SwarmEvent::AgentFailed {
            project: project.into(),
            agent_id: format!("daemon-{}", &task_id[..task_id.len().min(8)]),
            error: error_msg.into(),
            model: String::new(),
            duration_ms,
            timestamp: SwarmEvent::timestamp(),
        }).await;
    }
}

#[cfg(test)]
mod verify_tests {
    use super::{diff_adds_test, diff_is_doc_only};

    #[test]
    fn detects_added_test() {
        assert!(diff_adds_test("+    #[test]\n+    fn x() { assert_eq!(1, 1); }"));
        assert!(diff_adds_test("+        assert!(got.is_some());"));
        assert!(!diff_adds_test("+    pub fn foo() -> i32 { 1 }"));
        // a REMOVED test (-) doesn't count as adding coverage
        assert!(!diff_adds_test("-    #[test]\n-    fn gone() {}"));
    }

    #[test]
    fn doc_only_detection() {
        assert!(diff_is_doc_only("+/// a doc comment\n+//! module doc"));
        assert!(diff_is_doc_only("-// old comment\n+// new comment"));
        // real code change → not doc-only
        assert!(!diff_is_doc_only("+/// doc\n+pub const X: u32 = 1;"));
        // no changed lines at all → not doc-only (nothing to be lenient about)
        assert!(!diff_is_doc_only(" context line only\n@@ hunk @@"));
    }
}
