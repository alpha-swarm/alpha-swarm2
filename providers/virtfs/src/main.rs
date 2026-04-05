use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::info;
use wasmcloud_provider_sdk::{run_provider, LinkConfig, Provider};

/// VirtFS capability provider — manages git repos and worktrees.
#[derive(Clone)]
pub struct VirtFsProvider {
    /// Repo paths per linked component.
    repos: Arc<RwLock<HashMap<String, PathBuf>>>,
    /// Base directory for worktrees.
    work_dir: PathBuf,
}

impl VirtFsProvider {
    pub fn new() -> Self {
        let work_dir = std::env::var("ALPHA_SWARM_WORK_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/alpha-swarm/worktrees"));
        info!(work_dir = %work_dir.display(), "VirtFS provider initialized");
        Self {
            repos: Arc::new(RwLock::new(HashMap::new())),
            work_dir,
        }
    }
}

impl Provider for VirtFsProvider {
    fn receive_link_config_as_target(
        &self,
        link: LinkConfig<'_>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let component = link.source_id.to_string();
        let repo_path = link.config
            .iter()
            .find(|(k, _)| k.as_str() == "repo_path")
            .map(|(_, v)| PathBuf::from(v))
            .unwrap_or_else(|| PathBuf::from("."));
        let repos = Arc::clone(&self.repos);

        async move {
            info!(component = %component, repo = %repo_path.display(), "VirtFS link established");
            repos.write().await.insert(component, repo_path);
            Ok(())
        }
    }

    fn delete_link_as_target(
        &self,
        info: impl wasmcloud_provider_sdk::LinkDeleteInfo,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let source_id = info.get_source_id().to_string();
        let repos = Arc::clone(&self.repos);
        async move {
            info!(component = %source_id, "VirtFS link removed");
            repos.write().await.remove(&source_id);
            Ok(())
        }
    }

    fn shutdown(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async {
            info!("VirtFS provider shutting down");
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let provider = VirtFsProvider::new();
    let shutdown = run_provider(provider, "virtfs-provider")
        .await
        .expect("Failed to initialize provider");
    shutdown.await;
    Ok(())
}
