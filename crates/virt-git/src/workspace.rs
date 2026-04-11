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
use crate::store::{BlobStore, BlobHash, MemoryBlobStore};
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
    /// Returns a summary of the workspace state as a String.
    pub fn summary(&self) -> String {
        format!(
            "VirtWorkspace Summary:\n\
             - Files in working tree: {}\n\
             - Has uncommitted changes: {}\n\
             - Number of commits: {}",
            self.list_files().join(", "),
            self.has_changes(),
            self.commits.len()
        )
    }
}

impl VirtWorkspace {
    /// Create an empty workspace.
    pub fn new() -> Self {
        Self {
            base: TreeSnapshot::new(),
            working: TreeSnapshot::new(),
            commits: Vec::new(),
            next_commit: 1,
        }
    }

    /// Create workspace from a set of files (the "base" state).
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
    pub fn load_file(&mut self, store: &mut dyn BlobStore, path: &str, content: &str) {
        self.base.insert(store, path, content.as_bytes());
        self.working.insert(store, path, content.as_bytes());
    }

    /// Read a file from the working tree.
    pub fn read_file(&self, store: &dyn BlobStore, path: &str) -> Option<String> {
        self.working.read_string(store, path)
    }

    /// Write a file to the working tree (not base).
    pub fn write_file(&mut self, store: &mut dyn BlobStore, path: &str, content: &str) {
        self.working.insert(store, path, content.as_bytes());
    }

    /// Delete a file from the working tree.
    pub fn delete_file(&mut self, path: &str) {
        self.working.remove(path);
    }

    /// Check if file exists in working tree.
    pub fn file_exists(&self, path: &str) -> bool {
        self.working.exists(path)
    }

    /// List all files in working tree.
    pub fn list_files(&self) -> Vec<&str> {
        self.working.list_files()
    }

    /// Check if there are uncommitted changes.
    pub fn has_changes(&self) -> bool {
        !self.diff_entries().is_empty()
    }

    /// Get the diff between base and working tree.
    pub fn diff(&self, store: &dyn BlobStore) -> Vec<FileDiff> {
        diff::diff_trees(store, &self.base, &self.working)
    }

    /// Get formatted diff as patch text.
    pub fn diff_text(&self, store: &dyn BlobStore) -> String {
        diff::format_diff(&self.diff(store))
    }

    /// Commit the current working tree state.
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

impl VirtWorkspace {
    /// Fork: create an independent copy. The fork starts from the current working tree.
    pub fn fork(&self, store: &MemoryBlobStore) -> (VirtWorkspace, MemoryBlobStore) {
        let forked = VirtWorkspace {
            base: self.working.clone(),
            working: self.working.clone(),
            commits: Vec::new(),
            next_commit: 1,
        };
        (forked, store.clone())
    }

    /// Merge another workspace's changes into this one (3-way merge).
    /// Uses self.base as the common ancestor.
    /// On conflict (both modified same file), theirs wins.
    /// Returns list of conflicted paths.
    pub fn merge(
        &mut self,
        store: &mut MemoryBlobStore,
        theirs: &VirtWorkspace,
        their_store: &MemoryBlobStore,
    ) -> Vec<String> {
        // Clone entries to avoid borrowing self while we mutate self.working.
        let base_entries = self.base.entries().clone();
        let our_entries = self.working.entries().clone();
        let their_entries = theirs.working.entries().clone();
        let mut conflicts = Vec::new();

        // Collect all paths across all three trees.
        let mut all_paths: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for path in base_entries.keys() {
            all_paths.insert(path.clone());
        }
        for path in our_entries.keys() {
            all_paths.insert(path.clone());
        }
        for path in their_entries.keys() {
            all_paths.insert(path.clone());
        }

        for path in &all_paths {
            let base_hash: Option<&BlobHash> = base_entries.get(path);
            let our_hash: Option<&BlobHash> = our_entries.get(path);
            let their_hash: Option<&BlobHash> = their_entries.get(path);

            let we_changed = our_hash != base_hash;
            let they_changed = their_hash != base_hash;

            if !they_changed {
                // They didn't touch this file; keep ours as-is.
                continue;
            }

            if they_changed && !we_changed {
                // They changed, we didn't: take theirs.
                match their_hash {
                    Some(hash) => {
                        // Copy blob content from their store into ours, then update tree.
                        if let Some(content) = their_store.get(hash) {
                            self.working.insert(store, path, &content);
                        }
                    }
                    None => {
                        // They deleted it.
                        self.working.remove(path);
                    }
                }
            } else {
                // Both changed: take theirs, but record conflict.
                conflicts.push(path.clone());
                match their_hash {
                    Some(hash) => {
                        if let Some(content) = their_store.get(hash) {
                            self.working.insert(store, path, &content);
                        }
                    }
                    None => {
                        // They deleted, we modified: conflict + delete.
                        self.working.remove(path);
                    }
                }
            }
        }

        conflicts
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

    #[test]
    fn test_fork_independence() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("a.rs", "original"),
        ]);

        let (mut forked, mut forked_store) = ws.fork(&store);

        // Write to the fork; original should not see the change.
        forked.write_file(&mut forked_store, "a.rs", "forked change");
        forked.write_file(&mut forked_store, "b.rs", "new in fork");

        assert_eq!(ws.read_file(&store, "a.rs"), Some("original".into()));
        assert!(!ws.file_exists("b.rs"));

        // Write to original; fork should not see the change.
        ws.write_file(&mut store, "a.rs", "original change");

        assert_eq!(forked.read_file(&forked_store, "a.rs"), Some("forked change".into()));
        assert_eq!(ws.read_file(&store, "a.rs"), Some("original change".into()));
    }

    #[test]
    fn test_merge_clean() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("a.rs", "base a"),
            ("b.rs", "base b"),
        ]);

        let (mut forked, mut forked_store) = ws.fork(&store);

        // We change a.rs, they change b.rs -- no overlap.
        ws.write_file(&mut store, "a.rs", "our change to a");
        forked.write_file(&mut forked_store, "b.rs", "their change to b");

        let conflicts = ws.merge(&mut store, &forked, &forked_store);

        assert!(conflicts.is_empty(), "expected no conflicts, got: {:?}", conflicts);
        assert_eq!(ws.read_file(&store, "a.rs"), Some("our change to a".into()));
        assert_eq!(ws.read_file(&store, "b.rs"), Some("their change to b".into()));
    }

    #[test]
    fn test_merge_conflict() {
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("shared.rs", "base content"),
            ("only_ours.rs", "ours"),
        ]);

        let (mut forked, mut forked_store) = ws.fork(&store);

        // Both modify the same file.
        ws.write_file(&mut store, "shared.rs", "our version");
        forked.write_file(&mut forked_store, "shared.rs", "their version");

        let conflicts = ws.merge(&mut store, &forked, &forked_store);

        assert_eq!(conflicts, vec!["shared.rs".to_string()]);
        // Theirs wins on conflict.
        assert_eq!(ws.read_file(&store, "shared.rs"), Some("their version".into()));
        // Untouched file preserved.
        assert_eq!(ws.read_file(&store, "only_ours.rs"), Some("ours".into()));
    }
}
