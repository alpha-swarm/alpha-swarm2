//! virt-git: In-memory content-addressed file store with git-like operations.
//!
//! Fully WASI-portable — no filesystem, no git2, no native deps.
//! Backed by any key-value store (HashMap for local, NATS Object Store for distributed).
//!
//! # Architecture
//!
//! ```text
//! Blob:   store[sha256(content)] = content bytes
//! Tree:   store[sha256(entries)] = [TreeEntry { name, blob_sha }]
//! Commit: store["commit/{id}"]  = { tree_sha, parent, message, timestamp }
//! ```

mod store;
mod tree;
mod diff;
mod workspace;

mod wasi_store;
pub mod github;
pub mod github_loader;
pub mod file_provider;

pub use store::{BlobStore, MemoryBlobStore, content_hash};
pub use tree::{TreeEntry, TreeSnapshot};
pub use diff::{FileDiff, DiffKind, diff_trees, format_diff};
pub use workspace::{VirtWorkspace, CommitInfo};
pub use wasi_store::WasiBlobStoreAdapter;
pub use file_provider::{FileProvider, DiskFileProvider, VirtFileProvider};
pub use github::{GitHubConfig, PrResult, create_pr};
pub use github_loader::{load_repo_into_workspace, load_file_from_github};

#[cfg(feature = "nats")]
pub use store::NatsBlobStore;
