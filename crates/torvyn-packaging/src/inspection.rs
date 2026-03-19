//! Artifact metadata inspection.
//!
//! Supports inspecting a `.torvyn` file to display: component name and version,
//! contract packages, capabilities, compatibility, provenance, and WIT interfaces.

use std::path::Path;

use crate::artifact;
use crate::error::ArtifactError;

// ---------------------------------------------------------------------------
// InspectionResult
// ---------------------------------------------------------------------------

/// The result of inspecting an artifact.
///
/// Contains all metadata that `torvyn inspect` would display.
#[derive(Clone, Debug)]
pub struct InspectionResult {
    /// Component name.
    pub name: String,

    /// Component version (semver).
    pub version: String,

    /// Human-readable description.
    pub description: String,

    /// SPDX license identifier.
    pub license: String,

    /// Contract packages (e.g., "torvyn:streaming@0.1.0").
    pub contract_packages: Vec<String>,

    /// Required capabilities.
    pub capabilities_required: Vec<String>,

    /// Optional capabilities.
    pub capabilities_optional: Vec<String>,

    /// Minimum Torvyn runtime version.
    pub min_torvyn_version: String,

    /// WASI target.
    pub wasi_target: String,

    /// Target architecture.
    pub target_arch: String,

    /// Build tool and version.
    pub build_tool: String,

    /// Wasm binary size in bytes.
    pub wasm_size_bytes: usize,

    /// List of WIT filenames included.
    pub wit_files: Vec<String>,

    /// Provenance summary, if present.
    pub provenance_summary: Option<ProvenanceSummary>,

    /// Deprecation notice, if present.
    pub deprecation_message: Option<String>,
}

/// Summary of provenance for display.
#[derive(Clone, Debug)]
pub struct ProvenanceSummary {
    /// Builder identifier.
    pub builder_id: String,
    /// Build start timestamp.
    pub build_started: String,
    /// Build finish timestamp.
    pub build_finished: String,
    /// Source repository URL.
    pub source_repo: Option<String>,
    /// Torvyn CLI version used.
    pub torvyn_cli_version: String,
}

// ---------------------------------------------------------------------------
// inspect()
// ---------------------------------------------------------------------------

/// Inspect a `.torvyn` artifact and return structured metadata.
///
/// # Preconditions
/// - `artifact_path` points to a valid `.torvyn` file.
///
/// # Errors
/// - `ArtifactError` if the artifact cannot be opened or is corrupted.
///
/// COLD PATH — called by `torvyn inspect`.
pub fn inspect(artifact_path: &Path) -> Result<InspectionResult, ArtifactError> {
    let contents = artifact::unpack(artifact_path)?;
    Ok(inspection_result_from_contents(&contents))
}

/// Build an `InspectionResult` from in-memory artifact contents.
///
/// Useful when contents are already available (e.g., after unpack).
///
/// COLD PATH.
pub fn inspection_result_from_contents(contents: &artifact::ArtifactContents) -> InspectionResult {
    let m = &contents.manifest;

    let provenance_summary = contents.provenance.as_ref().map(|p| ProvenanceSummary {
        builder_id: p.builder.id.clone(),
        build_started: p.build_started.clone(),
        build_finished: p.build_finished.clone(),
        source_repo: p.source.repo.clone(),
        torvyn_cli_version: p.internal_params.torvyn_cli_version.clone(),
    });

    let deprecation_message = m.deprecation.as_ref().map(|d| {
        format!(
            "Deprecated since {}: {}{}",
            d.deprecated_since,
            d.message,
            d.successor
                .as_ref()
                .map(|s| format!(" Successor: {s}"))
                .unwrap_or_default()
        )
    });

    let build_tool = if m.build_info.tool.is_empty() {
        "unknown".to_owned()
    } else {
        format!("{} {}", m.build_info.tool, m.build_info.tool_version)
    };

    InspectionResult {
        name: m.name().to_owned(),
        version: m.version().to_owned(),
        description: m.description().to_owned(),
        license: m.license().to_owned(),
        contract_packages: m.contract_package_strings().to_vec(),
        capabilities_required: m.capabilities.required.keys().cloned().collect(),
        capabilities_optional: m.capabilities.optional.keys().cloned().collect(),
        min_torvyn_version: m.compatibility.min_torvyn_version.clone(),
        wasi_target: m.compatibility.wasi_target.clone(),
        target_arch: m.compatibility.target_arch.clone(),
        build_tool,
        wasm_size_bytes: contents.wasm_bytes.len(),
        wit_files: contents.wit_files.keys().cloned().collect(),
        provenance_summary,
        deprecation_message,
    }
}

/// Format an `InspectionResult` as a human-readable string.
///
/// COLD PATH — used by CLI for terminal output.
pub fn format_inspection(result: &InspectionResult) -> String {
    let mut out = String::new();

    out.push_str(&format!("Component: {} v{}\n", result.name, result.version));
    if !result.description.is_empty() {
        out.push_str(&format!("Description: {}\n", result.description));
    }
    if !result.license.is_empty() {
        out.push_str(&format!("License: {}\n", result.license));
    }
    out.push('\n');

    out.push_str("Contracts:\n");
    if result.contract_packages.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for pkg in &result.contract_packages {
            out.push_str(&format!("  {pkg}\n"));
        }
    }
    out.push('\n');

    out.push_str("Compatibility:\n");
    out.push_str(&format!(
        "  Min Torvyn version: {}\n",
        result.min_torvyn_version
    ));
    out.push_str(&format!("  WASI target: {}\n", result.wasi_target));
    out.push_str(&format!("  Target arch: {}\n", result.target_arch));
    out.push('\n');

    if !result.capabilities_required.is_empty() {
        out.push_str("Required capabilities:\n");
        for cap in &result.capabilities_required {
            out.push_str(&format!("  {cap}\n"));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "Wasm binary size: {} bytes\n",
        result.wasm_size_bytes
    ));
    out.push_str(&format!("Build tool: {}\n", result.build_tool));

    if !result.wit_files.is_empty() {
        out.push_str(&format!("WIT files: {}\n", result.wit_files.join(", ")));
    }

    if let Some(ref prov) = result.provenance_summary {
        out.push_str("\nProvenance:\n");
        out.push_str(&format!("  Builder: {}\n", prov.builder_id));
        out.push_str(&format!(
            "  Built: {} -> {}\n",
            prov.build_started, prov.build_finished
        ));
        if let Some(ref repo) = prov.source_repo {
            out.push_str(&format!("  Source: {repo}\n"));
        }
        out.push_str(&format!("  Torvyn CLI: {}\n", prov.torvyn_cli_version));
    }

    if let Some(ref dep) = result.deprecation_message {
        out.push_str(&format!("\nDEPRECATED: {dep}\n"));
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{pack, PackInput};
    use crate::provenance::ProvenanceRecord;
    use tempfile::TempDir;

    fn make_test_wasm() -> Vec<u8> {
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\0asm");
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        wasm
    }

    fn create_and_pack(dir: &Path) -> std::path::PathBuf {
        let wasm_path = dir.join("component.wasm");
        std::fs::write(&wasm_path, make_test_wasm()).unwrap();

        let manifest_path = dir.join("Torvyn.toml");
        std::fs::write(
            &manifest_path,
            r#"
[component]
name = "inspectable"
version = "2.0.0"
description = "An inspectable component"
license = "Apache-2.0"

[contracts]
packages = ["torvyn:streaming@0.1.0", "torvyn:filtering@0.1.0"]

[compatibility]
min-torvyn-version = "0.3.0"
wasi-target = "preview2"

[build]
tool = "cargo-component"
tool-version = "0.20.0"
"#,
        )
        .unwrap();

        let wit_dir = dir.join("wit");
        std::fs::create_dir_all(&wit_dir).unwrap();
        std::fs::write(wit_dir.join("streaming.wit"), "package torvyn:streaming;\n").unwrap();

        let input = PackInput {
            wasm_path,
            manifest_path,
            wit_dir,
            provenance: ProvenanceRecord::builder("inspectable", "sha256:test")
                .torvyn_cli_version("0.3.0")
                .source_repo("https://github.com/example/test")
                .build_timestamps("2026-03-11T12:00:00Z", "2026-03-11T12:02:30Z")
                .build(),
        };

        let output_dir = dir.join("output");
        let pack_result = pack(&input, &output_dir).unwrap();
        pack_result.artifact_path
    }

    #[test]
    fn inspect_returns_correct_metadata() {
        let dir = TempDir::new().unwrap();
        let artifact_path = create_and_pack(dir.path());

        let result = inspect(&artifact_path).unwrap();
        assert_eq!(result.name, "inspectable");
        assert_eq!(result.version, "2.0.0");
        assert_eq!(result.description, "An inspectable component");
        assert_eq!(result.license, "Apache-2.0");
        assert_eq!(result.contract_packages.len(), 2);
        assert_eq!(result.min_torvyn_version, "0.3.0");
        assert_eq!(result.wasi_target, "preview2");
        assert!(result.wasm_size_bytes > 0);
        assert_eq!(result.build_tool, "cargo-component 0.20.0");
        assert!(result.wit_files.contains(&"streaming.wit".to_owned()));
    }

    #[test]
    fn inspect_includes_provenance() {
        let dir = TempDir::new().unwrap();
        let artifact_path = create_and_pack(dir.path());

        let result = inspect(&artifact_path).unwrap();
        let prov = result.provenance_summary.unwrap();
        assert_eq!(prov.builder_id, "local");
        assert_eq!(prov.torvyn_cli_version, "0.3.0");
        assert_eq!(
            prov.source_repo.as_deref(),
            Some("https://github.com/example/test")
        );
    }

    #[test]
    fn format_inspection_produces_readable_output() {
        let dir = TempDir::new().unwrap();
        let artifact_path = create_and_pack(dir.path());

        let result = inspect(&artifact_path).unwrap();
        let formatted = format_inspection(&result);

        assert!(formatted.contains("inspectable v2.0.0"));
        assert!(formatted.contains("torvyn:streaming@0.1.0"));
        assert!(formatted.contains("Min Torvyn version: 0.3.0"));
        assert!(formatted.contains("cargo-component 0.20.0"));
    }

    #[test]
    fn inspect_nonexistent_file_returns_error() {
        let result = inspect(Path::new("/nonexistent/bad.torvyn"));
        assert!(result.is_err());
    }
}
