// Planner types are portable, execution needs native inference
pub mod planner_types;
#[cfg(feature = "native")]
mod planner;

// Native-only: git operations, file I/O, tokio runtime
#[cfg(feature = "native")]
mod worktree;
#[cfg(feature = "native")]
mod memtree;
#[cfg(feature = "native")]
mod runner;

pub use planner_types::SubTask;
#[cfg(feature = "native")]
pub use planner::plan_goal;
#[cfg(feature = "native")]
pub use worktree::WorktreeManager;
#[cfg(feature = "native")]
pub use memtree::MemTreeManager;
#[cfg(feature = "native")]
pub use runner::{SwarmRunner, SwarmResult, GitHubRepo};
