mod types;
mod backend;

// Native-only modules (reqwest, tokio, tracing)
#[cfg(feature = "native")]
mod claude;
#[cfg(feature = "native")]
mod ollama;
#[cfg(feature = "native")]
mod router;
#[cfg(feature = "native")]
mod openai_compat;
#[cfg(feature = "native")]
pub mod mock;

pub use types::*;
pub use backend::InferenceBackend;

#[cfg(feature = "native")]
pub use claude::ClaudeBackend;
#[cfg(feature = "native")]
pub use ollama::{OllamaBackend, OllamaTool, OllamaToolFunction, OllamaToolCall, OllamaToolCallFunction, OllamaMessage};
#[cfg(feature = "native")]
pub use router::InferenceRouter;
#[cfg(feature = "native")]
pub use openai_compat::OpenAICompatBackend;
