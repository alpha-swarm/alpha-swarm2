use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::info;
use wasmcloud_provider_sdk::{run_provider, LinkConfig, Provider};

/// QualityGate capability provider — runs build/lint/fmt/test.
#[derive(Clone)]
pub struct QualityGateProvider {
    /// Repo paths per linked component (for toolchain detection + execution).
    repos: Arc<RwLock<HashMap<String, PathBuf>>>,
}

impl QualityGateProvider {
    pub fn new() -> Self {
        info!("QualityGate provider initialized");
        Self {
            repos: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Provider for QualityGateProvider {
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
            let config = quality_gate_lib::detect_toolchain(&repo_path);
            info!(
                component = %component,
                repo = %repo_path.display(),
                build = ?config.build_cmd,
                "QualityGate link established, toolchain detected"
            );
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
            info!(component = %source_id, "QualityGate link removed");
            repos.write().await.remove(&source_id);
            Ok(())
        }
    }

    fn shutdown(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async {
            info!("QualityGate provider shutting down");
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let provider = QualityGateProvider::new();
    let shutdown = run_provider(provider, "quality-gate-provider")
        .await
        .expect("Failed to initialize provider");
    shutdown.await;
    Ok(())
}
