//! Adapter: wasi:blobstore → BlobStore trait.
//!
//! Used inside WASI components that import wasi:blobstore.
//! The wasmCloud host links blobstore-nats-provider at runtime.
//!
//! # Usage in a WASI component
//!
//! ```ignore
//! // In your component's wit world:
//! // import wasi:blobstore/blobstore;
//!
//! use virt_git::{VirtWorkspace, WasiBlobStoreAdapter};
//!
//! let mut store = WasiBlobStoreAdapter::new("agent-workspace-123");
//! let mut ws = VirtWorkspace::from_files(&mut store, &[("main.rs", "fn main() {}")]);
//! ws.write_file(&mut store, "main.rs", "/// Updated\nfn main() {}");
//! let diff = ws.diff_text(&store);
//! ```
//!
//! # wadm manifest
//!
//! ```yaml
//! - name: agent-worker
//!   type: component
//!   properties:
//!     image: file://agent-worker.wasm
//!   traits:
//!     - type: link
//!       properties:
//!         target: blobstore-nats
//!         namespace: wasi
//!         package: blobstore
//!         interfaces: [blobstore]
//!         source_config:
//!           - name: nats-config
//!             properties:
//!               bucket_name: agent-workspaces
//!
//! - name: blobstore-nats
//!   type: capability
//!   properties:
//!     image: ghcr.io/wasmcloud/blobstore-nats:0.3.0
//! ```

use std::collections::HashMap;
use crate::store::{BlobStore, BlobHash, content_hash};

/// Adapter bridging wasi:blobstore to the BlobStore trait.
///
/// When compiled as a native library (not WASI), this acts as a cache-only store.
/// When compiled as part of a WASI component with wasi:blobstore imported,
/// the component code calls the generated wasi bindings and delegates here.
pub struct WasiBlobStoreAdapter {
    /// Container/bucket name in NATS.
    pub container: String,
    /// Local cache to avoid repeated host calls.
    cache: HashMap<String, Vec<u8>>,
    /// Callback for put operations (set by component code).
    put_fn: Option<Box<dyn Fn(&str, &[u8]) + Send + Sync>>,
    /// Callback for get operations.
    get_fn: Option<Box<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>>,
    /// Callback for exists operations.
    exists_fn: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
    /// Callback for delete operations.
    delete_fn: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl WasiBlobStoreAdapter {
    /// Create with just a container name (cache-only mode).
    pub fn new(container: &str) -> Self {
        Self {
            container: container.to_string(),
            cache: HashMap::new(),
            put_fn: None,
            get_fn: None,
            exists_fn: None,
            delete_fn: None,
        }
    }

    /// Wire up the WASI blobstore callbacks.
    /// Called by the component after wit_bindgen generates the blobstore imports.
    pub fn with_callbacks(
        mut self,
        put: impl Fn(&str, &[u8]) + Send + Sync + 'static,
        get: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static,
        exists: impl Fn(&str) -> bool + Send + Sync + 'static,
        delete: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        self.put_fn = Some(Box::new(put));
        self.get_fn = Some(Box::new(get));
        self.exists_fn = Some(Box::new(exists));
        self.delete_fn = Some(Box::new(delete));
        self
    }
}

impl BlobStore for WasiBlobStoreAdapter {
    fn put(&mut self, content: &[u8]) -> BlobHash {
        let hash = content_hash(content);
        if self.cache.contains_key(&hash) {
            return hash;
        }

        let key = format!("blob/{hash}");
        if let Some(ref put_fn) = self.put_fn {
            put_fn(&key, content);
        }

        self.cache.insert(hash.clone(), content.to_vec());
        hash
    }

    fn get(&self, hash: &str) -> Option<Vec<u8>> {
        if let Some(cached) = self.cache.get(hash) {
            return Some(cached.clone());
        }

        let key = format!("blob/{hash}");
        if let Some(ref get_fn) = self.get_fn {
            return get_fn(&key);
        }

        None
    }

    fn exists(&self, hash: &str) -> bool {
        if self.cache.contains_key(hash) {
            return true;
        }

        let key = format!("blob/{hash}");
        if let Some(ref exists_fn) = self.exists_fn {
            return exists_fn(&key);
        }

        false
    }

    fn delete(&mut self, hash: &str) {
        self.cache.remove(hash);

        let key = format!("blob/{hash}");
        if let Some(ref delete_fn) = self.delete_fn {
            delete_fn(&key);
        }
    }
}
