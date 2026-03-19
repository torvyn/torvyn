//! Integration tests for `torvyn init`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_init_creates_transform_project() {
    let dir = TempDir::new().unwrap();
    let project_dir = dir.path().join("my-transform");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-transform", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Created project"));

    assert!(project_dir.join("Torvyn.toml").exists());
    assert!(project_dir.join("Cargo.toml").exists());
    assert!(project_dir.join("wit/torvyn-streaming/world.wit").exists());
    assert!(project_dir.join("src/lib.rs").exists());
    assert!(project_dir.join(".gitignore").exists());
    assert!(project_dir.join("README.md").exists());

    // Verify Torvyn.toml contains the project name
    let toml_content = std::fs::read_to_string(project_dir.join("Torvyn.toml")).unwrap();
    assert!(toml_content.contains("my-transform"));
}

#[test]
fn test_init_creates_full_pipeline_project() {
    let dir = TempDir::new().unwrap();
    let project_dir = dir.path().join("my-pipeline");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-pipeline", "--template", "full-pipeline"])
        .current_dir(dir.path())
        .assert()
        .success();

    assert!(project_dir.join("Torvyn.toml").exists());
    assert!(project_dir.join("components/source/src/lib.rs").exists());
    assert!(project_dir.join("components/transform/src/lib.rs").exists());
    assert!(project_dir.join("components/sink/src/lib.rs").exists());

    // Verify flow definition exists in manifest
    let toml_content = std::fs::read_to_string(project_dir.join("Torvyn.toml")).unwrap();
    assert!(
        toml_content.contains("[flow.main]"),
        "Full-pipeline template must include a flow definition"
    );
}

#[test]
fn test_init_fails_on_existing_nonempty_dir() {
    let dir = TempDir::new().unwrap();
    let project_dir = dir.path().join("existing");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(project_dir.join("file.txt"), "content").unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "existing"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_init_force_overwrites() {
    let dir = TempDir::new().unwrap();
    let project_dir = dir.path().join("existing");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(project_dir.join("file.txt"), "content").unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "existing", "--force"])
        .current_dir(dir.path())
        .assert()
        .success();

    assert!(project_dir.join("Torvyn.toml").exists());
}

#[test]
fn test_init_no_git() {
    let dir = TempDir::new().unwrap();
    let project_dir = dir.path().join("no-git-proj");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "no-git-proj", "--no-git"])
        .current_dir(dir.path())
        .assert()
        .success();

    // .git directory should NOT exist
    assert!(!project_dir.join(".git").exists());
}

#[test]
fn test_init_json_output() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "init", "json-proj"])
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\""))
        .stdout(predicate::str::contains("json-proj"));
}

#[test]
fn test_init_all_single_templates() {
    for template in ["source", "sink", "transform", "filter", "empty"] {
        let dir = TempDir::new().unwrap();
        let name = format!("proj-{template}");

        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["init", &name, "--template", template])
            .current_dir(dir.path())
            .assert()
            .success();

        assert!(
            dir.path().join(&name).join("Torvyn.toml").exists(),
            "Template {template} did not create Torvyn.toml"
        );
    }
}
