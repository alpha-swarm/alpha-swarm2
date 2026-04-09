//! Content-addressed blob store trait + in-memory implementation.

use std::collections::HashMap;
use sha2::{Sha256, Digest};

/// SHA256 hex string used as blob key.
pub type BlobHash = String;

/// Content-addressed blob store.
/// Implementations: HashMap (local), NATS Object Store (distributed).
pub trait BlobStore: Send + Sync {
    fn put(&mut self, content: &[u8]) -> BlobHash;
    fn get(&self, hash: &str) -> Option<&[u8]>;
    fn exists(&self, hash: &str) -> bool;
    fn delete(&mut self, hash: &str);
}

/// Hash content to SHA256 hex.
pub fn content_hash(content: &[u8]) -> BlobHash {
    let hash = Sha256::digest(content);
    format!("{:x}", hash)
}

/// In-memory blob store backed by HashMap.
#[derive(Default, Clone)]
pub struct MemoryBlobStore {
    blobs: HashMap<String, Vec<u8>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }

    pub fn total_bytes(&self) -> usize {
        self.blobs.values().map(|v| v.len()).sum()
    }
}

impl BlobStore for MemoryBlobStore {
    fn put(&mut self, content: &[u8]) -> BlobHash {
        let hash = content_hash(content);
        self.blobs.entry(hash.clone()).or_insert_with(|| content.to_vec());
        hash
    }

    fn get(&self, hash: &str) -> Option<&[u8]> {
        self.blobs.get(hash).map(|v| v.as_slice())
    }

    fn exists(&self, hash: &str) -> bool {
        self.blobs.contains_key(hash)
    }

    fn delete(&mut self, hash: &str) {
        self.blobs.remove(hash);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_addressing_is_deterministic() {
        let mut store = MemoryBlobStore::new();
        let h1 = store.put(b"hello world");
        let h2 = store.put(b"hello world");
        assert_eq!(h1, h2);
        assert_eq!(store.blob_count(), 1);
    }

    #[test]
    fn get_returns_content() {
        let mut store = MemoryBlobStore::new();
        let hash = store.put(b"test content");
        assert_eq!(store.get(&hash), Some(b"test content".as_slice()));
    }

    #[test]
    fn different_content_different_hash() {
        let mut store = MemoryBlobStore::new();
        let h1 = store.put(b"foo");
        let h2 = store.put(b"bar");
        assert_ne!(h1, h2);
        assert_eq!(store.blob_count(), 2);
    }
}
