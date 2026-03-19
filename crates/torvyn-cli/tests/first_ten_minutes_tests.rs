//! End-to-end test for the first-ten-minutes experience.
//!
//! This test exercises: init -> check -> doctor -> pack.
//! Full pipeline execution (run, bench, trace) requires compiled components
//! and is tested separately with pre-built fixtures.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_first_ten_minutes_init_check_pack() {
    let workspace = TempDir::new().unwrap();

    // Step 1: torvyn init
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-first-pipeline", "--template", "full-pipeline"])
        .current_dir(workspace.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Created project"));

    let project_dir = workspace.path().join("my-first-pipeline");
    assert!(project_dir.join("Torvyn.toml").exists());
    assert!(project_dir.join("components/source/src/lib.rs").exists());
    assert!(project_dir.join("components/transform/src/lib.rs").exists());

    // Step 2: torvyn check
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project_dir)
        .assert()
        .success();

    // Step 3: torvyn doctor
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["doctor"])
        .current_dir(&project_dir)
        .assert()
        .success();

    // Step 4: torvyn pack
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Packed"));

    // Verify artifact
    let artifacts_dir = project_dir.join(".torvyn").join("artifacts");
    assert!(artifacts_dir.exists());
    let artifacts: Vec<_> = std::fs::read_dir(&artifacts_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!artifacts.is_empty(), "pack should create an artifact");
}

#[test]
fn test_first_ten_minutes_json_mode() {
    let workspace = TempDir::new().unwrap();

    // Init with JSON output
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args([
            "--format",
            "json",
            "init",
            "json-pipeline",
            "--template",
            "transform",
        ])
        .current_dir(workspace.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(json_str.trim()).unwrap();
    assert_eq!(parsed["data"]["project_name"], "json-pipeline");
    assert!(parsed["success"].as_bool().unwrap());
}
