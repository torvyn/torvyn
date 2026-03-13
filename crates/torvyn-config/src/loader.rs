//! Top-level configuration loading and orchestration.
//!
//! This module provides the primary public API for loading Torvyn
//! configuration from files. It orchestrates: file reading, TOML parsing,
//! environment variable interpolation, semantic validation, and merging.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{ConfigErrors, ConfigParseError};
use crate::manifest::ComponentManifest;
use crate::pipeline::{FlowDef, PipelineDefinition};
use crate::validate::{validate_manifest, validate_pipeline};

/// The fully resolved configuration for a Torvyn project.
///
/// This is the output of the configuration loading pipeline. It contains
/// the validated manifest and any flow definitions (either inline from
/// `Torvyn.toml` or from a separate `pipeline.toml`).
///
/// # Examples
/// ```
/// use torvyn_config::ResolvedConfig;
///
/// // Typically created via `load_config()`, but can be constructed directly for testing:
/// let config = ResolvedConfig {
///     manifest: Default::default(),
///     flows: Default::default(),
///     source_file: "Torvyn.toml".into(),
/// };
/// assert!(config.flows.is_empty());
/// ```
#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    /// The validated component manifest.
    pub manifest: ComponentManifest,

    /// Resolved flow definitions (from inline `[flow.*]` or `pipeline.toml`).
    pub flows: BTreeMap<String, FlowDef>,

    /// Path to the source configuration file.
    pub source_file: String,
}

/// Load and validate a Torvyn project configuration from the filesystem.
///
/// Looks for `Torvyn.toml` at the given path. If the manifest contains
/// inline `[flow.*]` tables, those are parsed as the pipeline definition.
/// If a `pipeline.toml` exists in the same directory, it is also loaded
/// and its flows are merged (pipeline.toml takes precedence on conflicts).
///
/// # COLD PATH — called once during startup.
///
/// # Errors
/// Returns `Err(Vec<ConfigParseError>)` if any parsing or validation fails.
/// All errors are collected and returned together.
///
/// # Examples
/// ```no_run
/// use torvyn_config::load_config;
///
/// let config = load_config("./Torvyn.toml").unwrap();
/// println!("Project: {}", config.manifest.torvyn.name);
/// ```
pub fn load_config(manifest_path: &str) -> Result<ResolvedConfig, Vec<ConfigParseError>> {
    let mut all_errors = ConfigErrors::new();

    // 1. Read manifest file
    let manifest_contents = read_file(manifest_path)?;

    // 2. Parse manifest
    let manifest = ComponentManifest::from_toml_str(&manifest_contents, manifest_path)?;

    // 3. Validate manifest semantically
    let validation_errors = validate_manifest(&manifest, manifest_path);
    if !validation_errors.is_empty() {
        return Err(validation_errors.into_vec());
    }

    // 4. Extract inline flows from manifest
    let mut flows = BTreeMap::new();
    if manifest.has_flows() {
        match PipelineDefinition::from_manifest_flows(&manifest.flow, manifest_path) {
            Ok(inline_flows) => {
                flows = inline_flows;
            }
            Err(errs) => {
                for e in errs {
                    all_errors.push(e);
                }
            }
        }
    }

    // 5. Check for pipeline.toml alongside the manifest
    let manifest_dir = Path::new(manifest_path)
        .parent()
        .unwrap_or(Path::new("."));
    let pipeline_path = manifest_dir.join("pipeline.toml");

    if pipeline_path.exists() {
        let pipeline_path_str = pipeline_path.to_string_lossy().to_string();
        match read_file(&pipeline_path_str) {
            Ok(pipeline_contents) => {
                match PipelineDefinition::from_toml_str(&pipeline_contents, &pipeline_path_str) {
                    Ok(pipeline_def) => {
                        let pipeline_validation =
                            validate_pipeline(&pipeline_def, &pipeline_path_str);
                        if !pipeline_validation.is_empty() {
                            for e in pipeline_validation {
                                all_errors.push(e);
                            }
                        } else {
                            // pipeline.toml flows take precedence over inline flows
                            for (name, flow) in pipeline_def.flows {
                                flows.insert(name, flow);
                            }
                        }
                    }
                    Err(errs) => {
                        for e in errs {
                            all_errors.push(e);
                        }
                    }
                }
            }
            Err(errs) => {
                for e in errs {
                    all_errors.push(e);
                }
            }
        }
    }

    all_errors.into_result()?;

    Ok(ResolvedConfig {
        manifest,
        flows,
        source_file: manifest_path.to_owned(),
    })
}

/// Load and validate just a manifest (no pipeline resolution).
///
/// Useful for `torvyn check` which only validates the project structure.
///
/// # COLD PATH — called once during startup.
///
/// # Errors
/// Returns `Err(Vec<ConfigParseError>)` if any parsing or validation fails.
pub fn load_manifest(manifest_path: &str) -> Result<ComponentManifest, Vec<ConfigParseError>> {
    let contents = read_file(manifest_path)?;
    let manifest = ComponentManifest::from_toml_str(&contents, manifest_path)?;

    let validation_errors = validate_manifest(&manifest, manifest_path);
    if !validation_errors.is_empty() {
        return Err(validation_errors.into_vec());
    }

    Ok(manifest)
}

/// Load and validate a standalone pipeline definition file.
///
/// # COLD PATH — called once during pipeline loading.
///
/// # Errors
/// Returns `Err(Vec<ConfigParseError>)` if any parsing or validation fails.
pub fn load_pipeline(pipeline_path: &str) -> Result<PipelineDefinition, Vec<ConfigParseError>> {
    let contents = read_file(pipeline_path)?;
    let pipeline = PipelineDefinition::from_toml_str(&contents, pipeline_path)?;

    let validation_errors = validate_pipeline(&pipeline, pipeline_path);
    if !validation_errors.is_empty() {
        return Err(validation_errors.into_vec());
    }

    Ok(pipeline)
}

/// Read a file to a string, returning a config error on failure.
///
/// # COLD PATH.
fn read_file(path: &str) -> Result<String, Vec<ConfigParseError>> {
    std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            vec![ConfigParseError::file_not_found(path)]
        } else {
            vec![ConfigParseError::invalid_value(
                path,
                "",
                &e.to_string(),
                "a readable file",
                &format!("Check file permissions and path: {path}"),
            )]
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_file(name: &str, content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{content}").unwrap();
        dir
    }

    #[test]
    fn test_load_config_valid_manifest() {
        let dir = write_temp_file(
            "Torvyn.toml",
            r#"
[torvyn]
name = "test-project"
version = "0.1.0"
contract_version = "0.1.0"
"#,
        );
        let path = dir.path().join("Torvyn.toml");
        let config = load_config(path.to_str().unwrap()).unwrap();
        assert_eq!(config.manifest.torvyn.name, "test-project");
        assert!(config.flows.is_empty());
    }

    #[test]
    fn test_load_config_with_inline_flows() {
        let dir = write_temp_file(
            "Torvyn.toml",
            r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[flow.main.nodes.src]
component = "a.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "b.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "src", port = "output" }
to = { node = "sink", port = "input" }
"#,
        );
        let path = dir.path().join("Torvyn.toml");
        let config = load_config(path.to_str().unwrap()).unwrap();
        assert!(config.flows.contains_key("main"));
    }

    #[test]
    fn test_load_config_file_not_found() {
        let result = load_config("/nonexistent/Torvyn.toml");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.code == "E0700"));
    }

    #[test]
    fn test_load_manifest_valid() {
        let dir = write_temp_file(
            "Torvyn.toml",
            r#"
[torvyn]
name = "simple"
version = "0.1.0"
contract_version = "0.1.0"
"#,
        );
        let path = dir.path().join("Torvyn.toml");
        let manifest = load_manifest(path.to_str().unwrap()).unwrap();
        assert_eq!(manifest.torvyn.name, "simple");
    }

    #[test]
    fn test_load_manifest_invalid_returns_errors() {
        let dir = write_temp_file(
            "Torvyn.toml",
            r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[runtime.scheduling]
policy = "nonexistent"
"#,
        );
        let path = dir.path().join("Torvyn.toml");
        let result = load_manifest(path.to_str().unwrap());
        assert!(result.is_err());
    }
}
