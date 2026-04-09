//! Tree: a snapshot of file paths → blob hashes.

use std::collections::BTreeMap;
use serde::{Serialize, Deserialize};
use crate::store::{BlobStore, BlobHash, content_hash};

/// A single entry in a tree (file → blob hash).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub blob_hash: BlobHash,
}

/// A tree snapshot: ordered map of file paths to blob hashes.
/// The tree itself is content-addressed (hash of serialized entries).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TreeSnapshot {
    /// Sorted entries for deterministic hashing.
    entries: BTreeMap<String, BlobHash>,
    /// Hash of the serialized tree (computed lazily).
    #[serde(skip)]
    cached_hash: Option<BlobHash>,
}

impl TreeSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a tree from a list of (path, content) pairs.
    pub fn from_files(store: &mut dyn BlobStore, files: &[(&str, &[u8])]) -> Self {
        let mut tree = Self::new();
        for (path, content) in files {
            let hash = store.put(content);
            tree.entries.insert(path.to_string(), hash);
        }
        tree
    }

    /// Insert or update a file.
    pub fn insert(&mut self, store: &mut dyn BlobStore, path: &str, content: &[u8]) {
        let hash = store.put(content);
        self.entries.insert(path.to_string(), hash);
        self.cached_hash = None;
    }

    /// Remove a file.
    pub fn remove(&mut self, path: &str) -> bool {
        self.cached_hash = None;
        self.entries.remove(path).is_some()
    }

    /// Read file content.
    pub fn read(&self, store: &dyn BlobStore, path: &str) -> Option<Vec<u8>> {
        self.entries.get(path)
            .and_then(|hash| store.get(hash))
    }

    /// Read file as UTF-8 string.
    pub fn read_string(&self, store: &dyn BlobStore, path: &str) -> Option<String> {
        self.read(store, path)
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    /// Check if file exists.
    pub fn exists(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    /// List all file paths.
    pub fn list_files(&self) -> Vec<&str> {
        self.entries.keys().map(|s| s.as_str()).collect()
    }

    /// Number of files.
    pub fn file_count(&self) -> usize {
        self.entries.len()
    }

    /// Get the content-addressed hash of this tree.
    pub fn hash(&mut self) -> BlobHash {
        if let Some(ref h) = self.cached_hash {
            return h.clone();
        }
        let serialized = serde_json::to_vec(&self.entries).unwrap_or_default();
        let h = content_hash(&serialized);
        self.cached_hash = Some(h.clone());
        h
    }

    /// Get entries for diffing.
    pub fn entries(&self) -> &BTreeMap<String, BlobHash> {
        &self.entries
    }

    /// Get the blob hash for a path.
    pub fn blob_hash(&self, path: &str) -> Option<&str> {
        self.entries.get(path).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryBlobStore;

    #[test]
    fn tree_from_files() {
        let mut store = MemoryBlobStore::new();
        let tree = TreeSnapshot::from_files(&mut store, &[
            ("src/main.rs", b"fn main() {}"),
            ("Cargo.toml", b"[package]"),
        ]);
        assert_eq!(tree.file_count(), 2);
        assert_eq!(tree.read_string(&store, "src/main.rs"), Some("fn main() {}".into()));
    }

    #[test]
    fn tree_insert_and_remove() {
        let mut store = MemoryBlobStore::new();
        let mut tree = TreeSnapshot::new();
        tree.insert(&mut store, "a.rs", b"aaa");
        tree.insert(&mut store, "b.rs", b"bbb");
        assert_eq!(tree.file_count(), 2);
        tree.remove("a.rs");
        assert_eq!(tree.file_count(), 1);
        assert!(!tree.exists("a.rs"));
        assert!(tree.exists("b.rs"));
    }

    #[test]
    fn tree_hash_is_deterministic() {
        let mut store = MemoryBlobStore::new();
        let mut t1 = TreeSnapshot::from_files(&mut store, &[("a", b"1"), ("b", b"2")]);
        let mut t2 = TreeSnapshot::from_files(&mut store, &[("b", b"2"), ("a", b"1")]);
        assert_eq!(t1.hash(), t2.hash()); // BTreeMap sorts keys
    }
}
