//! Integration tests for `torvyn pack`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_pack_missing_manifest() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_pack_creates_artifact() {
    let dir = TempDir::new().unwrap();

    // Init a project first
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pack-test", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project_dir = dir.path().join("pack-test");

    // Pack it
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Packed"));

    // Verify artifact was created
    let artifacts_dir = project_dir.join(".torvyn").join("artifacts");
    assert!(artifacts_dir.exists(), "artifacts directory should exist");

    let artifacts: Vec<_> = std::fs::read_dir(&artifacts_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "tar")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        !artifacts.is_empty(),
        "at least one artifact should be created"
    );
}

#[test]
fn test_pack_json_output() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pack-json", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project_dir = dir.path().join("pack-json");

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "pack"])
        .current_dir(&project_dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(parsed["success"].as_bool().unwrap());
    assert_eq!(parsed["command"], "pack");
    assert!(parsed["data"]["name"].as_str().is_some());
    assert!(parsed["data"]["artifact_path"].as_str().is_some());
}

#[test]
fn test_pack_custom_output_dir() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pack-custom", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project_dir = dir.path().join("pack-custom");
    let custom_output = dir.path().join("my-artifacts");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack", "--output", custom_output.to_str().unwrap()])
        .current_dir(&project_dir)
        .assert()
        .success();

    assert!(custom_output.exists());
    let artifacts: Vec<_> = std::fs::read_dir(&custom_output)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!artifacts.is_empty());
}
