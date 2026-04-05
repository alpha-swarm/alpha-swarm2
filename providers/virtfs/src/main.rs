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
        world: "virtfs-provider",
        path: "wit",
        generate_all,
    });
}

use bindings::exports::alpha_swarm::virtfs_provider::repository;

#[derive(Clone)]
pub struct VirtFsProvider {
    repos: Arc<RwLock<HashMap<String, PathBuf>>>,
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
            info!(component = %component, repo = %repo_path.display(), "VirtFS link");
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
        async move { repos.write().await.remove(&source_id); Ok(()) }
    }

    fn shutdown(&self) -> impl std::future::Future<Output = Result<()>> + Send {
        async { info!("VirtFS shutting down"); Ok(()) }
    }
}

#[derive(Clone)]
pub struct VirtFsHandler {
    repos: Arc<RwLock<HashMap<String, PathBuf>>>,
    default_repo: PathBuf,
}

impl repository::Handler<Option<wasmcloud_provider_sdk::Context>> for VirtFsHandler {
    async fn read_file(
        &self, _ctx: Option<wasmcloud_provider_sdk::Context>,
        _repo: String, _agent: String, path: String,
    ) -> anyhow::Result<Result<String, repository::SwarmError>> {
        let repo_path = self.default_repo.clone();
        let full = repo_path.join(&path);
        match std::fs::read_to_string(&full) {
            Ok(content) => Ok(Ok(content)),
            Err(e) => Ok(Err(repository::SwarmError::IoError(format!("{}: {e}", full.display())))),
        }
    }

    async fn write_file(
        &self, _ctx: Option<wasmcloud_provider_sdk::Context>,
        _repo: String, _agent: String, path: String, content: String,
    ) -> anyhow::Result<Result<(), repository::SwarmError>> {
        let full = self.default_repo.join(&path);
        if let Some(parent) = full.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&full, content) {
            Ok(()) => Ok(Ok(())),
            Err(e) => Ok(Err(repository::SwarmError::IoError(e.to_string()))),
        }
    }

    async fn list_files(
        &self, _ctx: Option<wasmcloud_provider_sdk::Context>,
        _repo: String, _agent: String, glob: String,
    ) -> anyhow::Result<Result<Vec<String>, repository::SwarmError>> {
        // Simple recursive listing — glob ignored for now
        let mut files = Vec::new();
        fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    if !name.starts_with('.') && name != "target" { walk(&p, base, out); }
                } else if let Ok(rel) = p.strip_prefix(base) {
                    out.push(rel.to_string_lossy().to_string());
                }
            }
        }
        walk(&self.default_repo, &self.default_repo, &mut files);
        files.sort();
        Ok(Ok(files))
    }

    async fn extract_diff(
        &self, _ctx: Option<wasmcloud_provider_sdk::Context>,
        _repo: String, _agent: String,
    ) -> anyhow::Result<Result<String, repository::SwarmError>> {
        // Run git diff in the repo
        match tokio::process::Command::new("git")
            .args(["diff"])
            .current_dir(&self.default_repo)
            .output()
            .await
        {
            Ok(output) => Ok(Ok(String::from_utf8_lossy(&output.stdout).to_string())),
            Err(e) => Ok(Err(repository::SwarmError::IoError(e.to_string()))),
        }
    }

    async fn read_files(
        &self, ctx: Option<wasmcloud_provider_sdk::Context>,
        repo: String, agent: String, paths: Vec<String>,
    ) -> anyhow::Result<Result<Vec<repository::FileEntry>, repository::SwarmError>> {
        let mut entries = Vec::new();
        for path in paths {
            match self.read_file(ctx.clone(), repo.clone(), agent.clone(), path.clone()).await? {
                Ok(content) => entries.push(repository::FileEntry { path, content }),
                Err(e) => return Ok(Err(e)),
            }
        }
        Ok(Ok(entries))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let provider = VirtFsProvider::new();
    let repos = Arc::clone(&provider.repos);
    let default_repo = std::env::var("ALPHA_SWARM_DEFAULT_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    let shutdown = run_provider(provider, "virtfs-provider")
        .await
        .expect("Failed to initialize provider");

    let connection = get_connection();
    let wrpc_client = connection.get_wrpc_client(connection.provider_key()).await?;

    let handler = VirtFsHandler { repos, default_repo };

    tokio::select! {
        _ = shutdown => info!("Shutdown"),
        result = serve_provider_exports(&wrpc_client, handler, futures::future::pending::<()>(), bindings::serve) => {
            if let Err(e) = result { tracing::error!("wRPC: {e}"); }
        }
    }
    Ok(())
}
