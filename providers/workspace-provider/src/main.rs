//! Workspace provider: NATS service for file operations + git.
//!
//! Exposes `alpha-swarm:workspace/files` via NATS request-reply.
//! Each workspace is an isolated git clone where agents can read/write files.
//!
//! Subjects:
//!   swarm.workspace.create      — create workspace (clone repo)
//!   swarm.workspace.read_file   — read file from workspace
//!   swarm.workspace.write_file  — write file to workspace
//!   swarm.workspace.list_files  — list files in workspace
//!   swarm.workspace.diff        — extract diff (workspace vs HEAD)
//!   swarm.workspace.commit      — create git commit from changes
//!   swarm.workspace.destroy     — clean up workspace

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn, error};

const WORKSPACE_BASE: &str = "/tmp/alpha-swarm/workspaces";

struct WorkspaceState {
    workspaces: HashMap<String, WorkspaceInfo>,
}

struct WorkspaceInfo {
    path: PathBuf,
    repo_path: PathBuf,
}

// --- Request/Response types ---

#[derive(Deserialize)]
struct CreateRequest {
    workspace_id: String,
    repo_path: String,
}

#[derive(Deserialize)]
struct FileRequest {
    workspace_id: String,
    path: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct WorkspaceIdRequest {
    workspace_id: String,
    #[serde(default)]
    message: String,
}

#[derive(Serialize)]
struct GenericResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl GenericResponse {
    fn ok(result: impl Into<String>) -> Self {
        Self { result: Some(result.into()), files: None, error: None }
    }
    fn files(files: Vec<String>) -> Self {
        Self { result: None, files: Some(files), error: None }
    }
    fn err(e: impl std::fmt::Display) -> Self {
        Self { result: None, files: None, error: Some(e.to_string()) }
    }
}

// --- Workspace operations ---

fn create_workspace(state: &mut WorkspaceState, req: &CreateRequest) -> Result<PathBuf> {
    let ws_path = PathBuf::from(WORKSPACE_BASE).join(&req.workspace_id);

    // Clean stale
    if ws_path.exists() {
        let _ = std::process::Command::new("rm").args(["-rf"]).arg(&ws_path).output();
    }

    // Clone repo
    let output = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--single-branch"])
        .arg(&req.repo_path)
        .arg(&ws_path)
        .output()
        .context("git clone failed")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git clone failed: {stderr}");
    }

    state.workspaces.insert(req.workspace_id.clone(), WorkspaceInfo {
        path: ws_path.clone(),
        repo_path: PathBuf::from(&req.repo_path),
    });

    Ok(ws_path)
}

fn read_file(state: &WorkspaceState, ws_id: &str, path: &str) -> Result<String> {
    let ws = state.workspaces.get(ws_id)
        .ok_or_else(|| anyhow::anyhow!("Workspace not found: {ws_id}"))?;
    let full_path = ws.path.join(path);
    std::fs::read_to_string(&full_path)
        .with_context(|| format!("Failed to read {path}"))
}

fn write_file(state: &WorkspaceState, ws_id: &str, path: &str, content: &str) -> Result<()> {
    let ws = state.workspaces.get(ws_id)
        .ok_or_else(|| anyhow::anyhow!("Workspace not found: {ws_id}"))?;
    let full_path = ws.path.join(path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&full_path, content)?;
    Ok(())
}

fn list_files(state: &WorkspaceState, ws_id: &str) -> Result<Vec<String>> {
    let ws = state.workspaces.get(ws_id)
        .ok_or_else(|| anyhow::anyhow!("Workspace not found: {ws_id}"))?;

    let mut files = Vec::new();
    walk_dir(&ws.path, &ws.path, &mut files);
    files.sort();
    Ok(files)
}

fn extract_diff(state: &WorkspaceState, ws_id: &str) -> Result<String> {
    let ws = state.workspaces.get(ws_id)
        .ok_or_else(|| anyhow::anyhow!("Workspace not found: {ws_id}"))?;

    // Use git2 to diff workspace against HEAD
    let repo = git2::Repository::open(&ws.path)?;
    let head = repo.head()?.peel_to_commit()?;
    let head_tree = head.tree()?;

    // Stage all changes
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    let new_tree_id = index.write_tree()?;
    let new_tree = repo.find_tree(new_tree_id)?;

    let diff = repo.diff_tree_to_tree(Some(&head_tree), Some(&new_tree), None)?;
    let mut diff_text = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let prefix = match line.origin() {
            '+' | '-' | ' ' => &line.origin().to_string(),
            _ => "",
        };
        diff_text.push_str(prefix);
        if let Ok(content) = std::str::from_utf8(line.content()) {
            diff_text.push_str(content);
        }
        true
    })?;

    Ok(diff_text)
}

fn commit_changes(state: &WorkspaceState, ws_id: &str, message: &str) -> Result<String> {
    let ws = state.workspaces.get(ws_id)
        .ok_or_else(|| anyhow::anyhow!("Workspace not found: {ws_id}"))?;

    let repo = git2::Repository::open(&ws.path)?;

    // Stage all
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let head = repo.head()?.peel_to_commit()?;
    let sig = git2::Signature::now("alpha-swarm", "agent@alpha-swarm.local")?;

    let commit_id = repo.commit(
        Some("HEAD"), &sig, &sig, message, &tree, &[&head],
    )?;

    Ok(commit_id.to_string())
}

fn destroy_workspace(state: &mut WorkspaceState, ws_id: &str) {
    if let Some(ws) = state.workspaces.remove(ws_id) {
        let _ = std::process::Command::new("rm").args(["-rf"]).arg(&ws.path).output();
        info!(workspace = ws_id, "Destroyed workspace");
    }
}

fn walk_dir(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "target" || name == "node_modules" { continue; }
            walk_dir(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_string_lossy().to_string());
        }
    }
}

// --- NATS service ---

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let nats_url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://127.0.0.1:4223".into());

    let client = async_nats::connect(&nats_url).await?;
    info!("Workspace provider connected to NATS");

    std::fs::create_dir_all(WORKSPACE_BASE)?;

    let state = Arc::new(RwLock::new(WorkspaceState {
        workspaces: HashMap::new(),
    }));

    let mut create_sub = client.subscribe("swarm.workspace.create").await?;
    let mut read_sub = client.subscribe("swarm.workspace.read_file").await?;
    let mut write_sub = client.subscribe("swarm.workspace.write_file").await?;
    let mut list_sub = client.subscribe("swarm.workspace.list_files").await?;
    let mut diff_sub = client.subscribe("swarm.workspace.diff").await?;
    let mut commit_sub = client.subscribe("swarm.workspace.commit").await?;
    let mut destroy_sub = client.subscribe("swarm.workspace.destroy").await?;

    info!("Listening on swarm.workspace.*");

    loop {
        tokio::select! {
            Some(msg) = create_sub.next() => {
                let state = Arc::clone(&state);
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<CreateRequest>(&msg.payload) {
                        Ok(req) => {
                            let mut s = state.write().await;
                            match create_workspace(&mut s, &req) {
                                Ok(path) => {
                                    info!(workspace = %req.workspace_id, path = %path.display(), "Created workspace");
                                    GenericResponse::ok(path.to_string_lossy())
                                }
                                Err(e) => GenericResponse::err(e),
                            }
                        }
                        Err(e) => GenericResponse::err(e),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, serde_json::to_vec(&resp).unwrap_or_default().into()).await;
                    }
                });
            }
            Some(msg) = read_sub.next() => {
                let state = Arc::clone(&state);
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<FileRequest>(&msg.payload) {
                        Ok(req) => {
                            let s = state.read().await;
                            match read_file(&s, &req.workspace_id, &req.path) {
                                Ok(content) => GenericResponse::ok(content),
                                Err(e) => GenericResponse::err(e),
                            }
                        }
                        Err(e) => GenericResponse::err(e),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, serde_json::to_vec(&resp).unwrap_or_default().into()).await;
                    }
                });
            }
            Some(msg) = write_sub.next() => {
                let state = Arc::clone(&state);
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<FileRequest>(&msg.payload) {
                        Ok(req) => {
                            let s = state.read().await;
                            match write_file(&s, &req.workspace_id, &req.path, &req.content) {
                                Ok(()) => GenericResponse::ok("ok"),
                                Err(e) => GenericResponse::err(e),
                            }
                        }
                        Err(e) => GenericResponse::err(e),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, serde_json::to_vec(&resp).unwrap_or_default().into()).await;
                    }
                });
            }
            Some(msg) = list_sub.next() => {
                let state = Arc::clone(&state);
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<WorkspaceIdRequest>(&msg.payload) {
                        Ok(req) => {
                            let s = state.read().await;
                            match list_files(&s, &req.workspace_id) {
                                Ok(files) => GenericResponse::files(files),
                                Err(e) => GenericResponse::err(e),
                            }
                        }
                        Err(e) => GenericResponse::err(e),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, serde_json::to_vec(&resp).unwrap_or_default().into()).await;
                    }
                });
            }
            Some(msg) = diff_sub.next() => {
                let state = Arc::clone(&state);
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<WorkspaceIdRequest>(&msg.payload) {
                        Ok(req) => {
                            let s = state.read().await;
                            match extract_diff(&s, &req.workspace_id) {
                                Ok(diff) => GenericResponse::ok(diff),
                                Err(e) => GenericResponse::err(e),
                            }
                        }
                        Err(e) => GenericResponse::err(e),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, serde_json::to_vec(&resp).unwrap_or_default().into()).await;
                    }
                });
            }
            Some(msg) = commit_sub.next() => {
                let state = Arc::clone(&state);
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<WorkspaceIdRequest>(&msg.payload) {
                        Ok(req) => {
                            let s = state.read().await;
                            let msg = if req.message.is_empty() { "agent commit" } else { &req.message };
                            match commit_changes(&s, &req.workspace_id, msg) {
                                Ok(commit_id) => GenericResponse::ok(commit_id),
                                Err(e) => GenericResponse::err(e),
                            }
                        }
                        Err(e) => GenericResponse::err(e),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, serde_json::to_vec(&resp).unwrap_or_default().into()).await;
                    }
                });
            }
            Some(msg) = destroy_sub.next() => {
                let state = Arc::clone(&state);
                let client = client.clone();
                tokio::spawn(async move {
                    if let Ok(req) = serde_json::from_slice::<WorkspaceIdRequest>(&msg.payload) {
                        let mut s = state.write().await;
                        destroy_workspace(&mut s, &req.workspace_id);
                    }
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, serde_json::to_vec(&GenericResponse::ok("destroyed")).unwrap_or_default().into()).await;
                    }
                });
            }
        }
    }
}
