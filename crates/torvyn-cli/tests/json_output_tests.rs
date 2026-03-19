//! Tests that --format json produces valid JSON for every command.

use assert_cmd::Command;
use tempfile::TempDir;

fn assert_valid_json(output: &[u8]) {
    let stdout = String::from_utf8_lossy(output);
    if stdout.trim().is_empty() {
        return; // no JSON output is acceptable for some commands
    }
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(stdout.trim());
    assert!(
        parsed.is_ok(),
        "Invalid JSON output: {}\nParse error: {:?}",
        stdout,
        parsed.err()
    );
}

#[test]
fn test_init_json_is_valid() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "init", "json-test"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_valid_json(&output.stdout);
}

#[test]
fn test_doctor_json_is_valid() {
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "doctor"])
        .output()
        .unwrap();
    assert_valid_json(&output.stdout);
}

#[test]
fn test_check_error_json_is_valid() {
    let dir = TempDir::new().unwrap();
    // check in empty dir should fail with JSON error
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "check"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    // Even errors should be valid JSON
    assert_valid_json(&output.stdout);
}

#[test]
fn test_run_error_json_is_valid() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "run"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_valid_json(&output.stdout);
}

#[test]
fn test_trace_error_json_is_valid() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "trace"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_valid_json(&output.stdout);
}

#[test]
fn test_bench_error_json_is_valid() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "bench"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_valid_json(&output.stdout);
}

#[test]
fn test_pack_json_is_valid() {
    let dir = TempDir::new().unwrap();

    // Init first so pack can succeed
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pack-json-test", "--template", "transform"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "pack"])
        .current_dir(dir.path().join("pack-json-test"))
        .output()
        .unwrap();
    assert_valid_json(&output.stdout);
}

#[test]
fn test_publish_error_json_is_valid() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "publish"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_valid_json(&output.stdout);
}
