use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

// Ephemeral scratch: clones re-fetch on demand, so /tmp (wiped on reboot) is
// the correct home. Only the database needs durable storage (see SurrealConfig).
const REPOS_BASE: &str = "/tmp/alpha-swarm/repos";
// Per-run isolated working copies (one git worktree-equivalent per run) so
// concurrent runs never share mutable git state (sync/reset/edit/gate).
const RUNS_BASE: &str = "/tmp/alpha-swarm/runs";

/// Filesystem-safe slug for a run id (e.g. `agent_run:abc` → `agent_run_abc`).
fn run_slug(run_id: &str) -> String {
    run_id.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Per-run isolated working copy under [`RUNS_BASE`]. Reused if already present
/// (planning then execution of the same run share it).
pub fn run_workspace_path(run_id: &str) -> PathBuf {
    PathBuf::from(RUNS_BASE).join(run_slug(run_id)).join("repo")
}

/// Derive an isolated per-run working copy from a shared base clone via a fast
/// `git clone --local` (hardlinked objects — cheap, immutable, safe to share).
/// The run can then sync/checkout/reset/edit in full isolation from other runs.
/// `origin` is repointed at the true source so the run still fetches fresh.
/// Reuses an existing per-run copy (returns it untouched).
pub fn isolate_run_workspace(base: &std::path::Path, run_id: &str, repo_url: &str) -> Result<PathBuf> {
    let work = run_workspace_path(run_id);
    if work.join(".git").exists() {
        return Ok(work);
    }
    if let Some(parent) = work.parent() {
        std::fs::create_dir_all(parent).context("create run workspace dir")?;
    }
    let out = Command::new("git")
        .args(["clone", "--local"])
        .arg(base)
        .arg(&work)
        .output()
        .context("git clone --local (run workspace) failed")?;
    if !out.status.success() {
        bail!("git clone --local failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    // Point origin back at the true source so sync_repo_to_branch fetches fresh
    // (the --local clone's origin would otherwise be the base clone).
    let _ = Command::new("git")
        .args(["remote", "set-url", "origin", repo_url])
        .current_dir(&work)
        .output();
    info!(run_id, path = %work.display(), "Isolated per-run workspace");
    Ok(work)
}

/// Per-run quality-gate worktree dir (sibling of the run's repo, so concurrent
/// gates never collide). Removed by [`cleanup_run_workspace`].
pub fn run_gate_path(run_id: &str) -> PathBuf {
    PathBuf::from(RUNS_BASE).join(run_slug(run_id)).join("gate")
}

/// Remove a run's isolated workspace (call on run completion).
pub fn cleanup_run_workspace(run_id: &str) {
    let dir = PathBuf::from(RUNS_BASE).join(run_slug(run_id));
    let _ = std::fs::remove_dir_all(dir);
}

/// Ensure a project's repo is cloned and up-to-date.
/// Returns the local path to the repo.
pub fn ensure_repo(project: &str, repo_url: &str) -> Result<PathBuf> {
    let base = PathBuf::from(REPOS_BASE);
    std::fs::create_dir_all(&base).context("Failed to create repos directory")?;

    let repo_path = base.join(project);

    if repo_path.join(".git").exists() {
        // Already cloned — pull latest
        info!(project, path = %repo_path.display(), "Updating existing repo");
        let output = Command::new("git")
            .args(["pull", "--ff-only"])
            .current_dir(&repo_path)
            .output()
            .context("git pull failed")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(project, "git pull failed (non-fatal): {stderr}");
            // Continue with existing state — pull failure shouldn't block execution
        }
    } else {
        // Clone fresh
        info!(project, url = repo_url, path = %repo_path.display(), "Cloning repo");
        let output = Command::new("git")
            .args(["clone", repo_url])
            .arg(&repo_path)
            .output()
            .context("git clone failed")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git clone failed: {stderr}");
        }

        info!(project, "Repo cloned successfully");
    }

    Ok(repo_path)
}
