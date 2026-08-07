//! Integration tests for `torvyn publish`.
//!
//! Like the `pack` tests, these relied on `pack` succeeding without a build,
//! which it did only because it wrote a stub. They now build a minimal but
//! genuine component first, and assert what the command reports: the digest
//! must be the artifact's real SHA-256, and a registry it cannot push to must
//! fail rather than report a publish that never happened.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A complete, valid, empty WebAssembly component.
const EMPTY_COMPONENT: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

/// Scaffold a project, place a component binary where `torvyn build` would, and
/// pack it. Returns the project directory.
fn packed_project(dir: &Path, name: &str) -> PathBuf {
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", name, "--template", "transform"])
        .current_dir(dir)
        .assert()
        .success();

    let project_dir = dir.join(name);
    let build_dir = project_dir.join(".torvyn").join("build");
    std::fs::create_dir_all(&build_dir).unwrap();
    // The `transform` template scaffolds an example source and sink alongside
    // the component being built; `pack` writes nothing until all are built.
    for component in declared_components(&project_dir) {
        std::fs::write(build_dir.join(format!("{component}.wasm")), EMPTY_COMPONENT).unwrap();
    }

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .success();

    project_dir
}

/// The `[[component]]` names the project's manifest declares, in order.
fn declared_components(project_dir: &Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(project_dir.join("Torvyn.toml")).expect("read manifest");
    let mut names = Vec::new();
    let mut in_component = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_component = line == "[[component]]";
            continue;
        }
        if in_component {
            if let Some(value) = line.strip_prefix("name") {
                if let Some(name) = value.split('=').nth(1) {
                    names.push(name.trim().trim_matches('"').to_owned());
                    in_component = false;
                }
            }
        }
    }
    names
}

/// Extract the digest from `publish`'s human output, without its prefix.
fn digest_reported(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.split("sha256:").nth(1))
        .map(|rest| rest.trim().to_owned())
}

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
    let project_dir = packed_project(dir.path(), "pub-test");

    let assert = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["publish", "--dry-run"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Dry run"));

    // A dry run reports the digest a real publish would, so it is worth
    // checking against before pushing.
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    let digest = digest_reported(&stderr).expect("dry run reported no digest");
    assert_eq!(digest.len(), 64, "not a sha256 digest: {digest}");

    // And it must not have written anything.
    assert!(
        !project_dir.join(".torvyn").join("registry").exists(),
        "a dry run must not publish"
    );
}

#[test]
fn test_publish_to_local_directory() {
    let dir = TempDir::new().unwrap();
    let project_dir = packed_project(dir.path(), "pub-local");

    let registry_dir = dir.path().join("my-registry");
    let registry_arg = format!("local:{}", registry_dir.display());

    // Name the artifact explicitly: with no `--artifact`, `publish` takes the
    // most recently packed one, which for a multi-component project is
    // whichever was packed last rather than the project's own component.
    let assert = Command::cargo_bin("torvyn")
        .unwrap()
        .args([
            "publish",
            "--artifact",
            ".torvyn/artifacts/pub-local-0.1.0.torvyn",
            "--registry",
            &registry_arg,
        ])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Published"));

    let published = registry_dir.join("pub-local-0.1.0.torvyn");
    assert!(
        published.is_file(),
        "the artifact was not copied into the registry"
    );

    // The digest must be the artifact's content. The previous implementation
    // hashed the artifact's *path* with `DefaultHasher` and labelled the
    // 64-bit result `sha256:`, so it matched nothing and changed when the file
    // moved rather than when the file changed.
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    let reported = digest_reported(&stderr).expect("publish reported no digest");
    let actual = torvyn_packaging::ContentDigest::of_file(&published)
        .expect("digest the published artifact")
        .hex;
    assert_eq!(
        reported, actual,
        "publish reported a digest that is not the artifact's SHA-256"
    );

    // The copy must be byte-identical to what was packed.
    let source = project_dir.join(".torvyn/artifacts/pub-local-0.1.0.torvyn");
    assert_eq!(
        torvyn_packaging::ContentDigest::of_file(&source)
            .unwrap()
            .hex,
        actual,
        "the registry copy does not match the artifact that was published"
    );
}

/// A registry the command cannot push to must fail. It used to report success
/// with the literal digest `sha256:placeholder` while pushing nothing.
#[test]
fn test_publish_rejects_a_remote_registry() {
    let dir = TempDir::new().unwrap();
    let project_dir = packed_project(dir.path(), "pub-remote");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["publish", "--registry", "oci://ghcr.io/example"])
        .current_dir(&project_dir)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not implemented")
                .and(predicate::str::contains("placeholder").not()),
        );
}

#[test]
fn test_publish_json_output() {
    let dir = TempDir::new().unwrap();
    let project_dir = packed_project(dir.path(), "pub-json");

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "publish", "--dry-run"])
        .current_dir(&project_dir)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "publish failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(parsed["success"].as_bool().unwrap());
    assert_eq!(parsed["command"], "publish");
    assert!(parsed["data"]["dry_run"].as_bool().unwrap());

    let digest = parsed["data"]["digest"].as_str().expect("digest");
    assert!(
        digest.starts_with("sha256:") && digest.len() == "sha256:".len() + 64,
        "not a sha256 digest: {digest}"
    );
    assert!(
        parsed["data"]["size_bytes"].as_u64().unwrap_or(0) > 0,
        "publish reported a zero-byte artifact"
    );
}
