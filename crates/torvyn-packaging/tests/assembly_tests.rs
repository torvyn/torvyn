//! Integration tests for artifact assembly.

use tempfile::TempDir;
use torvyn_packaging::{
    artifact::{pack, unpack, PackInput},
    provenance::ProvenanceRecord,
};

fn make_test_wasm() -> Vec<u8> {
    let mut wasm = Vec::new();
    wasm.extend_from_slice(b"\0asm");
    wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    // Add some payload to make size realistic
    wasm.extend_from_slice(&[0u8; 1024]);
    wasm
}

fn create_fixture(dir: &std::path::Path) -> PackInput {
    let wasm_path = dir.join("component.wasm");
    std::fs::write(&wasm_path, make_test_wasm()).unwrap();

    let manifest_path = dir.join("Torvyn.toml");
    std::fs::write(
        &manifest_path,
        r#"
[component]
name = "integration-test"
version = "1.0.0"
description = "Integration test component"
license = "MIT"

[contracts]
packages = ["torvyn:streaming@0.1.0"]

[compatibility]
min-torvyn-version = "0.3.0"
wasi-target = "preview2"
target-arch = "wasm32"

[build]
tool = "cargo-component"
tool-version = "0.20.0"
"#,
    )
    .unwrap();

    let wit_dir = dir.join("wit");
    std::fs::create_dir_all(&wit_dir).unwrap();
    std::fs::write(
        wit_dir.join("streaming.wit"),
        "package torvyn:streaming@0.1.0;\n\ninterface types {\n  resource buffer;\n}\n",
    )
    .unwrap();
    std::fs::write(
        wit_dir.join("filtering.wit"),
        "package torvyn:filtering@0.1.0;\n\ninterface filter {\n  filter: func() -> bool;\n}\n",
    )
    .unwrap();

    PackInput {
        wasm_path,
        manifest_path,
        wit_dir,
        provenance: ProvenanceRecord::builder("integration-test", "sha256:placeholder")
            .torvyn_cli_version("0.3.0")
            .build_tool("cargo-component", "0.20.0")
            .source_repo("https://github.com/test/repo")
            .build_timestamps("2026-03-11T10:00:00Z", "2026-03-11T10:01:00Z")
            .build(),
    }
}

#[test]
fn full_pack_unpack_cycle() {
    let dir = TempDir::new().unwrap();
    let input = create_fixture(dir.path());
    let output_dir = dir.path().join("output");

    // Pack
    let pack_result = pack(&input, &output_dir).unwrap();
    assert!(pack_result.artifact_path.exists());

    // Unpack
    let contents = unpack(&pack_result.artifact_path).unwrap();

    // Verify all metadata
    assert_eq!(contents.manifest.name(), "integration-test");
    assert_eq!(contents.manifest.version(), "1.0.0");
    assert_eq!(
        contents.manifest.description(),
        "Integration test component"
    );
    assert_eq!(
        contents.manifest.contract_package_strings(),
        &["torvyn:streaming@0.1.0"]
    );
    assert_eq!(contents.manifest.compatibility.min_torvyn_version, "0.3.0");

    // Verify Wasm binary
    assert!(contents.wasm_bytes.starts_with(b"\0asm"));

    // Verify WIT files
    assert!(contents.wit_files.contains_key("streaming.wit"));
    assert!(contents.wit_files.contains_key("filtering.wit"));

    // Verify provenance
    let prov = contents.provenance.unwrap();
    assert_eq!(prov.subject_name, "integration-test");
    assert_eq!(prov.internal_params.torvyn_cli_version, "0.3.0");
    assert_eq!(
        prov.source.repo.as_deref(),
        Some("https://github.com/test/repo")
    );

    // Verify layer digests match
    assert_eq!(
        pack_result.layer_digests["component.wasm"],
        contents.layer_digests["component.wasm"]
    );
}

#[test]
fn artifact_digest_is_deterministic() {
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();
    let input1 = create_fixture(dir1.path());
    let input2 = create_fixture(dir2.path());
    let out1 = dir1.path().join("out");
    let out2 = dir2.path().join("out");

    let r1 = pack(&input1, &out1).unwrap();
    let r2 = pack(&input2, &out2).unwrap();

    // Layer digests should be identical for identical inputs
    assert_eq!(
        r1.layer_digests["component.wasm"], r2.layer_digests["component.wasm"],
        "wasm layer digests should match for identical inputs"
    );
}
