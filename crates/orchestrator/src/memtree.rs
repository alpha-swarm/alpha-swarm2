//! In-memory git tree workspace using git2.
//!
//! Replaces git worktrees with a hybrid approach:
//! - Agent works on temp directory (tools need real files)
//! - git2 handles commit creation and merge (no `git apply`)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::info;

/// Manages agent workspaces backed by git2 for reliable commit/merge.
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

    /// Create an isolated workspace for an agent.
    /// Copies only the files the agent needs into a temp directory.
    pub fn create(&mut self, agent_id: &str, files: &[String]) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.base_dir)
            .context("Failed to create workspace base dir")?;

        let work_dir = self.base_dir.join(agent_id);

        // Clean up stale workspace
        if work_dir.exists() {
            let _ = std::fs::remove_dir_all(&work_dir);
        }
        std::fs::create_dir_all(&work_dir)?;

        // Copy needed files from repo to workspace
        for file in files {
            let src = self.repo_path.join(file);
            let dst = work_dir.join(file);

            if src.exists() {
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&src, &dst)?;
            }
        }

        info!(agent = agent_id, path = %work_dir.display(), files = files.len(), "Created workspace");

        self.workspaces.push(WorkspaceInfo {
            agent_id: agent_id.to_string(),
            work_dir: work_dir.clone(),
        });

        Ok(work_dir)
    }

    /// After agent finishes, collect changes and create a git commit using git2.
    /// Returns the diff as a string for visibility.
    pub fn commit_changes(&self, agent_id: &str, message: &str) -> Result<CommitResult> {
        let ws = self.workspaces.iter()
            .find(|w| w.agent_id == agent_id)
            .context("Workspace not found")?;

        let repo = git2::Repository::open(&self.repo_path)
            .context("Failed to open git repo")?;

        let head = repo.head().context("Failed to get HEAD")?;
        let head_commit = head.peel_to_commit().context("Failed to peel HEAD to commit")?;
        let base_tree = head_commit.tree().context("Failed to get HEAD tree")?;

        // Collect changes: compare workspace files against repo HEAD
        let changes = collect_changes(&repo, &base_tree, &ws.work_dir, &self.repo_path)?;

        if changes.is_empty() {
            return Ok(CommitResult {
                has_changes: false,
                diff: String::new(),
                commit_id: None,
            });
        }

        // Build new tree with changes applied
        let new_tree_id = build_tree_with_changes(&repo, &base_tree, &changes)?;
        let new_tree = repo.find_tree(new_tree_id)?;

        // Generate diff for visibility
        let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&new_tree), None)?;
        let mut diff_text = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let prefix = match line.origin() {
                '+' => "+",
                '-' => "-",
                ' ' => " ",
                'H' => "",  // file header
                'F' => "",  // file header
                _ => "",
            };
            diff_text.push_str(prefix);
            if let Ok(content) = std::str::from_utf8(line.content()) {
                diff_text.push_str(content);
            }
            true
        })?;

        // Create commit
        let sig = git2::Signature::now("alpha-swarm", "agent@alpha-swarm.local")?;
        let commit_id = repo.commit(
            None,  // Don't update any ref — we'll merge/apply manually
            &sig,
            &sig,
            message,
            &new_tree,
            &[&head_commit],
        )?;

        info!(agent = agent_id, changes = changes.len(), commit = %commit_id, "Created commit from workspace");

        Ok(CommitResult {
            has_changes: true,
            diff: diff_text,
            commit_id: Some(commit_id),
        })
    }

    /// Apply committed changes to the working directory.
    pub fn apply_to_working_dir(&self, commit_result: &CommitResult) -> Result<()> {
        let Some(commit_id) = commit_result.commit_id else {
            return Ok(());
        };

        let repo = git2::Repository::open(&self.repo_path)?;
        let commit = repo.find_commit(commit_id)?;
        let tree = commit.tree()?;

        // Checkout the new tree to the working directory
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();  // Don't overwrite untracked files
        checkout.update_index(true);

        repo.checkout_tree(tree.as_object(), Some(&mut checkout))
            .context("Failed to checkout new tree")?;

        // Fast-forward HEAD to the new commit
        repo.head()?.set_target(commit_id, "alpha-swarm agent commit")?;

        info!(commit = %commit_id, "Applied changes to working directory");
        Ok(())
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
}

/// File change types.
enum Change {
    Modified(Vec<u8>),
    Created(Vec<u8>),
    #[allow(dead_code)]
    Deleted,
}

/// Compare workspace files against the git HEAD tree.
fn collect_changes(
    repo: &git2::Repository,
    base_tree: &git2::Tree,
    work_dir: &Path,
    repo_path: &Path,
) -> Result<HashMap<String, Change>> {
    let mut changes = HashMap::new();

    // Walk the workspace directory
    walk_dir(work_dir, work_dir, &mut |rel_path| {
        let ws_content = match std::fs::read(work_dir.join(rel_path)) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Check if file exists in base tree
        match base_tree.get_path(Path::new(rel_path)) {
            Ok(entry) => {
                // File exists — check if modified
                if let Ok(blob) = repo.find_blob(entry.id()) {
                    if blob.content() != ws_content.as_slice() {
                        changes.insert(rel_path.to_string(), Change::Modified(ws_content));
                    }
                }
            }
            Err(_) => {
                // File doesn't exist in base tree — new file
                // But only if it also doesn't exist on disk (agent created it)
                let disk_path = repo_path.join(rel_path);
                if !disk_path.exists() || std::fs::read(&disk_path).ok().as_deref() != Some(&ws_content) {
                    changes.insert(rel_path.to_string(), Change::Created(ws_content));
                }
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
    // For nested paths, we need to build trees bottom-up.
    // Group changes by top-level directory.
    build_tree_recursive(repo, base_tree, changes, "")
}

fn build_tree_recursive(
    repo: &git2::Repository,
    base_tree: &git2::Tree,
    changes: &HashMap<String, Change>,
    prefix: &str,
) -> Result<git2::Oid> {
    let mut builder = repo.treebuilder(Some(base_tree))?;

    for (path, change) in changes {
        let rel = if prefix.is_empty() {
            path.as_str()
        } else if let Some(stripped) = path.strip_prefix(prefix) {
            stripped.trim_start_matches('/')
        } else {
            continue; // Not in this subtree
        };

        // If path has '/', it's in a subdirectory — handle recursively
        if let Some(slash_pos) = rel.find('/') {
            let dir_name = &rel[..slash_pos];

            // Get existing subtree or create empty
            let sub_tree = if let Ok(entry) = base_tree.get_path(Path::new(dir_name)) {
                repo.find_tree(entry.id()).ok()
            } else {
                None
            };
            let sub_tree = sub_tree.unwrap_or_else(|| {
                let empty = repo.treebuilder(None).unwrap().write().unwrap();
                repo.find_tree(empty).unwrap()
            });

            // Collect changes for this subdirectory
            let sub_prefix = if prefix.is_empty() {
                dir_name.to_string()
            } else {
                format!("{prefix}/{dir_name}")
            };
            let sub_changes: HashMap<String, Change> = changes.iter()
                .filter(|(p, _)| p.starts_with(&format!("{sub_prefix}/")))
                .map(|(p, c)| (p.clone(), match c {
                    Change::Modified(v) => Change::Modified(v.clone()),
                    Change::Created(v) => Change::Created(v.clone()),
                    Change::Deleted => Change::Deleted,
                }))
                .collect();

            if !sub_changes.is_empty() {
                let sub_tree_id = build_tree_recursive(repo, &sub_tree, &sub_changes, &sub_prefix)?;
                builder.insert(dir_name, sub_tree_id, 0o040000)?;
            }
        } else {
            // Direct file in this directory
            match change {
                Change::Modified(content) | Change::Created(content) => {
                    let blob_id = repo.blob(content)?;
                    builder.insert(rel, blob_id, 0o100644)?;
                }
                Change::Deleted => {
                    let _ = builder.remove(rel);
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
