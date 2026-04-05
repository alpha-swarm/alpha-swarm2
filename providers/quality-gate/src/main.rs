use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::info;
use wasmcloud_provider_sdk::{
    get_connection, run_provider, serve_provider_exports, LinkConfig, Provider,
};

mod bindings {
    wit_bindgen_wrpc::generate!({
        world: "quality-gate-provider",
        path: "wit",
        generate_all,
    });
}

use bindings::exports::alpha_swarm::quality_gate_provider::gate;

#[derive(Clone)]
pub struct QualityGateProvider {
    repos: Arc<RwLock<HashMap<String, PathBuf>>>,
}

impl QualityGateProvider {
    pub fn new() -> Self {
        info!("QualityGate provider initialized");
        Self { repos: Arc::new(RwLock::new(HashMap::new())) }
    }
}

impl Provider for QualityGateProvider {
    fn receive_link_config_as_target(
        &self, link: LinkConfig<'_>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let component = link.source_id.to_string();
        let repo_path = link.config
            .iter()
            .find(|(k, _)| k.as_str() == "repo_path")
            .map(|(_, v)| PathBuf::from(v))
            .unwrap_or_else(|| PathBuf::from("."));
        let repos = Arc::clone(&self.repos);
        async move {
            info!(component = %component, repo = %repo_path.display(), "QualityGate link");
            repos.write().await.insert(component, repo_path);
            Ok(())
        }
    }

    fn delete_link_as_target(
        &self, info: impl wasmcloud_provider_sdk::LinkDeleteInfo,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let source_id = info.get_source_id().to_string();
        let repos = Arc::clone(&self.repos);
        async move { repos.write().await.remove(&source_id); Ok(()) }
    }

    fn shutdown(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async { info!("QualityGate shutting down"); Ok(()) }
    }
}

#[derive(Clone)]
pub struct GateHandler {
    default_repo: PathBuf,
}

impl gate::Handler<Option<wasmcloud_provider_sdk::Context>> for GateHandler {
    async fn check_all(
        &self, _ctx: Option<wasmcloud_provider_sdk::Context>,
        _repo: String, _agent: String,
    ) -> anyhow::Result<Result<Vec<gate::CheckResult>, gate::SwarmError>> {
        let config = quality_gate_lib::detect_toolchain(&self.default_repo);
        match quality_gate_lib::run_all(&self.default_repo, &config).await {
            Ok(results) => {
                let checks: Vec<gate::CheckResult> = results.into_iter().map(|r| {
                    gate::CheckResult {
                        check_name: r.check_name,
                        passed: r.passed,
                        stdout: r.stdout,
                        stderr: r.stderr,
                        exit_code: r.exit_code,
                        duration_ms: r.duration_ms,
                    }
                }).collect();
                Ok(Ok(checks))
            }
            Err(e) => Ok(Err(gate::SwarmError::QualityGateFailed(e.to_string()))),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let provider = QualityGateProvider::new();
    let default_repo = std::env::var("ALPHA_SWARM_DEFAULT_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let shutdown = run_provider(provider, "quality-gate-provider")
        .await
        .expect("Failed to initialize provider");

    let connection = get_connection();
    let wrpc_client = connection.get_wrpc_client(connection.provider_key()).await?;

    let handler = GateHandler { default_repo };

    tokio::select! {
        _ = shutdown => info!("Shutdown"),
        result = serve_provider_exports(&wrpc_client, handler, futures::future::pending::<()>(), bindings::serve) => {
            if let Err(e) = result { tracing::error!("wRPC: {e}"); }
        }
    }
    Ok(())
}
