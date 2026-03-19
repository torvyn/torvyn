//! Integration tests for `torvyn bench`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_bench_missing_manifest() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["bench"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_bench_missing_manifest_json() {
    let dir = TempDir::new().unwrap();

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "bench"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
        assert!(parsed["error"].as_bool().unwrap_or(false));
    }
}

#[test]
fn test_bench_no_flow_defined() {
    let dir = TempDir::new().unwrap();

    std::fs::write(
        dir.path().join("Torvyn.toml"),
        r#"
[torvyn]
name = "test-proj"
version = "0.1.0"
contract_version = "0.1.0"

[[component]]
name = "test-proj"
path = "."
language = "rust"
"#,
    )
    .unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["bench"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("No flow defined"));
}
