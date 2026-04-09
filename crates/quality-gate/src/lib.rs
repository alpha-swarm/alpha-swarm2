use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_name: String,
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainConfig {
    pub build_cmd: Option<String>,
    pub fmt_cmd: Option<String>,
    pub lint_cmd: Option<String>,
    pub unit_test_cmd: Option<String>,
    pub integration_test_cmd: Option<String>,
    pub e2e_test_cmd: Option<String>,
}

/// Auto-detect the toolchain based on files present in the repo.
pub fn detect_toolchain(repo_path: &Path) -> ToolchainConfig {
    if repo_path.join("Cargo.toml").exists() {
        info!("Detected Rust/Cargo toolchain");
        ToolchainConfig {
            build_cmd: Some("cargo check".into()),
            fmt_cmd: Some("cargo fmt -- --check".into()),
            lint_cmd: Some("cargo clippy -- -D warnings".into()),
            unit_test_cmd: Some("cargo test --lib".into()),
            integration_test_cmd: None,
            e2e_test_cmd: None,
        }
    } else if repo_path.join("package.json").exists() {
        info!("Detected Node.js toolchain");
        ToolchainConfig {
            build_cmd: Some("npm run build".into()),
            fmt_cmd: Some("npm run fmt -- --check".into()),
            lint_cmd: Some("npm run lint".into()),
            unit_test_cmd: Some("npm test".into()),
            integration_test_cmd: None,
            e2e_test_cmd: None,
        }
    } else if repo_path.join("go.mod").exists() {
        info!("Detected Go toolchain");
        ToolchainConfig {
            build_cmd: Some("go build ./...".into()),
            fmt_cmd: Some("gofmt -l .".into()),
            lint_cmd: Some("golangci-lint run".into()),
            unit_test_cmd: Some("go test ./...".into()),
            integration_test_cmd: None,
            e2e_test_cmd: None,
        }
    } else {
        warn!("No known toolchain detected");
        ToolchainConfig {
            build_cmd: None,
            fmt_cmd: None,
            lint_cmd: None,
            unit_test_cmd: None,
            integration_test_cmd: None,
            e2e_test_cmd: None,
        }
    }
}

async fn run_check(name: &str, cmd: &str, cwd: &Path) -> Result<CheckResult> {
    info!(check = name, cmd, "Running quality check");
    let start = Instant::now();

    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let (program, args) = parts.split_first()
        .context("Empty command")?;

    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .with_context(|| format!("Failed to execute: {cmd}"))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let exit_code = output.status.code().unwrap_or(-1);
    let passed = output.status.success();

    let result = CheckResult {
        check_name: name.to_string(),
        passed,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code,
        duration_ms,
    };

    if passed {
        info!(check = name, duration_ms, "PASSED");
    } else {
        warn!(check = name, exit_code, duration_ms, "FAILED");
    }

    Ok(result)
}

/// Run all quality checks in order. Stops at first failure.
pub async fn run_all(repo_path: &Path, config: &ToolchainConfig) -> Result<Vec<CheckResult>> {
    let checks: Vec<(&str, &Option<String>)> = vec![
        ("fmt", &config.fmt_cmd),
        ("lint", &config.lint_cmd),
        ("build", &config.build_cmd),
        ("test:unit", &config.unit_test_cmd),
    ];

    let mut results = Vec::new();

    for (name, cmd) in checks {
        let Some(cmd) = cmd else { continue };
        let result = run_check(name, cmd, repo_path).await?;
        let passed = result.passed;
        results.push(result);
        if !passed {
            break;
        }
    }

    Ok(results)
}

/// Run a single named check.
pub async fn run_single(
    name: &str,
    repo_path: &Path,
    config: &ToolchainConfig,
) -> Result<Option<CheckResult>> {
    let cmd = match name {
        "fmt" => &config.fmt_cmd,
        "lint" => &config.lint_cmd,
        "build" => &config.build_cmd,
        "test:unit" => &config.unit_test_cmd,
        "test:integration" => &config.integration_test_cmd,
        "test:e2e" => &config.e2e_test_cmd,
        _ => return Ok(None),
    };

    match cmd {
        Some(cmd) => Ok(Some(run_check(name, cmd, repo_path).await?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detect_rust_toolchain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"t\"").unwrap();
        let config = detect_toolchain(dir.path());
        assert_eq!(config.build_cmd.as_deref(), Some("cargo check"));
        assert_eq!(config.fmt_cmd.as_deref(), Some("cargo fmt -- --check"));
        assert_eq!(config.lint_cmd.as_deref(), Some("cargo clippy -- -D warnings"));
        assert_eq!(config.unit_test_cmd.as_deref(), Some("cargo test --lib"));
    }

    #[test]
    fn detect_node_toolchain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        let config = detect_toolchain(dir.path());
        assert_eq!(config.build_cmd.as_deref(), Some("npm run build"));
    }

    #[test]
    fn detect_go_toolchain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("go.mod"), "module test").unwrap();
        let config = detect_toolchain(dir.path());
        assert_eq!(config.build_cmd.as_deref(), Some("go build ./..."));
    }

    #[test]
    fn detect_unknown_toolchain() {
        let dir = tempfile::tempdir().unwrap();
        let config = detect_toolchain(dir.path());
        assert!(config.build_cmd.is_none());
        assert!(config.fmt_cmd.is_none());
        assert!(config.lint_cmd.is_none());
        assert!(config.unit_test_cmd.is_none());
    }
}
