//! Agent workspace manager using local git clones.
//!
//! Agents work in isolated clones. The main repo working directory is NEVER
//! modified during agent execution — diffs are captured via virt-git after
//! agents finish.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::info;

/// Manages agent workspaces backed by local git clones.
pub struct MemTreeManager {
    repo_path: PathBuf,
    base_dir: PathBuf,
    workspaces: Vec<WorkspaceInfo>,
}

struct WorkspaceInfo {
    agent_id: String,
    work_dir: PathBuf,
}

/// Base directory for agent temp workspaces.
const WORKSPACE_BASE: &str = "/tmp/alpha-swarm/workspaces";

impl MemTreeManager {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        let repo_path = repo_path.into();
        // Per-run workspaces: sit them beside the run's isolated clone so two
        // parallel runs never collide on a shared task-id path (e.g. both runs'
        // "task-1"). When repo_path is the per-run clone
        // (/tmp/alpha-swarm/runs/{run}/repo) this yields
        // /tmp/alpha-swarm/runs/{run}/workspaces, cleaned up with the run.
        // Falls back to the global base if repo_path has no parent.
        let base_dir = repo_path.parent()
            .map(|p| p.join("workspaces"))
            .unwrap_or_else(|| PathBuf::from(WORKSPACE_BASE));
        Self {
            repo_path,
            base_dir,
            workspaces: Vec::new(),
        }
    }

    /// Create an isolated workspace by cloning the repo.
    /// Agent gets a full buildable project. Returns the clone path.
    pub fn create(&mut self, agent_id: &str, _files: &[String]) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.base_dir)
            .context("Failed to create workspace base dir")?;

        let work_dir = self.base_dir.join(agent_id);

        // Force-clean stale workspace
        if work_dir.exists() {
            let _ = std::process::Command::new("rm")
                .args(["-rf"])
                .arg(&work_dir)
                .output();
        }

        // Local git clone — depth 1 for speed, shared objects
        let output = std::process::Command::new("git")
            .args(["clone", "--depth", "1", "--single-branch"])
            .arg(&self.repo_path)
            .arg(&work_dir)
            .output()
            .context("Failed to run git clone")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git clone failed: {stderr}");
        }

        // Symlink target/ to shared cache for faster incremental builds
        let shared_target = std::path::PathBuf::from("/tmp/alpha-swarm/shared-target");
        let _ = std::fs::create_dir_all(&shared_target);
        let ws_target = work_dir.join("target");
        if !ws_target.exists() {
            #[cfg(unix)]
            { let _ = std::os::unix::fs::symlink(&shared_target, &ws_target); }
        }

        if let Ok(out) = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&work_dir)
            .output()
        {
            let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
            info!(agent = agent_id, path = %work_dir.display(), head = %head, "Created workspace (shared build cache)");
        }

        self.workspaces.push(WorkspaceInfo {
            agent_id: agent_id.to_string(),
            work_dir: work_dir.clone(),
        });

        Ok(work_dir)
    }

    /// Get the workspace path for a given agent.
    pub fn workspace_path(&self, agent_id: &str) -> Option<&Path> {
        self.workspaces.iter()
            .find(|w| w.agent_id == agent_id)
            .map(|w| w.work_dir.as_path())
    }

    /// Clean up all workspaces.
    pub fn cleanup(&mut self) {
        for ws in self.workspaces.drain(..) {
            let _ = std::fs::remove_dir_all(&ws.work_dir);
            info!(agent = %ws.agent_id, "Cleaned up workspace");
        }
    }
}

impl Drop for MemTreeManager {
    fn drop(&mut self) {
        self.cleanup();
    }
}
