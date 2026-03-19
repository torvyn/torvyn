//! Integration tests for `torvyn check`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn create_valid_project(dir: &std::path::Path) -> std::path::PathBuf {
    // Create a minimal valid project (without wit/ dir to avoid
    // NullWitParser limitations — produces a warning, not an error)
    let project = dir.join("valid-proj");
    std::fs::create_dir_all(project.join("src")).unwrap();

    // Use the minimal manifest format that torvyn-config accepts
    std::fs::write(
        project.join("Torvyn.toml"),
        "[torvyn]\nname = \"valid-proj\"\nversion = \"0.1.0\"\ncontract_version = \"0.1.0\"\n",
    )
    .unwrap();

    project
}

#[test]
fn test_check_valid_project() {
    let dir = TempDir::new().unwrap();
    let project = create_valid_project(dir.path());

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .success();
}

#[test]
fn test_check_missing_manifest() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Manifest not found"));
}

#[test]
fn test_check_json_output() {
    let dir = TempDir::new().unwrap();
    let project = create_valid_project(dir.path());

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "check"])
        .current_dir(&project)
        .assert()
        .success();
}
