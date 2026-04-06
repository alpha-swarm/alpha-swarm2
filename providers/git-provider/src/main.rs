//! Git provider — NATS service that handles git operations.
//!
//! Subscribes to `swarm.git.*` subjects and executes git/gh CLI commands.
//! Runs natively on machines with repo access.
//!
//! Future: wrap as a wasmCloud native capability provider.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

/// Base directory for cloned repos.
const REPO_BASE: &str = "/tmp/alpha-swarm/repos";
/// Base directory for worktrees.
const WORKTREE_BASE: &str = "/tmp/alpha-swarm/worktrees";
/// Timeout for git commands (seconds).
const GIT_TIMEOUT_SECS: u64 = 60;

#[derive(Deserialize)]
struct GitRequest {
    op: String,
    #[serde(default)]
    project: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    repo_path: String,
    #[serde(default)]
    agent_id: String,
    #[serde(default)]
    goal: String,
    #[serde(default)]
    quality_passed: bool,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    tokens_in: u32,
    #[serde(default)]
    tokens_out: u32,
}

#[derive(Serialize)]
struct GitResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<String>>,
}

impl GitResponse {
    fn ok(result: impl Into<String>) -> Self {
        Self { result: Some(result.into()), error: None, files: None }
    }
    fn err(error: impl Into<String>) -> Self {
        Self { result: None, error: Some(error.into()), files: None }
    }
    fn ok_files(files: Vec<String>) -> Self {
        Self { result: None, error: None, files: Some(files) }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4223".into());
    info!(nats_url = %nats_url, "Git provider starting");

    // Ensure base directories exist
    std::fs::create_dir_all(REPO_BASE).ok();
    std::fs::create_dir_all(WORKTREE_BASE).ok();

    let client = async_nats::connect(&nats_url).await
        .context("Failed to connect to NATS")?;

    let mut sub = client.subscribe("swarm.git.*").await
        .context("Failed to subscribe to swarm.git.*")?;

    info!("Git provider listening on swarm.git.*");

    while let Some(msg) = sub.next().await {
        let reply = msg.reply.clone();
        let payload = msg.payload.to_vec();

        let response = match serde_json::from_slice::<GitRequest>(&payload) {
            Ok(req) => handle_request(&req),
            Err(e) => GitResponse::err(format!("invalid request: {e}")),
        };

        if let Some(reply_to) = reply {
            let resp_bytes = serde_json::to_vec(&response).unwrap_or_default();
            let _ = client.publish(reply_to, resp_bytes.into()).await;
        }
    }

    Ok(())
}

fn handle_request(req: &GitRequest) -> GitResponse {
    match req.op.as_str() {
        "ensure_repo" => op_ensure_repo(&req.project, &req.url),
        "create_worktree" => op_create_worktree(&req.repo_path, &req.agent_id),
        "remove_worktree" => op_remove_worktree(&req.repo_path, &req.agent_id),
        "apply_diff" => op_apply_diff(&req.repo_path, &req.agent_id),
        "extract_diff" => op_extract_diff(&req.repo_path, &req.agent_id),
        "status" => op_status(&req.repo_path),
        "diff" => op_diff(&req.repo_path),
        "reset" => op_reset(&req.repo_path),
        "list_source_files" => op_list_source_files(&req.repo_path),
        "create_pr" => op_create_pr(&req.repo_path, &req.goal, req.quality_passed, req.duration_ms, req.tokens_in, req.tokens_out),
        _ => GitResponse::err(format!("unknown op: {}", req.op)),
    }
}

fn op_ensure_repo(project: &str, url: &str) -> GitResponse {
    let path = PathBuf::from(REPO_BASE).join(project);
    if path.exists() {
        // Pull latest
        match run_git(&path, &["pull", "--rebase"]) {
            Ok(_) => { info!(project, "Repo updated"); }
            Err(e) => { warn!(project, "Pull failed (non-fatal): {e}"); }
        }
    } else {
        match run_git(Path::new(REPO_BASE), &["clone", url, project]) {
            Ok(_) => { info!(project, url, "Repo cloned"); }
            Err(e) => return GitResponse::err(format!("clone failed: {e}")),
        }
    }
    GitResponse::ok(path.to_string_lossy())
}

fn op_create_worktree(repo_path: &str, agent_id: &str) -> GitResponse {
    let wt_path = PathBuf::from(WORKTREE_BASE).join(agent_id);
    let branch = format!("agent/{agent_id}");

    if wt_path.exists() {
        let _ = std::fs::remove_dir_all(&wt_path);
    }

    match run_git(Path::new(repo_path), &["worktree", "add", "-b", &branch, &wt_path.to_string_lossy()]) {
        Ok(_) => GitResponse::ok(wt_path.to_string_lossy()),
        Err(e) => GitResponse::err(format!("worktree create: {e}")),
    }
}

fn op_remove_worktree(repo_path: &str, agent_id: &str) -> GitResponse {
    let wt_path = PathBuf::from(WORKTREE_BASE).join(agent_id);
    let _ = run_git(Path::new(repo_path), &["worktree", "remove", "--force", &wt_path.to_string_lossy()]);
    let branch = format!("agent/{agent_id}");
    let _ = run_git(Path::new(repo_path), &["branch", "-D", &branch]);
    GitResponse::ok("removed")
}

fn op_apply_diff(repo_path: &str, agent_id: &str) -> GitResponse {
    let wt_path = PathBuf::from(WORKTREE_BASE).join(agent_id);

    // Get diff from worktree
    let output = Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(&wt_path)
        .output();

    let diff = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(e) => return GitResponse::err(format!("get diff: {e}")),
    };

    if diff.is_empty() {
        return GitResponse::ok("no changes");
    }

    // Apply to main repo
    let mut child = match Command::new("git")
        .args(["apply", "--3way", "-"])
        .stdin(std::process::Stdio::piped())
        .current_dir(repo_path)
        .spawn() {
        Ok(c) => c,
        Err(e) => return GitResponse::err(format!("spawn apply: {e}")),
    };

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        let _ = stdin.write_all(diff.as_bytes());
    }

    match child.wait_with_output() {
        Ok(o) if o.status.success() => GitResponse::ok(diff),
        Ok(o) => {
            // Rollback
            let _ = run_git(Path::new(repo_path), &["checkout", "."]);
            GitResponse::err(format!("apply failed (rolled back): {}", String::from_utf8_lossy(&o.stderr)))
        }
        Err(e) => GitResponse::err(format!("apply wait: {e}")),
    }
}

fn op_extract_diff(repo_path: &str, agent_id: &str) -> GitResponse {
    let wt_path = PathBuf::from(WORKTREE_BASE).join(agent_id);
    match Command::new("git").args(["diff", "HEAD"]).current_dir(&wt_path).output() {
        Ok(o) => GitResponse::ok(String::from_utf8_lossy(&o.stdout)),
        Err(e) => GitResponse::err(format!("extract diff: {e}")),
    }
}

fn op_status(repo_path: &str) -> GitResponse {
    match Command::new("git").args(["status", "--short"]).current_dir(repo_path).output() {
        Ok(o) => GitResponse::ok(String::from_utf8_lossy(&o.stdout)),
        Err(e) => GitResponse::err(format!("status: {e}")),
    }
}

fn op_diff(repo_path: &str) -> GitResponse {
    match Command::new("git").args(["diff"]).current_dir(repo_path).output() {
        Ok(o) => GitResponse::ok(String::from_utf8_lossy(&o.stdout)),
        Err(e) => GitResponse::err(format!("diff: {e}")),
    }
}

fn op_reset(repo_path: &str) -> GitResponse {
    match run_git(Path::new(repo_path), &["checkout", "."]) {
        Ok(_) => GitResponse::ok("reset"),
        Err(e) => GitResponse::err(format!("reset: {e}")),
    }
}

fn op_list_source_files(repo_path: &str) -> GitResponse {
    let extensions = ["rs", "ts", "js", "go", "py"];
    let mut files = Vec::new();

    fn walk(dir: &Path, base: &Path, ext: &[&str], out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with('.') || name == "target" || name == "node_modules" || name == "dist" { continue; }
                walk(&path, base, ext, out);
            } else if let Some(e) = path.extension().and_then(|e| e.to_str()) {
                if ext.contains(&e) {
                    if let Ok(rel) = path.strip_prefix(base) {
                        out.push(rel.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    walk(Path::new(repo_path), Path::new(repo_path), &extensions, &mut files);
    files.sort();
    GitResponse::ok_files(files)
}

fn op_create_pr(repo_path: &str, goal: &str, quality_passed: bool, duration_ms: u64, tokens_in: u32, tokens_out: u32) -> GitResponse {
    let slug: String = goal.to_lowercase().chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>().trim_matches('-').chars().take(50).collect();
    let branch = format!("agent/{slug}");

    if run_git(Path::new(repo_path), &["checkout", "-b", &branch]).is_err() {
        return GitResponse::err("branch creation failed");
    }

    if run_git(Path::new(repo_path), &["add", "-A"]).is_err() {
        return GitResponse::err("git add failed");
    }

    // Check for changes
    if let Ok(o) = Command::new("git").args(["diff", "--cached", "--quiet"]).current_dir(repo_path).status() {
        if o.success() {
            let _ = run_git(Path::new(repo_path), &["checkout", "-"]);
            let _ = run_git(Path::new(repo_path), &["branch", "-D", &branch]);
            return GitResponse::err("no changes to commit");
        }
    }

    let commit_msg = format!("agent: {goal}\n\nQuality: {}\nDuration: {duration_ms}ms\nTokens: {tokens_in} in / {tokens_out} out",
        if quality_passed { "passed" } else { "failed" });

    if run_git(Path::new(repo_path), &["commit", "-m", &commit_msg]).is_err() {
        return GitResponse::err("commit failed");
    }

    if run_git(Path::new(repo_path), &["push", "origin", &branch]).is_err() {
        return GitResponse::err("push failed");
    }

    let pr_title = format!("agent: {}", &goal.chars().take(70).collect::<String>());
    let pr_body = format!("## Goal\n{goal}\n\n## Quality Gate\n{}\n\n## Stats\n- Duration: {duration_ms}ms\n- Tokens: {tokens_in} in / {tokens_out} out",
        if quality_passed { "Passed" } else { "Failed" });

    match Command::new("gh").args(["pr", "create", "--title", &pr_title, "--body", &pr_body, "--head", &branch]).current_dir(repo_path).output() {
        Ok(o) if o.status.success() => {
            let url = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let _ = run_git(Path::new(repo_path), &["checkout", "-"]);
            GitResponse { result: Some(url.clone()), error: None, files: None }
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            warn!("gh pr create failed: {stderr}");
            let _ = run_git(Path::new(repo_path), &["checkout", "-"]);
            GitResponse::err(format!("PR creation failed: {stderr}"))
        }
        Err(e) => GitResponse::err(format!("gh not available: {e}")),
    }
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("git {} failed to start", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
