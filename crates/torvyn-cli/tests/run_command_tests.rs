//! Integration tests for `torvyn run`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_run_missing_manifest() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["run"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_run_missing_manifest_json() {
    let dir = TempDir::new().unwrap();

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "run"])
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
fn test_run_no_flow_defined() {
    let dir = TempDir::new().unwrap();

    // Create a minimal manifest without flows
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
        .args(["run"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("No flow defined"));
}
