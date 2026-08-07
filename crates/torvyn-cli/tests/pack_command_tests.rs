//! Integration tests for `torvyn pack`.
//!
//! These used to assert that packing an *unbuilt* project succeeded, which is
//! what the placeholder did: it wrote 57 bytes of JSON named `<component>.tar`
//! and reported "Packed". The tests encoded that as the specification, so
//! nothing failed while `pack` produced files no tool could read.
//!
//! They now supply a minimal but genuine Component Model binary — the eight
//! byte preamble is a complete empty component — so the real packaging path
//! runs without needing `cargo component` or the `wasm32-wasip2` target, and
//! the assertions are about the archive that actually lands on disk.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

/// A complete, valid, empty WebAssembly component: the `\0asm` magic, version
/// `0x000d`, and layer `0x0001` (which is what distinguishes a component from
/// a core module).
const EMPTY_COMPONENT: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

/// Scaffold a project and place a component binary for every component it
/// declares, where `torvyn build` would put them.
///
/// The `transform` template scaffolds an example source and sink alongside the
/// component being built, because a transform on its own has nothing to read
/// from and nowhere to write to. All three must be built before `pack` will
/// write anything.
fn project_with_built_components(dir: &Path, name: &str) -> std::path::PathBuf {
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", name, "--template", "transform"])
        .current_dir(dir)
        .assert()
        .success();

    let project_dir = dir.join(name);
    let build_dir = project_dir.join(".torvyn").join("build");
    std::fs::create_dir_all(&build_dir).unwrap();
    for component in declared_components(&project_dir) {
        std::fs::write(build_dir.join(format!("{component}.wasm")), EMPTY_COMPONENT).unwrap();
    }
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

/// Packing before building must fail, and say what to build. The precondition
/// is the whole reason the old stub looked like it worked.
#[test]
fn test_pack_requires_a_built_component() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "pack-unbuilt", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project_dir = dir.path().join("pack-unbuilt");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not been built")
                .and(predicate::str::contains("torvyn build")),
        );

    assert!(
        !project_dir.join(".torvyn").join("artifacts").exists(),
        "a failed pack must not leave an output directory behind"
    );
}

#[test]
fn test_pack_creates_artifact() {
    let dir = TempDir::new().unwrap();
    let project_dir = project_with_built_components(dir.path(), "pack-test");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("Packed"));

    let artifact = project_dir
        .join(".torvyn")
        .join("artifacts")
        .join("pack-test-0.1.0.torvyn");
    assert!(
        artifact.is_file(),
        "pack should write {}",
        artifact.display()
    );

    // The artifact is a gzip stream, not a JSON stub. `tar -tf` on the old
    // output said "Unrecognized archive format".
    let bytes = std::fs::read(&artifact).unwrap();
    assert_eq!(
        &bytes[..2],
        &[0x1f, 0x8b],
        "artifact is not gzip-compressed"
    );

    // And it round-trips: `inspect` reads back the name and version that were
    // packed into it.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["inspect", artifact.to_str().unwrap()])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("pack-test").and(predicate::str::contains("0.1.0")));
}

#[test]
fn test_pack_json_output() {
    let dir = TempDir::new().unwrap();
    let project_dir = project_with_built_components(dir.path(), "pack-json");

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "pack"])
        .current_dir(&project_dir)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "pack failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(parsed["success"].as_bool().unwrap());
    assert_eq!(parsed["command"], "pack");

    // With no `--component`, every declared component is packed.
    let artifacts = parsed["data"]["artifacts"].as_array().expect("artifacts");
    let packed_names: Vec<&str> = artifacts
        .iter()
        .filter_map(|a| a["name"].as_str())
        .collect();
    assert_eq!(
        packed_names,
        declared_components(&project_dir),
        "pack must produce one artifact per declared component"
    );

    let packed = artifacts
        .iter()
        .find(|a| a["name"] == "pack-json")
        .expect("the project's own component");
    assert_eq!(packed["version"], "0.1.0");
    assert!(packed["artifact_path"].as_str().is_some());

    // Digests must be real SHA-256 values, and must be present for every layer
    // in the archive — that is what makes the artifact content-addressable.
    let digest = packed["digest"].as_str().expect("digest");
    assert!(
        digest.starts_with("sha256:") && digest.len() == "sha256:".len() + 64,
        "not a sha256 digest: {digest}"
    );
    let layers = packed["layers"].as_array().expect("layers");
    assert!(!layers.is_empty(), "an artifact must record its layers");
    for layer in layers {
        let layer_digest = layer["digest"].as_str().expect("layer digest");
        assert!(
            layer_digest.starts_with("sha256:") && layer_digest.len() == "sha256:".len() + 64,
            "layer {} has no real digest: {layer_digest}",
            layer["name"]
        );
    }

    // The component binary and the manifest are what an artifact must carry.
    let names: Vec<&str> = layers.iter().filter_map(|l| l["name"].as_str()).collect();
    assert!(
        names.iter().any(|n| n.ends_with("component.wasm")),
        "artifact does not carry the component binary: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("Torvyn.toml")),
        "artifact does not carry its manifest: {names:?}"
    );
}

#[test]
fn test_pack_custom_output_dir() {
    let dir = TempDir::new().unwrap();
    let project_dir = project_with_built_components(dir.path(), "pack-custom");
    let custom_output = dir.path().join("my-artifacts");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack", "--output", custom_output.to_str().unwrap()])
        .current_dir(&project_dir)
        .assert()
        .success();

    for component in declared_components(&project_dir) {
        assert!(
            custom_output
                .join(format!("{component}-0.1.0.torvyn"))
                .is_file(),
            "--output was not honoured for {component}"
        );
    }
    assert!(
        !project_dir.join(".torvyn").join("artifacts").exists(),
        "--output was honoured but the default directory was created anyway"
    );
}

/// `--tag` names the artifact's version, so packing twice under different tags
/// produces two artifacts rather than overwriting one.
#[test]
fn test_pack_tag_overrides_the_version() {
    let dir = TempDir::new().unwrap();
    let project_dir = project_with_built_components(dir.path(), "pack-tagged");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack", "--tag", "2.0.0-rc.1"])
        .current_dir(&project_dir)
        .assert()
        .success();

    assert!(project_dir
        .join(".torvyn/artifacts/pack-tagged-2.0.0-rc.1.torvyn")
        .is_file());
}

/// `--include-source` asks for something that always happens. It must say so
/// rather than being read as a switch that changes the artifact.
#[test]
fn test_pack_warns_that_include_source_has_no_effect() {
    let dir = TempDir::new().unwrap();
    let project_dir = project_with_built_components(dir.path(), "pack-src");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack", "--include-source"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("--include-source has no effect"));
}
