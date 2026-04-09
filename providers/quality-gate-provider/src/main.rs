//! Quality gate provider: NATS service for running build/test/lint checks.
//!
//! Runs cargo commands on a workspace directory and reports pass/fail.
//!
//! Subjects:
//!   swarm.quality.check_all — run all checks (fmt, lint, build, test)
//!   swarm.quality.check     — run a single check

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Deserialize)]
struct CheckAllRequest {
    workspace_path: String,
}

#[derive(Deserialize)]
struct CheckRequest {
    workspace_path: String,
    check: String, // "fmt", "lint", "build", "test"
}

#[derive(Serialize, Clone)]
struct CheckResult {
    check_name: String,
    passed: bool,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
}

#[derive(Serialize)]
struct CheckAllResponse {
    results: Vec<CheckResult>,
    all_passed: bool,
}

fn detect_checks(workspace: &Path) -> Vec<(&'static str, &'static str)> {
    if workspace.join("Cargo.toml").exists() {
        vec![
            ("fmt", "cargo fmt -- --check"),
            ("lint", "cargo clippy"),
            ("build", "cargo check"),
            ("test", "cargo test --lib"),
        ]
    } else if workspace.join("package.json").exists() {
        vec![
            ("lint", "npm run lint"),
            ("build", "npm run build"),
            ("test", "npm test"),
        ]
    } else {
        vec![]
    }
}

fn run_check(workspace: &Path, name: &str, cmd: &str) -> CheckResult {
    let start = Instant::now();
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let (program, args) = parts.split_first().unwrap_or((&"echo", &[]));

    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(workspace)
        .output();

    let duration_ms = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let passed = out.status.success();
            let status = if passed { "PASS" } else { "FAIL" };
            info!(check = name, status, duration_ms, "Quality check");
            CheckResult {
                check_name: name.to_string(),
                passed,
                duration_ms,
                output: if passed { None } else {
                    Some(String::from_utf8_lossy(&out.stderr).chars().take(500).collect())
                },
            }
        }
        Err(e) => {
            warn!(check = name, error = %e, "Check failed to run");
            CheckResult {
                check_name: name.to_string(),
                passed: false,
                duration_ms,
                output: Some(e.to_string()),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let nats_url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://127.0.0.1:4223".into());

    let client = async_nats::connect(&nats_url).await?;
    info!("Quality gate provider connected to NATS");

    let mut all_sub = client.subscribe("swarm.quality.check_all").await?;
    let mut single_sub = client.subscribe("swarm.quality.check").await?;

    info!("Listening on swarm.quality.{{check_all,check}}");

    loop {
        tokio::select! {
            Some(msg) = all_sub.next() => {
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<CheckAllRequest>(&msg.payload) {
                        Ok(req) => {
                            let ws = Path::new(&req.workspace_path);

                            // Auto-format before checks
                            if ws.join("Cargo.toml").exists() {
                                let _ = std::process::Command::new("cargo")
                                    .args(["fmt"])
                                    .current_dir(ws)
                                    .output();
                            }

                            let checks = detect_checks(ws);
                            let results: Vec<CheckResult> = checks.iter()
                                .map(|(name, cmd)| run_check(ws, name, cmd))
                                .collect();
                            let all_passed = results.iter().all(|r| r.passed);
                            serde_json::to_vec(&CheckAllResponse { results, all_passed }).unwrap_or_default()
                        }
                        Err(e) => serde_json::to_vec(&CheckAllResponse {
                            results: vec![],
                            all_passed: false,
                        }).unwrap_or_default(),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, resp.into()).await;
                    }
                });
            }
            Some(msg) = single_sub.next() => {
                let client = client.clone();
                tokio::spawn(async move {
                    let resp = match serde_json::from_slice::<CheckRequest>(&msg.payload) {
                        Ok(req) => {
                            let ws = Path::new(&req.workspace_path);
                            let checks = detect_checks(ws);
                            let result = checks.iter()
                                .find(|(name, _)| *name == req.check)
                                .map(|(name, cmd)| run_check(ws, name, cmd))
                                .unwrap_or(CheckResult {
                                    check_name: req.check,
                                    passed: false,
                                    duration_ms: 0,
                                    output: Some("Unknown check".into()),
                                });
                            serde_json::to_vec(&result).unwrap_or_default()
                        }
                        Err(e) => serde_json::to_vec(&CheckResult {
                            check_name: "error".into(),
                            passed: false,
                            duration_ms: 0,
                            output: Some(e.to_string()),
                        }).unwrap_or_default(),
                    };
                    if let Some(reply) = msg.reply {
                        let _ = client.publish(reply, resp.into()).await;
                    }
                });
            }
        }
    }
}
