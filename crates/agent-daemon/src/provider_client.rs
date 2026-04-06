//! Client for calling native providers via NATS request-reply.
//! Falls back to local execution if NATS provider is unavailable.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{info, debug, warn};

/// Timeout for provider calls via NATS.
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(120);

/// Client for the git-provider NATS service.
pub struct GitProviderClient {
    client: Option<async_nats::Client>,
}

#[derive(Serialize)]
struct GitRequest {
    op: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    project: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    repo_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    agent_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    goal: String,
    #[serde(default)]
    quality_passed: bool,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    tokens_in: u32,
    #[serde(default)]
    tokens_out: u32,
}

#[derive(Deserialize)]
struct GitResponse {
    result: Option<String>,
    error: Option<String>,
    files: Option<Vec<String>>,
}

impl GitProviderClient {
    pub async fn new(nats_url: &str) -> Self {
        let client = match async_nats::connect(nats_url).await {
            Ok(c) => { info!("Git provider client connected to NATS"); Some(c) }
            Err(e) => { warn!("Git provider client: NATS unavailable ({e}), using local fallback"); None }
        };
        Self { client }
    }

    pub fn local_only() -> Self {
        Self { client: None }
    }

    /// Ensure a repo is cloned/updated. Returns local path.
    pub async fn ensure_repo(&self, project: &str, url: &str) -> Result<String, String> {
        if let Some(client) = &self.client {
            let resp = self.call(client, "ensure_repo", GitRequest {
                op: "ensure_repo".into(),
                project: project.into(),
                url: url.into(),
                ..Default::default()
            }).await;
            if let Ok(r) = resp { return Ok(r); }
            debug!("NATS git provider unavailable, falling back to local");
        }

        // Local fallback
        crate::repo::ensure_repo(project, url)
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|e| e.to_string())
    }

    /// Create a PR. Returns PR URL.
    pub async fn create_pr(
        &self,
        repo_path: &str,
        goal: &str,
        quality_passed: bool,
        duration_ms: u64,
        tokens_in: u32,
        tokens_out: u32,
    ) -> Result<String, String> {
        if let Some(client) = &self.client {
            let resp = self.call(client, "create_pr", GitRequest {
                op: "create_pr".into(),
                repo_path: repo_path.into(),
                goal: goal.into(),
                quality_passed,
                duration_ms,
                tokens_in,
                tokens_out,
                ..Default::default()
            }).await;
            if let Ok(r) = resp { return Ok(r); }
            debug!("NATS git provider unavailable for PR, falling back to local");
        }

        // Local fallback
        crate::git_pr::create_pr(
            std::path::Path::new(repo_path),
            goal, &[], quality_passed, duration_ms, tokens_in, tokens_out,
        ).map_err(|e| e.to_string())
    }

    async fn call(&self, client: &async_nats::Client, op: &str, req: GitRequest) -> Result<String, String> {
        let subject = format!("swarm.git.{op}");
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize: {e}"))?;

        let reply = tokio::time::timeout(PROVIDER_TIMEOUT, client.request(subject, payload.into()))
            .await
            .map_err(|_| "provider timeout".to_string())?
            .map_err(|e| format!("NATS request: {e}"))?;

        let resp: GitResponse = serde_json::from_slice(&reply.payload)
            .map_err(|e| format!("deserialize: {e}"))?;

        if let Some(err) = resp.error {
            return Err(err);
        }
        resp.result.ok_or_else(|| "no result".into())
    }
}

impl Default for GitRequest {
    fn default() -> Self {
        Self {
            op: String::new(), project: String::new(), url: String::new(),
            repo_path: String::new(), agent_id: String::new(), goal: String::new(),
            quality_passed: false, duration_ms: 0, tokens_in: 0, tokens_out: 0,
        }
    }
}

/// Client for the test-provider NATS service.
pub struct TestProviderClient {
    client: Option<async_nats::Client>,
}

#[derive(Serialize)]
struct TestRequest {
    op: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    repo_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    check_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pattern: String,
}

#[derive(Deserialize)]
pub struct CheckResult {
    pub check_name: String,
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

#[derive(Deserialize)]
struct TestResponse {
    results: Option<Vec<CheckResult>>,
    error: Option<String>,
}

impl TestProviderClient {
    pub async fn new(nats_url: &str) -> Self {
        let client = match async_nats::connect(nats_url).await {
            Ok(c) => { info!("Test provider client connected to NATS"); Some(c) }
            Err(e) => { warn!("Test provider client: NATS unavailable ({e}), using local fallback"); None }
        };
        Self { client }
    }

    /// Run all quality checks. Returns results.
    pub async fn run_all(&self, repo_path: &str) -> Result<Vec<CheckResult>, String> {
        if let Some(client) = &self.client {
            let subject = "swarm.test.run_all";
            let req = TestRequest { op: "run_all".into(), repo_path: repo_path.into(), check_name: String::new(), pattern: String::new() };
            let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize: {e}"))?;

            match tokio::time::timeout(PROVIDER_TIMEOUT, client.request(subject, payload.into())).await {
                Ok(Ok(reply)) => {
                    let resp: TestResponse = serde_json::from_slice(&reply.payload)
                        .map_err(|e| format!("deserialize: {e}"))?;
                    if let Some(err) = resp.error { return Err(err); }
                    return resp.results.ok_or_else(|| "no results".into());
                }
                _ => { debug!("NATS test provider unavailable, falling back to local"); }
            }
        }

        // Local fallback: use quality-gate-lib directly
        Err("test provider not available and local fallback not wired".into())
    }
}
