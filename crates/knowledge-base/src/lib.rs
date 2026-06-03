// Schema types are WASI-portable (serde only)
mod schema;
pub use schema::*;

// Native-only: SurrealDB client, queries, embeddings
#[cfg(feature = "native")]
mod store;
#[cfg(feature = "native")]
mod queries;
#[cfg(feature = "native")]
mod metrics;
#[cfg(feature = "native")]
pub mod embedding_manager;
#[cfg(feature = "native")]
pub mod memory;
#[cfg(feature = "native")]
pub mod bridge_client;
#[cfg(feature = "native")]
pub mod backend;

#[cfg(feature = "native")]
pub use store::KnowledgeStore;
#[cfg(feature = "native")]
pub use queries::SimilarRun;
#[cfg(feature = "native")]
pub use metrics::{ProjectMetrics, ModelMetrics};
#[cfg(feature = "native")]
pub use memory::MemoryStore;
#[cfg(feature = "native")]
pub use backend::KnowledgeBackend;
