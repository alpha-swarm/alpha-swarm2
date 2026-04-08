//! In-memory git tree workspace using git2.
//!
//! Agents work in isolated clones. git2 handles commit creation and diff
//! extraction. The main repo working directory is NEVER modified during
//! agent execution — changes are only applied after quality gate passes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::info;

/// Manages agent workspaces backed by local git clones + git2.
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
        Self {
            repo_path: repo_path.into(),
            base_dir: PathBuf::from(WORKSPACE_BASE),
            workspaces: Vec::new(),
        }
    }

    /// Create an isolated workspace by cloning the repo.
    /// Agent gets a full buildable project. Returns the clone path.
    pub fn create(&mut self, agent_id: &str, _files: &[String]) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.base_dir)
            .context("Failed to create workspace base dir")?;

        let work_dir = self.base_dir.join(agent_id);

        // Clean up stale workspace
        if work_dir.exists() {
            let _ = std::fs::remove_dir_all(&work_dir);
        }

        // Local git clone (uses hardlinks, fast)
        let output = std::process::Command::new("git")
            .args(["clone", "--local", "--no-hardlinks"])
            .arg(&self.repo_path)
            .arg(&work_dir)
            .output()
            .context("Failed to run git clone")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git clone failed: {stderr}");
        }

        info!(agent = agent_id, path = %work_dir.display(), "Created workspace (local clone)");

        self.workspaces.push(WorkspaceInfo {
            agent_id: agent_id.to_string(),
            work_dir: work_dir.clone(),
        });

        Ok(work_dir)
    }

    /// After agent finishes, extract diff from workspace and create a git2 commit
    /// in the MAIN repo (without updating working directory).
    pub fn commit_changes(&self, agent_id: &str, message: &str) -> Result<CommitResult> {
        let ws = self.workspaces.iter()
            .find(|w| w.agent_id == agent_id)
            .context("Workspace not found")?;

        // Open the main repo for commit creation
        let repo = git2::Repository::open(&self.repo_path)
            .context("Failed to open main git repo")?;

        let head = repo.head().context("Failed to get HEAD")?;
        let head_commit = head.peel_to_commit().context("Failed to peel HEAD to commit")?;
        let base_tree = head_commit.tree().context("Failed to get HEAD tree")?;

        // Collect changes: compare workspace files against main repo HEAD
        let changes = collect_changes(&repo, &base_tree, &ws.work_dir)?;

        if changes.is_empty() {
            return Ok(CommitResult {
                has_changes: false,
                diff: String::new(),
                commit_id: None,
                workspace_path: ws.work_dir.clone(),
            });
        }

        // Build new tree with changes applied
        let new_tree_id = build_tree_with_changes(&repo, &base_tree, &changes)?;
        let new_tree = repo.find_tree(new_tree_id)?;

        // Generate diff text for visibility
        let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&new_tree), None)?;
        let mut diff_text = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let prefix = match line.origin() {
                '+' | '-' | ' ' => &line.origin().to_string(),
                _ => "",
            };
            diff_text.push_str(prefix);
            if let Ok(content) = std::str::from_utf8(line.content()) {
                diff_text.push_str(content);
            }
            true
        })?;

        // Create commit in main repo (no ref update — just an object)
        let sig = git2::Signature::now("alpha-swarm", "agent@alpha-swarm.local")?;
        let commit_id = repo.commit(
            None, // Don't update any ref
            &sig, &sig, message, &new_tree, &[&head_commit],
        )?;

        info!(agent = agent_id, changes = changes.len(), commit = %commit_id, "Created commit from workspace");

        Ok(CommitResult {
            has_changes: true,
            diff: diff_text,
            commit_id: Some(commit_id),
            workspace_path: ws.work_dir.clone(),
        })
    }

    /// Apply a committed change to the main repo working directory.
    /// Called ONLY after quality gate passes on the workspace.
    pub fn apply_to_main(&self, commit_result: &CommitResult) -> Result<()> {
        let Some(commit_id) = commit_result.commit_id else {
            return Ok(());
        };

        let repo = git2::Repository::open(&self.repo_path)?;
        let commit = repo.find_commit(commit_id)?;

        // Fast-forward HEAD to the agent's commit
        let mut head_ref = repo.head()?;
        head_ref.set_target(commit_id, &format!("alpha-swarm: {}", commit.message().unwrap_or("agent commit")))?;

        // Checkout the new tree to update working directory
        let tree = commit.tree()?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force(); // We've verified quality, safe to overwrite
        repo.checkout_tree(tree.as_object(), Some(&mut checkout))?;

        info!(commit = %commit_id, "Applied agent commit to main repo");
        Ok(())
    }

    /// Get the workspace path for running quality gate.
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

pub struct CommitResult {
    pub has_changes: bool,
    pub diff: String,
    pub commit_id: Option<git2::Oid>,
    pub workspace_path: PathBuf,
}

/// File change types.
enum Change {
    Modified(Vec<u8>),
    Created(Vec<u8>),
}

/// Compare workspace files against git HEAD tree.
fn collect_changes(
    repo: &git2::Repository,
    base_tree: &git2::Tree,
    work_dir: &Path,
) -> Result<HashMap<String, Change>> {
    let mut changes = HashMap::new();

    walk_dir(work_dir, work_dir, &mut |rel_path| {
        let ws_content = match std::fs::read(work_dir.join(rel_path)) {
            Ok(c) => c,
            Err(_) => return,
        };

        match base_tree.get_path(Path::new(rel_path)) {
            Ok(entry) => {
                if let Ok(blob) = repo.find_blob(entry.id()) {
                    if blob.content() != ws_content.as_slice() {
                        changes.insert(rel_path.to_string(), Change::Modified(ws_content));
                    }
                }
            }
            Err(_) => {
                // New file created by agent
                changes.insert(rel_path.to_string(), Change::Created(ws_content));
            }
        }
    });

    Ok(changes)
}

/// Build a new tree with changes applied using TreeBuilder.
fn build_tree_with_changes(
    repo: &git2::Repository,
    base_tree: &git2::Tree,
    changes: &HashMap<String, Change>,
) -> Result<git2::Oid> {
    // Group changes by directory for recursive tree building
    build_subtree(repo, Some(base_tree), changes, "")
}

/// Recursively build tree, handling nested directories.
fn build_subtree(
    repo: &git2::Repository,
    base: Option<&git2::Tree>,
    changes: &HashMap<String, Change>,
    prefix: &str,
) -> Result<git2::Oid> {
    let mut builder = repo.treebuilder(base)?;

    // Find direct children at this level
    let mut handled_dirs = std::collections::HashSet::new();

    for (path, change) in changes {
        // Get path relative to current prefix
        let rel = if prefix.is_empty() {
            path.as_str()
        } else if let Some(stripped) = path.strip_prefix(&format!("{prefix}/")) {
            stripped
        } else {
            continue;
        };

        if let Some(slash) = rel.find('/') {
            // Nested in subdirectory
            let dir_name = &rel[..slash];
            if handled_dirs.insert(dir_name.to_string()) {
                // Get existing subtree
                let sub_tree = base.and_then(|t| {
                    t.get_name(dir_name)
                        .and_then(|e| repo.find_tree(e.id()).ok())
                });

                let sub_prefix = if prefix.is_empty() {
                    dir_name.to_string()
                } else {
                    format!("{prefix}/{dir_name}")
                };

                let sub_oid = build_subtree(repo, sub_tree.as_ref(), changes, &sub_prefix)?;
                builder.insert(dir_name, sub_oid, 0o040000)?;
            }
        } else {
            // Direct file at this level
            match change {
                Change::Modified(content) | Change::Created(content) => {
                    let blob_id = repo.blob(content)?;
                    builder.insert(rel, blob_id, 0o100644)?;
                }
            }
        }
    }

    Ok(builder.write()?)
}

/// Walk directory recursively, calling callback with relative paths.
fn walk_dir(dir: &Path, base: &Path, cb: &mut impl FnMut(&str)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            walk_dir(&path, base, cb);
        } else if let Ok(rel) = path.strip_prefix(base) {
            cb(&rel.to_string_lossy());
        }
    }
}
