//! End-to-end test for the path a new user actually takes.
//!
//! `torvyn init` prints three next steps. This runs all of them, on a project
//! it scaffolds from scratch, and asserts the pipeline produces the output the
//! template promises.
//!
//! It exists because every one of those steps used to fail. `torvyn build` was
//! advertised in seventeen places and did not exist; a flow node naming a
//! component never resolved to an artifact, so `run` reported "unsupported
//! component reference scheme" naming the component as though it were a
//! malformed URI; the scaffolded WIT had drifted from the contract crate until
//! its `data-source` world no longer exported `lifecycle`, which made the host
//! refuse every generated source. `first_ten_minutes_tests.rs` covers
//! init → check → doctor → pack and stops immediately before the steps that
//! were broken.
//!
//! Requires the component toolchain (`cargo component`, `wasm32-wasip2`),
//! which is why it is behind the `scaffold-e2e` feature rather than in the
//! default test set.

#![cfg(feature = "scaffold-e2e")]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Building three Wasm components from a cold cargo cache is slow.
const BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);

#[test]
fn scaffolded_pipeline_builds_and_runs() {
    let workspace = TempDir::new().expect("temp workspace");

    // 1. Scaffold, exactly as the README's quick start does.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-pipeline", "--template", "full-pipeline"])
        .current_dir(workspace.path())
        .assert()
        .success();

    let project = workspace.path().join("my-pipeline");

    // 2. `torvyn check` must actually read the project's contracts. It used to
    //    look only at `<project>/wit`, miss the per-component directories a
    //    multi-component project uses, and report "0 file(s)" as a pass.
    let check = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .success();
    let check_output = String::from_utf8_lossy(&check.get_output().stderr).into_owned();
    let parsed = wit_files_reported(&check_output)
        .unwrap_or_else(|| panic!("check did not report a WIT file count:\n{check_output}"));
    assert!(
        parsed > 0,
        "check reported {parsed} WIT files for a project that ships contracts for three \
         components:\n{check_output}"
    );

    // 3. `torvyn build` — the command `init` tells the user to run.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["build"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        .success();

    // Artifacts land where `run` looks for them. This is the contract between
    // the two commands; if it drifts, the pipeline stops resolving.
    for component in ["source", "transform", "sink"] {
        let artifact = project
            .join(".torvyn/build")
            .join(format!("{component}.wasm"));
        assert!(
            artifact.is_file(),
            "torvyn build did not produce {}",
            artifact.display()
        );
    }

    // 4. `torvyn run` — resolves each node's component name to its artifact,
    //    instantiates all three, and drives the pipeline to completion.
    let run = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["run"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&run.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.get_output().stderr).into_owned();

    // The template's transform uppercases what the source emits, and the sink
    // prints it — which also proves the sink's `stdio:stdout` capability grant
    // reached the sandbox. A component that prints without the grant produces
    // no output at all.
    assert!(
        stdout.contains("HELLO, TORVYN!"),
        "the pipeline produced no transformed output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // And the run must be clean: a pipeline that errors on every element still
    // "succeeds" as a process.
    assert!(
        stderr.contains("Errors:  0") || stderr.contains("Errors: 0"),
        "the run reported errors.\nstderr:\n{stderr}"
    );
}

/// `torvyn run` must refuse an option it cannot honour rather than accepting
/// it and behaving as though it were never typed.
#[test]
fn run_rejects_options_it_cannot_honour() {
    let workspace = TempDir::new().expect("temp workspace");
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-pipeline", "--template", "full-pipeline"])
        .current_dir(workspace.path())
        .assert()
        .success();
    let project = workspace.path().join("my-pipeline");

    for option in ["--limit", "--input", "--output"] {
        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["run", option, "10"])
            .current_dir(&project)
            .assert()
            .failure()
            .stderr(predicate::str::contains("is not implemented"));
    }
}

/// Extract the WIT file count from `check`'s human output.
fn wit_files_reported(output: &str) -> Option<usize> {
    let line = output
        .lines()
        .find(|line| line.contains("WIT contracts parsed"))?;
    let start = line.find('(')? + 1;
    let rest = &line[start..];
    let end = rest.find(' ')?;
    rest[..end].parse().ok()
}

#[cfg(test)]
mod unit {
    use super::wit_files_reported;

    #[test]
    fn parses_the_reported_file_count() {
        assert_eq!(
            wit_files_reported("[ok] WIT contracts parsed (21 file(s), 0 errors)"),
            Some(21)
        );
        assert_eq!(
            wit_files_reported("[ok] WIT contracts parsed (0 file(s), 0 errors)"),
            Some(0)
        );
        assert_eq!(wit_files_reported("[ok] Torvyn.toml is valid"), None);
    }
}
