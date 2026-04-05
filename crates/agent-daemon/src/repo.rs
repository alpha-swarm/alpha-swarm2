use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

const REPOS_BASE: &str = "/tmp/alpha-swarm/repos";

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
