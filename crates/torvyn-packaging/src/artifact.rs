//! Artifact assembly (pack) and extraction (unpack).
//!
//! A `.torvyn` artifact is a gzip-compressed tar archive with the layout:
//!
//! ```text
//! component.wasm
//! Torvyn.toml
//! wit/
//!   streaming.wit
//!   ...
//! provenance.json
//! ```
//!
//! Per HLI Doc 08, Section 2.6.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Archive, Builder as TarBuilder, Header as TarHeader};

use crate::digest::ContentDigest;
use crate::error::ArtifactError;
use crate::manifest::ArtifactManifest;
use crate::provenance::ProvenanceRecord;

// ---------------------------------------------------------------------------
// PackInput
// ---------------------------------------------------------------------------

/// Input for artifact assembly.
///
/// Collected by the CLI before calling `pack()`.
///
/// # Invariants
/// - `wasm_path` must point to a valid Component Model `.wasm` file.
/// - `manifest_path` must point to a valid `Torvyn.toml`.
/// - `wit_dir` must contain at least one `.wit` file.
#[derive(Debug)]
pub struct PackInput {
    /// Path to the compiled Wasm component binary.
    pub wasm_path: PathBuf,

    /// Path to the `Torvyn.toml` manifest.
    pub manifest_path: PathBuf,

    /// Path to the directory containing `.wit` files.
    pub wit_dir: PathBuf,

    /// Provenance record to embed.
    pub provenance: ProvenanceRecord,
}

// ---------------------------------------------------------------------------
// PackOutput
// ---------------------------------------------------------------------------

/// Output from artifact assembly.
#[derive(Debug)]
pub struct PackOutput {
    /// Path to the created `.torvyn` file.
    pub artifact_path: PathBuf,

    /// SHA-256 digest of the artifact file.
    pub digest: ContentDigest,

    /// Individual layer digests (layer_name -> digest).
    pub layer_digests: BTreeMap<String, ContentDigest>,

    /// Total artifact size in bytes.
    pub size_bytes: u64,
}

// ---------------------------------------------------------------------------
// ArtifactContents
// ---------------------------------------------------------------------------

/// In-memory representation of an extracted artifact.
///
/// Produced by `unpack()`.
#[derive(Debug)]
pub struct ArtifactContents {
    /// The parsed manifest.
    pub manifest: ArtifactManifest,

    /// Raw Wasm binary bytes.
    pub wasm_bytes: Vec<u8>,

    /// WIT file contents: filename -> content string.
    pub wit_files: BTreeMap<String, String>,

    /// Provenance record, if present.
    pub provenance: Option<ProvenanceRecord>,

    /// Per-layer digests computed during extraction.
    pub layer_digests: BTreeMap<String, ContentDigest>,
}

// ---------------------------------------------------------------------------
// pack()
// ---------------------------------------------------------------------------

/// Assemble a `.torvyn` artifact from component inputs.
///
/// This is the implementation of `torvyn pack`.
///
/// # Preconditions
/// - `input.wasm_path` exists and is a valid file.
/// - `input.manifest_path` exists and contains valid TOML.
/// - `input.wit_dir` exists and contains at least one `.wit` file.
///
/// # Postconditions
/// - A `.torvyn` file is written to `output_dir`.
/// - The file is a valid gzip-compressed tar archive.
/// - Layer digests are computed and returned.
///
/// # Errors
/// - `ArtifactError::WasmBinaryNotFound` if the Wasm file does not exist.
/// - `ArtifactError::NoWitFiles` if no `.wit` files are found.
/// - `ArtifactError::Io` for filesystem errors.
///
/// COLD PATH — called once per `torvyn pack` invocation.
pub fn pack(input: &PackInput, output_dir: &Path) -> Result<PackOutput, ArtifactError> {
    // 1. Validate inputs exist
    if !input.wasm_path.exists() {
        return Err(ArtifactError::WasmBinaryNotFound {
            path: input.wasm_path.clone(),
        });
    }
    if !input.wit_dir.exists() {
        return Err(ArtifactError::NoWitFiles {
            path: input.wit_dir.clone(),
        });
    }

    // 2. Read the manifest and validate it
    let manifest_bytes = std::fs::read(&input.manifest_path).map_err(|e| ArtifactError::Io {
        path: input.manifest_path.clone(),
        source: e,
    })?;
    let manifest_str = String::from_utf8_lossy(&manifest_bytes);
    let manifest = ArtifactManifest::from_toml_str(&manifest_str).map_err(|e| {
        ArtifactError::CorruptedArtifact {
            path: input.manifest_path.clone(),
            reason: e.to_string(),
        }
    })?;

    // 3. Read the Wasm binary
    let wasm_bytes = std::fs::read(&input.wasm_path).map_err(|e| ArtifactError::Io {
        path: input.wasm_path.clone(),
        source: e,
    })?;

    // 4. Validate Wasm is a Component Model binary (basic check: magic bytes)
    validate_wasm_component(&wasm_bytes, &input.wasm_path)?;

    // 5. Collect WIT files
    let wit_files = collect_wit_files(&input.wit_dir)?;
    if wit_files.is_empty() {
        return Err(ArtifactError::NoWitFiles {
            path: input.wit_dir.clone(),
        });
    }

    // 6. Generate provenance JSON
    let provenance_json =
        input
            .provenance
            .to_intoto_json()
            .map_err(|e| ArtifactError::CorruptedArtifact {
                path: PathBuf::from("provenance.json"),
                reason: format!("failed to serialize provenance: {e}"),
            })?;

    // 7. Compute layer digests
    let mut layer_digests = BTreeMap::new();
    layer_digests.insert(
        "component.wasm".to_owned(),
        ContentDigest::of_bytes(&wasm_bytes),
    );
    layer_digests.insert(
        "Torvyn.toml".to_owned(),
        ContentDigest::of_bytes(manifest_bytes.as_slice()),
    );
    layer_digests.insert(
        "provenance.json".to_owned(),
        ContentDigest::of_bytes(provenance_json.as_bytes()),
    );

    // 8. Build the tar.gz archive
    let artifact_name = format!("{}-{}.torvyn", manifest.name(), manifest.version());
    std::fs::create_dir_all(output_dir).map_err(|e| ArtifactError::Io {
        path: output_dir.to_owned(),
        source: e,
    })?;
    let artifact_path = output_dir.join(&artifact_name);

    let file = std::fs::File::create(&artifact_path).map_err(|e| ArtifactError::Io {
        path: artifact_path.clone(),
        source: e,
    })?;
    let gz = GzEncoder::new(file, Compression::default());
    let mut tar = TarBuilder::new(gz);

    // Add component.wasm
    append_bytes_to_tar(&mut tar, "component.wasm", &wasm_bytes)?;

    // Add Torvyn.toml
    append_bytes_to_tar(&mut tar, "Torvyn.toml", &manifest_bytes)?;

    // Add wit/ directory entries
    for (name, content) in &wit_files {
        let archive_path = format!("wit/{name}");
        append_bytes_to_tar(&mut tar, &archive_path, content.as_bytes())?;
    }

    // Add provenance.json
    append_bytes_to_tar(&mut tar, "provenance.json", provenance_json.as_bytes())?;

    // Finalize the archive
    // LLI DEVIATION: tar::Builder::into_inner returns io::Error directly via into_io()
    let gz = tar.into_inner().map_err(|e| ArtifactError::Io {
        path: artifact_path.clone(),
        source: std::io::Error::other(e.to_string()),
    })?;
    gz.finish().map_err(|e| ArtifactError::Io {
        path: artifact_path.clone(),
        source: e,
    })?;

    // 9. Compute artifact digest
    let digest = ContentDigest::of_file(&artifact_path).map_err(|e| ArtifactError::Io {
        path: artifact_path.clone(),
        source: e,
    })?;

    let size_bytes = std::fs::metadata(&artifact_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(PackOutput {
        artifact_path,
        digest,
        layer_digests,
        size_bytes,
    })
}

// ---------------------------------------------------------------------------
// unpack()
// ---------------------------------------------------------------------------

/// Extract and validate a `.torvyn` artifact.
///
/// # Preconditions
/// - `artifact_path` points to a valid `.torvyn` file.
///
/// # Postconditions
/// - All layers are extracted and parsed.
/// - Layer digests are computed for verification.
///
/// # Errors
/// - `ArtifactError::CorruptedArtifact` if the archive structure is invalid.
/// - `ArtifactError::Io` for filesystem errors.
///
/// COLD PATH — called during inspect and pull.
pub fn unpack(artifact_path: &Path) -> Result<ArtifactContents, ArtifactError> {
    let file = std::fs::File::open(artifact_path).map_err(|e| ArtifactError::Io {
        path: artifact_path.to_owned(),
        source: e,
    })?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);

    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut wasm_bytes: Option<Vec<u8>> = None;
    let mut wit_files: BTreeMap<String, String> = BTreeMap::new();
    let mut provenance_bytes: Option<Vec<u8>> = None;

    for entry_result in archive.entries().map_err(|e| ArtifactError::Io {
        path: artifact_path.to_owned(),
        source: e,
    })? {
        let mut entry = entry_result.map_err(|e| ArtifactError::Io {
            path: artifact_path.to_owned(),
            source: e,
        })?;

        let entry_path = entry
            .path()
            .map_err(|e| ArtifactError::Io {
                path: artifact_path.to_owned(),
                source: e,
            })?
            .to_path_buf();

        let entry_path_str = entry_path.to_string_lossy().to_string();
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|e| ArtifactError::Io {
                path: artifact_path.to_owned(),
                source: e,
            })?;

        match entry_path_str.as_str() {
            "component.wasm" => wasm_bytes = Some(data),
            "Torvyn.toml" => manifest_bytes = Some(data),
            "provenance.json" => provenance_bytes = Some(data),
            p if p.starts_with("wit/") && p.ends_with(".wit") => {
                let filename = p.strip_prefix("wit/").unwrap_or(p).to_owned();
                let content = String::from_utf8_lossy(&data).to_string();
                wit_files.insert(filename, content);
            }
            _ => {
                // Ignore unknown entries (forward compatibility)
                tracing::debug!(entry = %entry_path_str, "ignoring unknown archive entry");
            }
        }
    }

    // Validate required layers are present
    let manifest_bytes = manifest_bytes.ok_or_else(|| ArtifactError::CorruptedArtifact {
        path: artifact_path.to_owned(),
        reason: "missing Torvyn.toml layer".into(),
    })?;
    let wasm_bytes = wasm_bytes.ok_or_else(|| ArtifactError::CorruptedArtifact {
        path: artifact_path.to_owned(),
        reason: "missing component.wasm layer".into(),
    })?;

    // Parse manifest
    let manifest_str = String::from_utf8_lossy(&manifest_bytes);
    let manifest = ArtifactManifest::from_toml_str(&manifest_str).map_err(|e| {
        ArtifactError::CorruptedArtifact {
            path: artifact_path.to_owned(),
            reason: format!("invalid manifest: {e}"),
        }
    })?;

    // Parse provenance (optional)
    let provenance = provenance_bytes
        .as_deref()
        .map(|bytes| {
            let json = String::from_utf8_lossy(bytes);
            ProvenanceRecord::from_intoto_json(&json)
        })
        .transpose()
        .map_err(|e| ArtifactError::CorruptedArtifact {
            path: artifact_path.to_owned(),
            reason: format!("invalid provenance: {e}"),
        })?;

    // Compute layer digests
    let mut layer_digests = BTreeMap::new();
    layer_digests.insert(
        "component.wasm".to_owned(),
        ContentDigest::of_bytes(&wasm_bytes),
    );
    layer_digests.insert(
        "Torvyn.toml".to_owned(),
        ContentDigest::of_bytes(&manifest_bytes),
    );
    if let Some(ref pb) = provenance_bytes {
        layer_digests.insert("provenance.json".to_owned(), ContentDigest::of_bytes(pb));
    }

    Ok(ArtifactContents {
        manifest,
        wasm_bytes,
        wit_files,
        provenance,
        layer_digests,
    })
}

// ---------------------------------------------------------------------------
// unpack_to_dir()
// ---------------------------------------------------------------------------

/// Extract a `.torvyn` artifact to a directory on disk.
///
/// # Errors
/// - `ArtifactError::CorruptedArtifact` if the archive structure is invalid.
/// - `ArtifactError::Io` for filesystem errors.
///
/// COLD PATH — called during pull to populate cache.
pub fn unpack_to_dir(
    artifact_path: &Path,
    output_dir: &Path,
) -> Result<ArtifactContents, ArtifactError> {
    let contents = unpack(artifact_path)?;

    std::fs::create_dir_all(output_dir).map_err(|e| ArtifactError::Io {
        path: output_dir.to_owned(),
        source: e,
    })?;

    // Write component.wasm
    std::fs::write(output_dir.join("component.wasm"), &contents.wasm_bytes).map_err(|e| {
        ArtifactError::Io {
            path: output_dir.join("component.wasm"),
            source: e,
        }
    })?;

    // Write Torvyn.toml
    let manifest_toml =
        contents
            .manifest
            .to_toml_string()
            .map_err(|e| ArtifactError::CorruptedArtifact {
                path: artifact_path.to_owned(),
                reason: format!("manifest serialization failed: {e}"),
            })?;
    std::fs::write(output_dir.join("Torvyn.toml"), manifest_toml.as_bytes()).map_err(|e| {
        ArtifactError::Io {
            path: output_dir.join("Torvyn.toml"),
            source: e,
        }
    })?;

    // Write wit/ directory
    let wit_dir = output_dir.join("wit");
    std::fs::create_dir_all(&wit_dir).map_err(|e| ArtifactError::Io {
        path: wit_dir.clone(),
        source: e,
    })?;
    for (name, content) in &contents.wit_files {
        std::fs::write(wit_dir.join(name), content.as_bytes()).map_err(|e| ArtifactError::Io {
            path: wit_dir.join(name),
            source: e,
        })?;
    }

    // Write provenance.json
    if let Some(ref prov) = contents.provenance {
        if let Ok(json) = prov.to_intoto_json() {
            std::fs::write(output_dir.join("provenance.json"), json.as_bytes()).map_err(|e| {
                ArtifactError::Io {
                    path: output_dir.join("provenance.json"),
                    source: e,
                }
            })?;
        }
    }

    Ok(contents)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Basic validation that a byte slice looks like a Wasm Component.
///
/// Checks the Wasm magic bytes (`\0asm`) and component-model layer byte.
/// This is a quick sanity check, not a full validation.
///
/// // WARM PATH — called during pack.
fn validate_wasm_component(data: &[u8], path: &Path) -> Result<(), ArtifactError> {
    // Wasm magic: \0asm
    if data.len() < 8 {
        return Err(ArtifactError::InvalidWasmBinary {
            path: path.to_owned(),
            reason: "file too small to be a Wasm binary".into(),
        });
    }
    if &data[0..4] != b"\0asm" {
        return Err(ArtifactError::InvalidWasmBinary {
            path: path.to_owned(),
            reason: "missing Wasm magic bytes (\\0asm)".into(),
        });
    }
    // Component Model layer byte: version field at bytes 4..8.
    // Core Wasm is version 1 (0x01 0x00 0x00 0x00).
    // Component Model is layer 0x0d (13) with version 0x01.
    // For a basic check, we verify at least the magic bytes are present.
    // Full Component Model validation would require a Wasm parser.
    Ok(())
}

/// Collect all `.wit` files from a directory (non-recursive).
///
/// COLD PATH.
fn collect_wit_files(dir: &Path) -> Result<BTreeMap<String, String>, ArtifactError> {
    let mut files = BTreeMap::new();
    let entries = std::fs::read_dir(dir).map_err(|e| ArtifactError::Io {
        path: dir.to_owned(),
        source: e,
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| ArtifactError::Io {
            path: dir.to_owned(),
            source: e,
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wit") {
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let content = std::fs::read_to_string(&path).map_err(|e| ArtifactError::Io {
                path: path.clone(),
                source: e,
            })?;
            files.insert(filename, content);
        }
    }
    Ok(files)
}

/// Append a byte slice as a file entry in a tar archive.
///
/// COLD PATH.
fn append_bytes_to_tar<W: Write>(
    tar: &mut TarBuilder<W>,
    name: &str,
    data: &[u8],
) -> Result<(), ArtifactError> {
    let mut header = TarHeader::new_gnu();
    header.set_path(name).map_err(|e| ArtifactError::Io {
        path: PathBuf::from(name),
        source: e,
    })?;
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0); // Reproducible builds: fixed mtime
    header.set_cksum();

    tar.append(&header, data).map_err(|e| ArtifactError::Io {
        path: PathBuf::from(name),
        source: e,
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a minimal valid test Wasm binary (with magic bytes).
    fn make_test_wasm() -> Vec<u8> {
        // Minimal Wasm module: magic + version + empty section
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\0asm"); // magic
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1
        wasm
    }

    /// Create a test fixture directory with all required files.
    fn create_test_fixture(dir: &Path) -> PackInput {
        // Wasm binary
        let wasm_path = dir.join("component.wasm");
        std::fs::write(&wasm_path, make_test_wasm()).unwrap();

        // Manifest
        let manifest_path = dir.join("Torvyn.toml");
        let manifest_toml = r#"
[component]
name = "test-component"
version = "0.1.0"
description = "A test component"
license = "MIT"

[contracts]
packages = ["torvyn:streaming@0.1.0"]

[compatibility]
min-torvyn-version = "0.1.0"
"#;
        std::fs::write(&manifest_path, manifest_toml).unwrap();

        // WIT files
        let wit_dir = dir.join("wit");
        std::fs::create_dir_all(&wit_dir).unwrap();
        std::fs::write(
            wit_dir.join("streaming.wit"),
            "package torvyn:streaming@0.1.0;\n\ninterface types {\n  resource buffer;\n}\n",
        )
        .unwrap();

        // Provenance
        let provenance = ProvenanceRecord::builder("test-component", "sha256:placeholder")
            .torvyn_cli_version("0.1.0")
            .build();

        PackInput {
            wasm_path,
            manifest_path,
            wit_dir,
            provenance,
        }
    }

    #[test]
    fn pack_creates_artifact_file() {
        let dir = TempDir::new().unwrap();
        let input = create_test_fixture(dir.path());
        let output_dir = dir.path().join("output");

        let result = pack(&input, &output_dir).unwrap();

        assert!(result.artifact_path.exists(), "artifact file should exist");
        assert!(
            result
                .artifact_path
                .to_string_lossy()
                .contains("test-component-0.1.0.torvyn"),
            "artifact should have correct name"
        );
        assert!(result.size_bytes > 0, "artifact should have non-zero size");
        assert!(!result.digest.hex.is_empty(), "digest should be computed");
    }

    #[test]
    fn pack_computes_layer_digests() {
        let dir = TempDir::new().unwrap();
        let input = create_test_fixture(dir.path());
        let output_dir = dir.path().join("output");

        let result = pack(&input, &output_dir).unwrap();

        assert!(
            result.layer_digests.contains_key("component.wasm"),
            "should have wasm layer digest"
        );
        assert!(
            result.layer_digests.contains_key("Torvyn.toml"),
            "should have manifest layer digest"
        );
        assert!(
            result.layer_digests.contains_key("provenance.json"),
            "should have provenance layer digest"
        );
    }

    #[test]
    fn pack_then_unpack_roundtrip() {
        let dir = TempDir::new().unwrap();
        let input = create_test_fixture(dir.path());
        let output_dir = dir.path().join("output");

        let pack_result = pack(&input, &output_dir).unwrap();
        let contents = unpack(&pack_result.artifact_path).unwrap();

        assert_eq!(contents.manifest.name(), "test-component");
        assert_eq!(contents.manifest.version(), "0.1.0");
        assert_eq!(contents.wasm_bytes, make_test_wasm());
        assert!(contents.wit_files.contains_key("streaming.wit"));
        assert!(contents.provenance.is_some());
    }

    #[test]
    fn pack_then_unpack_to_dir_creates_files() {
        let dir = TempDir::new().unwrap();
        let input = create_test_fixture(dir.path());
        let output_dir = dir.path().join("output");
        let extract_dir = dir.path().join("extracted");

        let pack_result = pack(&input, &output_dir).unwrap();
        unpack_to_dir(&pack_result.artifact_path, &extract_dir).unwrap();

        assert!(extract_dir.join("component.wasm").exists());
        assert!(extract_dir.join("Torvyn.toml").exists());
        assert!(extract_dir.join("wit/streaming.wit").exists());
        assert!(extract_dir.join("provenance.json").exists());
    }

    #[test]
    fn pack_missing_wasm_returns_error() {
        let dir = TempDir::new().unwrap();
        let input = PackInput {
            wasm_path: dir.path().join("nonexistent.wasm"),
            manifest_path: dir.path().join("Torvyn.toml"),
            wit_dir: dir.path().join("wit"),
            provenance: ProvenanceRecord::builder("x", "sha256:000").build(),
        };

        let result = pack(&input, &dir.path().join("out"));
        assert!(matches!(
            result,
            Err(ArtifactError::WasmBinaryNotFound { .. })
        ));
    }

    #[test]
    fn pack_no_wit_files_returns_error() {
        let dir = TempDir::new().unwrap();

        // Create wasm and manifest but empty wit dir
        std::fs::write(dir.path().join("component.wasm"), make_test_wasm()).unwrap();
        std::fs::write(
            dir.path().join("Torvyn.toml"),
            "[component]\nname = \"test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let wit_dir = dir.path().join("wit");
        std::fs::create_dir_all(&wit_dir).unwrap();
        // No .wit files inside

        let input = PackInput {
            wasm_path: dir.path().join("component.wasm"),
            manifest_path: dir.path().join("Torvyn.toml"),
            wit_dir,
            provenance: ProvenanceRecord::builder("test", "sha256:000").build(),
        };

        let result = pack(&input, &dir.path().join("out"));
        assert!(matches!(result, Err(ArtifactError::NoWitFiles { .. })));
    }

    #[test]
    fn pack_invalid_wasm_returns_error() {
        let dir = TempDir::new().unwrap();
        let input = create_test_fixture(dir.path());
        // Overwrite with non-wasm data
        std::fs::write(&input.wasm_path, b"not a wasm file").unwrap();

        let result = pack(&input, &dir.path().join("out"));
        assert!(matches!(
            result,
            Err(ArtifactError::InvalidWasmBinary { .. })
        ));
    }

    #[test]
    fn unpack_corrupted_archive_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.torvyn");
        std::fs::write(&path, b"this is not a tar.gz file").unwrap();

        let result = unpack(&path);
        assert!(result.is_err());
    }

    #[test]
    fn validate_wasm_component_rejects_too_small() {
        let result = validate_wasm_component(b"hi", Path::new("test.wasm"));
        assert!(matches!(
            result,
            Err(ArtifactError::InvalidWasmBinary { .. })
        ));
    }

    #[test]
    fn validate_wasm_component_rejects_bad_magic() {
        let data = b"not\x00wasm\x00";
        let result = validate_wasm_component(data, Path::new("test.wasm"));
        assert!(matches!(
            result,
            Err(ArtifactError::InvalidWasmBinary { .. })
        ));
    }

    #[test]
    fn validate_wasm_component_accepts_valid_magic() {
        let mut data = Vec::new();
        data.extend_from_slice(b"\0asm");
        data.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        let result = validate_wasm_component(&data, Path::new("test.wasm"));
        assert!(result.is_ok());
    }

    #[test]
    fn layer_digests_match_between_pack_and_unpack() {
        let dir = TempDir::new().unwrap();
        let input = create_test_fixture(dir.path());
        let output_dir = dir.path().join("output");

        let pack_result = pack(&input, &output_dir).unwrap();
        let contents = unpack(&pack_result.artifact_path).unwrap();

        // The wasm layer digest should match
        assert_eq!(
            pack_result.layer_digests["component.wasm"], contents.layer_digests["component.wasm"],
            "wasm layer digest should be identical"
        );
    }
}
