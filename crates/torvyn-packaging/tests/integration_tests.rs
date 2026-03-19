//! End-to-end integration tests: pack -> inspect -> cache -> resolve.

use tempfile::TempDir;
use torvyn_packaging::{
    artifact::{pack, PackInput},
    cache::{ArtifactCache, CacheConfig},
    inspection::inspect,
    oci::OciReference,
    provenance::ProvenanceRecord,
    resolution::{resolve, ResolutionSource},
};

fn make_test_wasm() -> Vec<u8> {
    let mut wasm = Vec::new();
    wasm.extend_from_slice(b"\0asm");
    wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
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
name = "e2e-component"
version = "1.0.0"

[contracts]
packages = ["torvyn:streaming@0.1.0"]

[compatibility]
min-torvyn-version = "0.1.0"
"#,
    )
    .unwrap();

    let wit_dir = dir.join("wit");
    std::fs::create_dir_all(&wit_dir).unwrap();
    std::fs::write(wit_dir.join("types.wit"), "package torvyn:streaming;\n").unwrap();

    PackInput {
        wasm_path,
        manifest_path,
        wit_dir,
        provenance: ProvenanceRecord::builder("e2e-component", "sha256:e2e")
            .torvyn_cli_version("0.1.0")
            .build(),
    }
}

#[test]
fn pack_inspect_verify_metadata_matches() {
    let dir = TempDir::new().unwrap();
    let input = create_fixture(dir.path());
    let output_dir = dir.path().join("output");

    let pack_result = pack(&input, &output_dir).unwrap();
    let inspection = inspect(&pack_result.artifact_path).unwrap();

    assert_eq!(inspection.name, "e2e-component");
    assert_eq!(inspection.version, "1.0.0");
    assert_eq!(inspection.contract_packages, vec!["torvyn:streaming@0.1.0"]);
    assert_eq!(inspection.min_torvyn_version, "0.1.0");
}

#[test]
fn pack_cache_resolve_cycle() {
    let dir = TempDir::new().unwrap();
    let input = create_fixture(dir.path());
    let output_dir = dir.path().join("output");

    // Pack
    let pack_result = pack(&input, &output_dir).unwrap();

    // Unpack and cache
    let contents = torvyn_packaging::artifact::unpack(&pack_result.artifact_path).unwrap();
    let cache = ArtifactCache::new(CacheConfig {
        root: dir.path().join("test-cache"),
    });
    let reference = OciReference::parse("test.io/org/e2e-component:1.0.0").unwrap();
    cache
        .store(&reference, &contents, &pack_result.digest)
        .unwrap();

    // Resolve from cache
    let resolved = resolve("test.io/org/e2e-component:1.0.0", None, &cache).unwrap();
    assert_eq!(resolved.source, ResolutionSource::Cache);
    assert!(resolved.path.join("Torvyn.toml").exists());
}

#[test]
fn resolve_file_path_has_highest_precedence() {
    let dir = TempDir::new().unwrap();
    let input = create_fixture(dir.path());
    let output_dir = dir.path().join("output");

    let pack_result = pack(&input, &output_dir).unwrap();
    let cache = ArtifactCache::new(CacheConfig {
        root: dir.path().join("test-cache"),
    });

    let file_ref = format!("file://{}", pack_result.artifact_path.display());
    let resolved = resolve(&file_ref, None, &cache).unwrap();
    assert_eq!(resolved.source, ResolutionSource::FilePath);
}
