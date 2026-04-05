use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

/// Manages git worktrees for parallel agent isolation.
pub struct WorktreeManager {
    repo_path: PathBuf,
    base_dir: PathBuf,
    worktrees: Vec<WorktreeInfo>,
}

struct WorktreeInfo {
    agent_id: String,
    path: PathBuf,
    branch: String,
}

impl WorktreeManager {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        let repo_path = repo_path.into();
        let base_dir = PathBuf::from("/tmp/alpha-swarm/worktrees");
        Self {
            repo_path,
            base_dir,
            worktrees: Vec::new(),
        }
    }

    /// Create an isolated worktree for an agent.
    /// Returns the worktree path.
    pub fn create(&mut self, agent_id: &str) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.base_dir)
            .context("Failed to create worktree base dir")?;

        let wt_path = self.base_dir.join(agent_id);
        let branch = format!("agent/{agent_id}");

        // Remove stale worktree if exists
        if wt_path.exists() {
            let _ = Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(&wt_path)
                .current_dir(&self.repo_path)
                .output();
            let _ = std::fs::remove_dir_all(&wt_path);
        }

        // Delete stale branch if exists
        let _ = Command::new("git")
            .args(["branch", "-D", &branch])
            .current_dir(&self.repo_path)
            .output();

        let output = Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&wt_path)
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to run git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git worktree add failed: {stderr}");
        }

        info!(agent = agent_id, path = %wt_path.display(), "Created worktree");

        self.worktrees.push(WorktreeInfo {
            agent_id: agent_id.to_string(),
            path: wt_path.clone(),
            branch,
        });

        Ok(wt_path)
    }

    /// Extract the diff between a worktree and its base branch.
    pub fn extract_diff(&self, agent_id: &str) -> Result<String> {
        let wt = self.worktrees.iter()
            .find(|w| w.agent_id == agent_id)
            .context("Worktree not found")?;

        // Stage all changes first
        let _ = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&wt.path)
            .output();

        let output = Command::new("git")
            .args(["diff", "--cached"])
            .current_dir(&wt.path)
            .output()
            .context("Failed to run git diff")?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Get the worktree path for an agent.
    pub fn get_path(&self, agent_id: &str) -> Option<&Path> {
        self.worktrees.iter()
            .find(|w| w.agent_id == agent_id)
            .map(|w| w.path.as_path())
    }

    /// Apply a diff from a worktree onto the main repo.
    pub fn apply_diff_to_main(&self, agent_id: &str) -> Result<()> {
        let diff = self.extract_diff(agent_id)?;
        if diff.is_empty() {
            info!(agent = agent_id, "No changes to apply");
            return Ok(());
        }

        // Apply the diff to the main repo
        let mut child = Command::new("git")
            .args(["apply", "--3way", "-"])
            .stdin(std::process::Stdio::piped())
            .current_dir(&self.repo_path)
            .spawn()
            .context("Failed to start git apply")?;

        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(diff.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(agent = agent_id, "git apply failed: {stderr}");
            bail!("Failed to apply diff from agent {agent_id}: {stderr}");
        }

        info!(agent = agent_id, "Applied diff to main repo");
        Ok(())
    }

    /// Clean up all worktrees.
    pub fn cleanup(&mut self) {
        for wt in self.worktrees.drain(..) {
            let _ = Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(&wt.path)
                .current_dir(&self.repo_path)
                .output();

            let _ = Command::new("git")
                .args(["branch", "-D", &wt.branch])
                .current_dir(&self.repo_path)
                .output();

            info!(agent = %wt.agent_id, "Cleaned up worktree");
        }
    }
}

impl Drop for WorktreeManager {
    fn drop(&mut self) {
        self.cleanup();
    }
}
