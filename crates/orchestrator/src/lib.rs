// Planner types are WASI-portable
pub mod planner_types;
pub use planner_types::SubTask;

// Native-only: inference, git2, tokio runtime
#[cfg(feature = "native")]
mod planner;
#[cfg(feature = "native")]
mod memtree;
#[cfg(feature = "native")]
mod runner;

#[cfg(feature = "native")]
pub use planner::plan_goal;
#[cfg(feature = "native")]
pub use memtree::MemTreeManager;
#[cfg(feature = "native")]
pub use runner::{SwarmRunner, SwarmResult, GitHubRepo, PhaseTimings};
