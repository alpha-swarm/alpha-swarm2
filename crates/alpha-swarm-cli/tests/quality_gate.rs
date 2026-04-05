/// Integration tests for quality-gate crate.
/// Creates temporary Rust projects and runs checks.
use std::fs;
use std::process::Command;

#[tokio::test]
async fn run_all_on_clean_rust_project() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test-qg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();

    let config = quality_gate_lib::detect_toolchain(dir.path());
    let results = quality_gate_lib::run_all(dir.path(), &config).await.unwrap();

    // At least fmt should run
    assert!(!results.is_empty());
    assert_eq!(results[0].check_name, "fmt");
}

#[tokio::test]
async fn stops_on_first_failure() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test-qg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    // Bad formatting — tabs instead of spaces
    fs::write(
        dir.path().join("src/main.rs"),
        "fn main(){\n\tprintln!(\"bad format\");\n}\n",
    )
    .unwrap();

    let config = quality_gate_lib::detect_toolchain(dir.path());
    let results = quality_gate_lib::run_all(dir.path(), &config).await.unwrap();

    // fmt should fail, and lint/build/test should NOT run
    let failed = results.iter().find(|r| !r.passed);
    assert!(failed.is_some(), "Expected at least one failure");

    // Should stop early — not all 4 checks
    assert!(results.len() <= 2, "Should stop on first failure, got {} results", results.len());
}

#[tokio::test]
async fn check_result_captures_stderr() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test-qg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    // Syntax error
    fs::write(dir.path().join("src/main.rs"), "fn main() {{{}\n").unwrap();

    let config = quality_gate_lib::detect_toolchain(dir.path());
    let results = quality_gate_lib::run_all(dir.path(), &config).await.unwrap();

    // At least one check should fail with stderr output
    let failed: Vec<_> = results.iter().filter(|r| !r.passed).collect();
    if !failed.is_empty() {
        assert!(
            !failed[0].stderr.is_empty() || !failed[0].stdout.is_empty(),
            "Failed check should have output"
        );
        assert!(failed[0].duration_ms > 0);
    }
}

#[tokio::test]
async fn run_single_check() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test-qg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    let config = quality_gate_lib::detect_toolchain(dir.path());

    let result = quality_gate_lib::run_single("build", dir.path(), &config)
        .await
        .unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().check_name, "build");

    let unknown = quality_gate_lib::run_single("nonexistent", dir.path(), &config)
        .await
        .unwrap();
    assert!(unknown.is_none());
}
