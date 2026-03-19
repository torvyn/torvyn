//! Integration tests for `torvyn publish`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_publish_no_artifact() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["publish"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("No artifacts found"));
}

#[test]
fn test_publish_dry_run() {
    let dir = TempDir::new().unwrap();

    // Init and pack
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pub-test", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project_dir = dir.path().join("pub-test");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .success();

    // Publish --dry-run
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["publish", "--dry-run"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Dry run"));
}

#[test]
fn test_publish_to_local_directory() {
    let dir = TempDir::new().unwrap();

    // Init and pack
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pub-local", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project_dir = dir.path().join("pub-local");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .success();

    // Publish to local registry
    let registry_dir = dir.path().join("my-registry");
    let registry_arg = format!("local:{}", registry_dir.display());

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["publish", "--registry", &registry_arg])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Published"));

    // Verify artifact was copied to registry
    assert!(registry_dir.exists(), "registry directory should exist");
    let entries: Vec<_> = std::fs::read_dir(&registry_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!entries.is_empty(), "registry should contain the artifact");
}

#[test]
fn test_publish_json_output() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pub-json", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project_dir = dir.path().join("pub-json");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .success();

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "publish", "--dry-run"])
        .current_dir(&project_dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(parsed["success"].as_bool().unwrap());
    assert_eq!(parsed["command"], "publish");
    assert!(parsed["data"]["dry_run"].as_bool().unwrap());
}
