//! Test runner provider — NATS service that executes quality checks.
//!
//! Subscribes to `swarm.test.*` subjects and runs cargo/npm/go checks.
//! Runs natively on machines with build toolchains installed.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Max output bytes per check to avoid flooding responses.
const MAX_OUTPUT_BYTES: usize = 10_000;

#[derive(Deserialize)]
struct TestRequest {
    op: String,
    #[serde(default)]
    repo_path: String,
    #[serde(default)]
    check_name: String,
    #[serde(default)]
    pattern: String,
}

#[derive(Serialize)]
struct CheckResult {
    check_name: String,
    passed: bool,
    stdout: String,
    stderr: String,
    exit_code: i32,
    duration_ms: u64,
}

#[derive(Serialize)]
struct TestResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    results: Option<Vec<CheckResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toolchain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl TestResponse {
    fn err(e: impl Into<String>) -> Self {
        Self { results: None, toolchain: None, checks: None, error: Some(e.into()) }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4223".into());
    info!(nats_url = %nats_url, "Test provider starting");

    let client = async_nats::connect(&nats_url).await
        .context("Failed to connect to NATS")?;

    let mut sub = client.subscribe("swarm.test.*").await
        .context("Failed to subscribe")?;

    info!("Test provider listening on swarm.test.*");

    while let Some(msg) = sub.next().await {
        let reply = msg.reply.clone();
        let payload = msg.payload.to_vec();

        let response = match serde_json::from_slice::<TestRequest>(&payload) {
            Ok(req) => handle_request(&req),
            Err(e) => TestResponse::err(format!("invalid request: {e}")),
        };

        if let Some(reply_to) = reply {
            let resp_bytes = serde_json::to_vec(&response).unwrap_or_default();
            let _ = client.publish(reply_to, resp_bytes.into()).await;
        }
    }

    Ok(())
}

fn handle_request(req: &TestRequest) -> TestResponse {
    match req.op.as_str() {
        "detect_toolchain" => op_detect_toolchain(&req.repo_path),
        "run_all" => op_run_all(&req.repo_path),
        "run_check" => op_run_check(&req.repo_path, &req.check_name),
        "run_tests" => op_run_tests(&req.repo_path, &req.pattern),
        _ => TestResponse::err(format!("unknown op: {}", req.op)),
    }
}

fn op_detect_toolchain(repo_path: &str) -> TestResponse {
    let p = Path::new(repo_path);
    let (name, checks) = if p.join("Cargo.toml").exists() {
        ("rust", vec!["fmt", "clippy", "build", "test"])
    } else if p.join("package.json").exists() {
        ("node", vec!["lint", "build", "test"])
    } else if p.join("go.mod").exists() {
        ("go", vec!["vet", "build", "test"])
    } else {
        return TestResponse::err("no recognized toolchain");
    };

    TestResponse {
        results: None,
        toolchain: Some(name.into()),
        checks: Some(checks.iter().map(|s| s.to_string()).collect()),
        error: None,
    }
}

fn op_run_all(repo_path: &str) -> TestResponse {
    let p = Path::new(repo_path);
    let checks: Vec<(&str, &str, &[&str])> = if p.join("Cargo.toml").exists() {
        vec![
            ("fmt", "cargo", &["fmt", "--", "--check"]),
            ("clippy", "cargo", &["clippy", "--", "-D", "warnings"]),
            ("build", "cargo", &["build"]),
            ("test", "cargo", &["test"]),
        ]
    } else if p.join("package.json").exists() {
        vec![
            ("lint", "npm", &["run", "lint"]),
            ("build", "npm", &["run", "build"]),
            ("test", "npm", &["test"]),
        ]
    } else if p.join("go.mod").exists() {
        vec![
            ("vet", "go", &["vet", "./..."]),
            ("build", "go", &["build", "./..."]),
            ("test", "go", &["test", "./..."]),
        ]
    } else {
        return TestResponse::err("no recognized toolchain");
    };

    let results: Vec<CheckResult> = checks.iter().map(|(name, cmd, args)| {
        run_check(repo_path, name, cmd, args)
    }).collect();

    TestResponse { results: Some(results), toolchain: None, checks: None, error: None }
}

fn op_run_check(repo_path: &str, check_name: &str) -> TestResponse {
    let p = Path::new(repo_path);
    let (cmd, args): (&str, &[&str]) = if p.join("Cargo.toml").exists() {
        match check_name {
            "fmt" => ("cargo", &["fmt", "--", "--check"]),
            "clippy" => ("cargo", &["clippy", "--", "-D", "warnings"]),
            "build" => ("cargo", &["build"]),
            "test" => ("cargo", &["test"]),
            _ => return TestResponse::err(format!("unknown check: {check_name}")),
        }
    } else {
        return TestResponse::err("no recognized toolchain");
    };

    let result = run_check(repo_path, check_name, cmd, args);
    TestResponse { results: Some(vec![result]), toolchain: None, checks: None, error: None }
}

fn op_run_tests(repo_path: &str, pattern: &str) -> TestResponse {
    let p = Path::new(repo_path);
    let (cmd, args) = if p.join("Cargo.toml").exists() {
        if pattern.is_empty() {
            ("cargo", vec!["test", "--", "--nocapture"])
        } else {
            ("cargo", vec!["test", pattern, "--", "--nocapture"])
        }
    } else if p.join("package.json").exists() {
        ("npm", vec!["test"])
    } else if p.join("go.mod").exists() {
        ("go", vec!["test", "./..."])
    } else {
        return TestResponse::err("no recognized toolchain");
    };

    let args_ref: Vec<&str> = args.iter().map(|s| &**s).collect();
    let result = run_check(repo_path, "test", cmd, &args_ref);
    TestResponse { results: Some(vec![result]), toolchain: None, checks: None, error: None }
}

fn run_check(repo_path: &str, name: &str, cmd: &str, args: &[&str]) -> CheckResult {
    let start = Instant::now();
    info!(check = name, cmd = cmd, "Running check");

    match Command::new(cmd).args(args).current_dir(repo_path).output() {
        Ok(output) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let passed = output.status.success();
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = truncate_output(&output.stdout);
            let stderr = truncate_output(&output.stderr);

            if passed {
                info!(check = name, duration_ms, "PASSED");
            } else {
                warn!(check = name, duration_ms, exit_code, "FAILED");
            }

            CheckResult { check_name: name.into(), passed, stdout, stderr, exit_code, duration_ms }
        }
        Err(e) => CheckResult {
            check_name: name.into(),
            passed: false,
            stdout: String::new(),
            stderr: format!("failed to run {cmd}: {e}"),
            exit_code: -1,
            duration_ms: start.elapsed().as_millis() as u64,
        },
    }
}

fn truncate_output(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() > MAX_OUTPUT_BYTES {
        format!("{}...(truncated)", &s[..MAX_OUTPUT_BYTES])
    } else {
        s.to_string()
    }
}
