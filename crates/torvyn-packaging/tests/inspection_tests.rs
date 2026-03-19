//! Integration tests for artifact inspection.

use tempfile::TempDir;
use torvyn_packaging::{
    artifact::{pack, PackInput},
    inspection::{format_inspection, inspect},
    provenance::ProvenanceRecord,
};

fn make_test_wasm() -> Vec<u8> {
    let mut wasm = Vec::new();
    wasm.extend_from_slice(b"\0asm");
    wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    wasm
}

fn create_and_pack(dir: &std::path::Path) -> std::path::PathBuf {
    let wasm_path = dir.join("component.wasm");
    std::fs::write(&wasm_path, make_test_wasm()).unwrap();

    let manifest_path = dir.join("Torvyn.toml");
    std::fs::write(
        &manifest_path,
        r#"
[component]
name = "inspect-test"
version = "3.1.4"
description = "A component for inspection testing"
license = "Apache-2.0"

[contracts]
packages = ["torvyn:streaming@0.1.0"]

[compatibility]
min-torvyn-version = "0.5.0"
wasi-target = "preview2"

[build]
tool = "cargo-component"
tool-version = "0.25.0"

[deprecation]
deprecated-since = "3.0.0"
message = "Use v4.x with the new API"
successor = "inspect-test-v4"
"#,
    )
    .unwrap();

    let wit_dir = dir.join("wit");
    std::fs::create_dir_all(&wit_dir).unwrap();
    std::fs::write(wit_dir.join("types.wit"), "package torvyn:streaming;\n").unwrap();

    let input = PackInput {
        wasm_path,
        manifest_path,
        wit_dir,
        provenance: ProvenanceRecord::builder("inspect-test", "sha256:aaa")
            .torvyn_cli_version("0.5.0")
            .build(),
    };

    let output_dir = dir.join("output");
    pack(&input, &output_dir).unwrap().artifact_path
}

#[test]
fn inspect_shows_all_metadata() {
    let dir = TempDir::new().unwrap();
    let artifact_path = create_and_pack(dir.path());

    let result = inspect(&artifact_path).unwrap();

    assert_eq!(result.name, "inspect-test");
    assert_eq!(result.version, "3.1.4");
    assert_eq!(result.description, "A component for inspection testing");
    assert_eq!(result.license, "Apache-2.0");
    assert_eq!(result.min_torvyn_version, "0.5.0");
    assert_eq!(result.build_tool, "cargo-component 0.25.0");
}

#[test]
fn inspect_shows_deprecation() {
    let dir = TempDir::new().unwrap();
    let artifact_path = create_and_pack(dir.path());

    let result = inspect(&artifact_path).unwrap();

    let dep = result.deprecation_message.unwrap();
    assert!(dep.contains("Use v4.x"));
    assert!(dep.contains("inspect-test-v4"));
}

#[test]
fn format_inspection_is_readable() {
    let dir = TempDir::new().unwrap();
    let artifact_path = create_and_pack(dir.path());

    let result = inspect(&artifact_path).unwrap();
    let formatted = format_inspection(&result);

    assert!(formatted.contains("inspect-test v3.1.4"));
    assert!(formatted.contains("DEPRECATED"));
}
