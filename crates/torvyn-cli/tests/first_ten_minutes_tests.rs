//! End-to-end test for the first-ten-minutes experience.
//!
//! This test exercises: init -> check -> doctor -> link, and the precondition
//! `pack` enforces. Commands that need compiled Wasm (`build`, `pack`, `run`,
//! `inspect`) are covered by `scaffold_end_to_end_tests.rs`, which runs behind
//! the `scaffold-e2e` feature because it needs the component toolchain.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_first_ten_minutes_init_check_link() {
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

    // Step 3: torvyn doctor. Its own toolchain probes must pass on a machine
    // that can build this project — it used to run `--version` as though that
    // were a program name, so every tool it checked reported "NOT found".
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["doctor"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(
            predicate::str::contains("rustc").and(
                predicate::str::contains("rustc NOT found")
                    .not()
                    .and(predicate::str::contains("cargo NOT found").not()),
            ),
        );

    // Step 4: torvyn link — a static topology check over the manifest, so it
    // needs no compiled Wasm. It used to reject every project with
    // "invalid type: map, expected a string in `edges.from`", because it parsed
    // the manifest with a private schema that had drifted from the real one.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["link"])
        .current_dir(&project_dir)
        .assert()
        .success()
        // The counts prove the real manifest was read: the full-pipeline
        // template declares exactly three components joined by two edges.
        .stderr(predicate::str::contains("3 components").and(predicate::str::contains("2 edges")));

    // Step 5: torvyn pack, before anything is built. It must say what is
    // missing and what to do about it.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .failure()
        .stderr(
            // Names every component that is missing, not just the first, and
            // says what to run. `pack` used to write 57 bytes of JSON named
            // `.tar` and report success.
            predicate::str::contains("not been built")
                .and(predicate::str::contains("torvyn build"))
                .and(predicate::str::contains("source"))
                .and(predicate::str::contains("transform"))
                .and(predicate::str::contains("sink")),
        );

    assert!(
        !project_dir.join(".torvyn/artifacts").exists(),
        "a failed pack must not leave artifacts behind"
    );
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
