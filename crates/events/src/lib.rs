// Event types are WASI-portable
mod types;
pub use types::*;

// Native-only: NATS publisher/subscriber/scheduler
#[cfg(feature = "native")]
mod publisher;
#[cfg(feature = "native")]
mod subscriber;
#[cfg(feature = "native")]
pub mod scheduler;

#[cfg(feature = "native")]
pub use publisher::EventPublisher;
#[cfg(feature = "native")]
pub use subscriber::EventSubscriber;
#[cfg(feature = "native")]
pub use scheduler::NatsScheduler;
