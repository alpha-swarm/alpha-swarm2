// Planner is WASI-portable (only needs inference types + serde)
#[cfg(feature = "native")]
mod planner;

// Native-only: git operations, file I/O, tokio runtime
#[cfg(feature = "native")]
mod worktree;
#[cfg(feature = "native")]
mod memtree;
#[cfg(feature = "native")]
mod runner;

#[cfg(feature = "native")]
pub use planner::{SubTask, plan_goal};
#[cfg(feature = "native")]
pub use worktree::WorktreeManager;
#[cfg(feature = "native")]
pub use memtree::MemTreeManager;
#[cfg(feature = "native")]
pub use runner::{SwarmRunner, SwarmResult};
