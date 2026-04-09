//! Tree diffing using content hashes + text diff via `similar`.

use crate::store::BlobStore;
use crate::tree::TreeSnapshot;

/// Kind of change in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Modified,
    Deleted,
}

/// A single file difference between two trees.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub kind: DiffKind,
    /// Unified diff text (empty for binary or added/deleted files).
    pub patch: String,
}

/// Diff two tree snapshots. Returns list of changed files.
pub fn diff_trees(
    store: &dyn BlobStore,
    old: &TreeSnapshot,
    new: &TreeSnapshot,
) -> Vec<FileDiff> {
    let mut diffs = Vec::new();

    let old_entries = old.entries();
    let new_entries = new.entries();

    // Check for modified and deleted files
    for (path, old_hash) in old_entries {
        match new_entries.get(path) {
            Some(new_hash) if new_hash != old_hash => {
                // Modified — compute text diff
                let old_content = store.get(old_hash)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .unwrap_or("");
                let new_content = store.get(new_hash)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .unwrap_or("");

                let patch = text_diff(path, old_content, new_content);

                diffs.push(FileDiff {
                    path: path.clone(),
                    kind: DiffKind::Modified,
                    patch,
                });
            }
            Some(_) => {} // Same hash — unchanged
            None => {
                diffs.push(FileDiff {
                    path: path.clone(),
                    kind: DiffKind::Deleted,
                    patch: String::new(),
                });
            }
        }
    }

    // Check for added files
    for path in new_entries.keys() {
        if !old_entries.contains_key(path) {
            let content = new.read_string(store, path).unwrap_or_default();
            diffs.push(FileDiff {
                path: path.clone(),
                kind: DiffKind::Added,
                patch: format!("--- /dev/null\n+++ b/{path}\n{}",
                    content.lines().map(|l| format!("+{l}\n")).collect::<String>()),
            });
        }
    }

    diffs.sort_by(|a, b| a.path.cmp(&b.path));
    diffs
}

/// Generate unified diff text for a single file.
fn text_diff(path: &str, old: &str, new: &str) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut output = format!("--- a/{path}\n+++ b/{path}\n");

    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        output.push_str(&format!("{hunk}"));
    }

    output
}

/// Format all diffs as a single patch string.
pub fn format_diff(diffs: &[FileDiff]) -> String {
    diffs.iter()
        .map(|d| d.patch.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryBlobStore;

    #[test]
    fn diff_detects_added() {
        let store = MemoryBlobStore::new();
        let old = TreeSnapshot::new();
        let mut new_store = store.clone();
        let new = TreeSnapshot::from_files(&mut new_store, &[("new.rs", b"fn new() {}")]);

        let diffs = diff_trees(&new_store, &old, &new);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].kind, DiffKind::Added);
        assert_eq!(diffs[0].path, "new.rs");
    }

    #[test]
    fn diff_detects_modified() {
        let mut store = MemoryBlobStore::new();
        let old = TreeSnapshot::from_files(&mut store, &[("a.rs", b"old content")]);
        let mut new = old.clone();
        new.insert(&mut store, "a.rs", b"new content");

        let diffs = diff_trees(&store, &old, &new);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].kind, DiffKind::Modified);
        assert!(diffs[0].patch.contains("-old content"));
        assert!(diffs[0].patch.contains("+new content"));
    }

    #[test]
    fn diff_detects_deleted() {
        let mut store = MemoryBlobStore::new();
        let old = TreeSnapshot::from_files(&mut store, &[("a.rs", b"content")]);
        let new = TreeSnapshot::new();

        let diffs = diff_trees(&store, &old, &new);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].kind, DiffKind::Deleted);
    }

    #[test]
    fn unchanged_files_not_in_diff() {
        let mut store = MemoryBlobStore::new();
        let old = TreeSnapshot::from_files(&mut store, &[
            ("a.rs", b"unchanged"),
            ("b.rs", b"will change"),
        ]);
        let mut new = old.clone();
        new.insert(&mut store, "b.rs", b"changed");

        let diffs = diff_trees(&store, &old, &new);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "b.rs");
    }
}
