//! Content-addressed blob store trait + implementations.
//!
//! - `MemoryBlobStore`: in-memory HashMap (local, fast, WASI-portable)
//! - `NatsBlobStore`: NATS JetStream Object Store (distributed, persistent)

use std::collections::HashMap;
use sha2::{Sha256, Digest};

/// SHA256 hex string used as blob key.
pub type BlobHash = String;

/// Computes the SHA256 hash of the provided content and returns it as a hexadecimal string.
pub fn content_hash(content: &[u8]) -> BlobHash {
    let hash = Sha256::digest(content);
    format!("{:x}", hash)
}

/// Content-addressed blob store (synchronous).
/// For in-memory and WASI-portable use cases.
pub trait BlobStore: Send + Sync {
    fn put(&mut self, content: &[u8]) -> BlobHash;
    fn get(&self, hash: &str) -> Option<Vec<u8>>;
    fn exists(&self, hash: &str) -> bool;
    fn delete(&mut self, hash: &str);
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

    fn get(&self, hash: &str) -> Option<Vec<u8>> {
        self.blobs.get(hash).cloned()
    }

    fn exists(&self, hash: &str) -> bool {
        self.blobs.contains_key(hash)
    }

    fn delete(&mut self, hash: &str) {
        self.blobs.remove(hash);
    }
}

/// NATS JetStream Object Store backed blob store.
/// Distributed, persistent — agents on any machine share the same blobs.
///
/// Requires `native` feature (async-nats is not WASI-compatible).
#[cfg(feature = "nats")]
pub struct NatsBlobStore {
    store: async_nats::jetstream::object_store::ObjectStore,
    /// Local cache to avoid repeated fetches.
    cache: HashMap<String, Vec<u8>>,
    /// Runtime handle for sync trait impl over async NATS.
    rt: tokio::runtime::Handle,
}

#[cfg(feature = "nats")]
impl NatsBlobStore {
    /// Create from an existing NATS JetStream Object Store.
    pub fn new(store: async_nats::jetstream::object_store::ObjectStore, rt: tokio::runtime::Handle) -> Self {
        Self { store, cache: HashMap::new(), rt }
    }

    /// Connect to NATS and create/open an object store bucket.
    pub async fn connect(nats_url: &str, bucket: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let client = async_nats::connect(nats_url).await?;
        let jetstream = async_nats::jetstream::new(client);

        let store = jetstream.create_object_store(
            async_nats::jetstream::object_store::Config {
                bucket: bucket.to_string(),
                ..Default::default()
            }
        ).await?;

        Ok(Self {
            store,
            cache: HashMap::new(),
            rt: tokio::runtime::Handle::current(),
        })
    }
}

#[cfg(feature = "nats")]
impl BlobStore for NatsBlobStore {
    fn put(&mut self, content: &[u8]) -> BlobHash {
        let hash = content_hash(content);

        if self.cache.contains_key(&hash) {
            return hash;
        }

        let store = self.store.clone();
        let content_owned = content.to_vec();
        let key = format!("blob/{hash}");

        let _ = self.rt.block_on(async {
            use tokio::io::AsyncReadExt;
            let mut reader = &content_owned[..];
            store.put(key.as_str(), &mut reader).await
        });

        self.cache.insert(hash.clone(), content_owned);
        hash
    }

    fn get(&self, hash: &str) -> Option<Vec<u8>> {
        if let Some(cached) = self.cache.get(hash) {
            return Some(cached.clone());
        }

        let store = self.store.clone();
        let key = format!("blob/{hash}");

        self.rt.block_on(async {
            match store.get(&key).await {
                Ok(mut obj) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    obj.read_to_end(&mut buf).await.ok()?;
                    Some(buf)
                }
                Err(_) => None,
            }
        })
    }

    fn exists(&self, hash: &str) -> bool {
        if self.cache.contains_key(hash) {
            return true;
        }

        let store = self.store.clone();
        let key = format!("blob/{hash}");

        self.rt.block_on(async {
            store.info(&key).await.is_ok()
        })
    }

    fn delete(&mut self, hash: &str) {
        self.cache.remove(hash);
        let store = self.store.clone();
        let key = format!("blob/{hash}");
        let _ = self.rt.block_on(async { store.delete(&key).await });
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
        assert_eq!(store.get(&hash), Some(b"test content".to_vec()));
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
