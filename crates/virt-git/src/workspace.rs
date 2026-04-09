//! VirtWorkspace: an agent's isolated file workspace with commit/diff.
//!
//! Usage:
//! ```ignore
//! let mut ws = VirtWorkspace::new();
//! ws.load_files(&mut store, &[("src/main.rs", "fn main() {}")]);
//! ws.write_file(&mut store, "src/main.rs", "/// Doc\nfn main() {}");
//! let diff = ws.diff(&store);
//! let commit = ws.commit("added doc comment");
//! ```

use serde::{Serialize, Deserialize};
use crate::store::{BlobStore, BlobHash};
use crate::tree::TreeSnapshot;
use crate::diff::{self, FileDiff};

/// Commit metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub id: String,
    pub tree_hash: BlobHash,
    pub parent: Option<String>,
    pub message: String,
    pub timestamp: u64,
}

/// An isolated workspace with base snapshot + working tree.
pub struct VirtWorkspace {
    /// The base tree (what the agent started with).
    base: TreeSnapshot,
    /// The working tree (base + agent's edits).
    working: TreeSnapshot,
    /// Commit history.
    commits: Vec<CommitInfo>,
    /// Auto-incrementing commit counter.
    next_commit: u32,
}

impl VirtWorkspace {
    /// Create an empty workspace.
    /// Creates a new empty workspace.
    pub fn new() -> Self {
        Self {
            base: TreeSnapshot::new(),
            working: TreeSnapshot::new(),
            commits: Vec::new(),
            next_commit: 1,
        }
    }

    /// Create workspace from a set of files (the "base" state).
    /// Creates a workspace from a set of initial files.
    pub fn from_files(store: &mut dyn BlobStore, files: &[(&str, &str)]) -> Self {
        let file_bytes: Vec<(&str, &[u8])> = files.iter()
            .map(|(p, c)| (*p, c.as_bytes()))
            .collect();
        let tree = TreeSnapshot::from_files(store, &file_bytes);
        Self {
            base: tree.clone(),
            working: tree,
            commits: Vec::new(),
            next_commit: 1,
        }
    }

    /// Load a file into the base + working tree.
    /// Loads a file into the base and working trees.
    pub fn load_file(&mut self, store: &mut dyn BlobStore, path: &str, content: &str) {
        self.base.insert(store, path, content.as_bytes());
        self.working.insert(store, path, content.as_bytes());
    }

    /// Read a file from the working tree.
    /// Reads a file from the working tree.
    pub fn read_file(&self, store: &dyn BlobStore, path: &str) -> Option<String> {
        self.working.read_string(store, path)
    }

    /// Write a file to the working tree (not base).
    /// Writes a file to the working tree.
    pub fn write_file(&mut self, store: &mut dyn BlobStore, path: &str, content: &str) {
        self.working.insert(store, path, content.as_bytes());
    }

    /// Delete a file from the working tree.
    /// Deletes a file from the working tree.
    pub fn delete_file(&mut self, path: &str) {
        self.working.remove(path);
    }

    /// Check if file exists in working tree.
    /// Checks if a file exists in the working tree.
    pub fn file_exists(&self, path: &str) -> bool {
        self.working.exists(path)
    }

    /// List all files in working tree.
    /// Lists all files in the working tree.
    pub fn list_files(&self) -> Vec<&str> {
        self.working.list_files()
    }

    /// Check if there are uncommitted changes.
    /// Checks if there are uncommitted changes in the working tree.
    pub fn has_changes(&self) -> bool {
        !self.diff_entries().is_empty()
    }

    /// Get the diff between base and working tree.
    /// Gets the diff between the base and working trees.
    pub fn diff(&self, store: &dyn BlobStore) -> Vec<FileDiff> {
        diff::diff_trees(store, &self.base, &self.working)
    }

    /// Get formatted diff as patch text.
    /// Gets the formatted diff as patch text.
    pub fn diff_text(&self, store: &dyn BlobStore) -> String {
        diff::format_diff(&self.diff(store))
    }

    /// Commit the current working tree state.
    /// Commits the current state of the working tree.
    pub fn commit(&mut self, message: &str) -> CommitInfo {
        let tree_hash = self.working.hash();
        let parent = self.commits.last().map(|c| c.id.clone());
        let id = format!("commit-{:04}", self.next_commit);
        self.next_commit += 1;

        let commit = CommitInfo {
            id: id.clone(),
            tree_hash,
            parent,
            message: message.to_string(),
            timestamp: current_timestamp(),
        };

        // After commit, base = working (no more pending changes)
        self.base = self.working.clone();
        self.commits.push(commit.clone());
        commit
    }

    /// Get commit history.
    /// Gets the commit history.
    pub fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }

    /// Get changed file paths (quick check without full diff).
    fn diff_entries(&self) -> Vec<String> {
        let base = self.base.entries();
        let work = self.working.entries();
        let mut changed = Vec::new();

        for (path, hash) in work {
            match base.get(path) {
                Some(base_hash) if base_hash != hash => changed.push(path.clone()),
                None => changed.push(path.clone()),
                _ => {}
            }
        }
        for path in base.keys() {
            if !work.contains_key(path) {
                changed.push(path.clone());
            }
        }

        changed
    }
}

impl Default for VirtWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

fn current_timestamp() -> u64 {
    // WASI-compatible: use std time (works on wasm32-wasip2)
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryBlobStore;

    #[test]
    fn workspace_read_write() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("src/main.rs", "fn main() {}"),
        ]);

        assert_eq!(ws.read_file(&store, "src/main.rs"), Some("fn main() {}".into()));
        ws.write_file(&mut store, "src/main.rs", "/// Doc\nfn main() {}");
        assert_eq!(ws.read_file(&store, "src/main.rs"), Some("/// Doc\nfn main() {}".into()));
    }

    #[test]
    fn workspace_diff_shows_changes() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("a.rs", "old"),
        ]);

        assert!(!ws.has_changes());
        ws.write_file(&mut store, "a.rs", "new");
        assert!(ws.has_changes());

        let diffs = ws.diff(&store);
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].patch.contains("-old"));
        assert!(diffs[0].patch.contains("+new"));
    }

    #[test]
    fn workspace_commit_resets_base() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("a.rs", "v1"),
        ]);

        ws.write_file(&mut store, "a.rs", "v2");
        assert!(ws.has_changes());

        let commit = ws.commit("update a");
        assert!(!ws.has_changes());
        assert_eq!(commit.message, "update a");
        assert_eq!(ws.commits().len(), 1);
    }

    #[test]
    fn workspace_multiple_commits() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::new();

        ws.load_file(&mut store, "a.rs", "v1");
        ws.commit("initial");

        ws.write_file(&mut store, "a.rs", "v2");
        let c2 = ws.commit("update");

        assert_eq!(ws.commits().len(), 2);
        assert_eq!(c2.parent, Some("commit-0001".into()));
    }

    #[test]
    fn workspace_add_and_delete() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("keep.rs", "keep"),
            ("delete.rs", "delete me"),
        ]);

        ws.delete_file("delete.rs");
        ws.write_file(&mut store, "new.rs", "new file");

        let diffs = ws.diff(&store);
        assert_eq!(diffs.len(), 2); // 1 deleted + 1 added
    }
}
