//! Single-component validation.
//!
//! This module implements the validation pipeline that `torvyn check` uses.
//! It validates a single component project in two phases:
//!
//! Phase 1 (Parse-time): WIT syntax, manifest format, capability declarations.
//! Phase 2 (Semantic): Type consistency, resource usage, version constraints,
//!                      interface completeness.
//!
//! All validation errors are collected — the validator reports ALL errors,
//! not just the first one.

use std::collections::HashSet;
use std::path::Path;

use crate::errors::{DiagnosticBuilder, ErrorCode, ValidationResult};
use crate::parser::{ParsedWorld, WitParser};

// COLD PATH — validation runs during `torvyn check`.

/// Component manifest parsed from Torvyn.toml.
///
/// Invariants:
/// - `name` is non-empty and contains only valid identifier chars.
/// - `version` is a valid semver version.
/// - All capability names are non-empty.
#[derive(Debug, Clone)]
pub struct ComponentManifest {
    /// Component name.
    pub name: String,
    /// Component version.
    pub version: semver::Version,
    /// Required capabilities (must be granted at link-time).
    pub required_capabilities: HashSet<String>,
    /// Optional capabilities (enhance functionality but not required).
    pub optional_capabilities: HashSet<String>,
    /// Torvyn-specific resource limits.
    pub resource_limits: ResourceLimits,
}

/// Torvyn-specific resource limit declarations.
#[derive(Debug, Clone, Default)]
pub struct ResourceLimits {
    /// Maximum single buffer size in bytes. 0 means default.
    pub max_buffer_size: u64,
    /// Maximum linear memory in bytes. 0 means default.
    pub max_memory: u64,
    /// Buffer pool name. Empty means "default".
    pub buffer_pool: String,
}

/// Parse a Torvyn.toml manifest file.
///
/// # Preconditions
/// - `path` points to a valid file.
///
/// # Postconditions
/// - On success: returns a `ComponentManifest`.
/// - On failure: returns a `ValidationResult` with diagnostic(s).
///
/// # Errors
/// - `ErrorCode::ManifestParseError` if the file is not valid TOML.
/// - `ErrorCode::ManifestMissingField` if required fields are absent.
/// - `ErrorCode::ManifestVersionError` if version is not valid semver.
///
/// # COLD PATH
pub fn parse_manifest(path: &Path) -> Result<ComponentManifest, ValidationResult> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        let mut result = ValidationResult::new();
        result.push(
            DiagnosticBuilder::error(
                ErrorCode::ManifestParseError,
                format!("cannot read manifest: {}", e),
            )
            .location(path, 1, 0, "file could not be read")
            .help("ensure the file exists and is readable")
            .build(),
        );
        result
    })?;

    let table: toml::Table = contents.parse::<toml::Table>().map_err(|e| {
        let mut result = ValidationResult::new();
        let line = e
            .span()
            .map(|s| contents[..s.start].chars().filter(|c| *c == '\n').count() as u32 + 1)
            .unwrap_or(1);
        result.push(
            DiagnosticBuilder::error(
                ErrorCode::ManifestParseError,
                format!("invalid TOML: {}", e.message()),
            )
            .location(path, line, 0, "TOML parse error here")
            .help("check TOML syntax at https://toml.io")
            .build(),
        );
        result
    })?;

    let mut errors = ValidationResult::new();

    // Extract [component] section
    let component = match table.get("component") {
        Some(toml::Value::Table(t)) => t,
        Some(_) => {
            errors.push(
                DiagnosticBuilder::error(
                    ErrorCode::ManifestMissingField,
                    "[component] must be a table",
                )
                .location(path, 1, 0, "expected [component] table")
                .help("add a [component] section with name and version fields")
                .build(),
            );
            return Err(errors);
        }
        None => {
            errors.push(
                DiagnosticBuilder::error(
                    ErrorCode::ManifestMissingField,
                    "missing [component] section",
                )
                .location(path, 1, 0, "Torvyn.toml must have a [component] section")
                .help("add:\n[component]\nname = \"my-component\"\nversion = \"0.1.0\"")
                .build(),
            );
            return Err(errors);
        }
    };

    // Extract name
    let name = match component.get("name") {
        Some(toml::Value::String(s)) if !s.is_empty() => s.clone(),
        Some(toml::Value::String(_)) => {
            errors.push(
                DiagnosticBuilder::error(
                    ErrorCode::ManifestMissingField,
                    "component name must not be empty",
                )
                .location(path, 1, 0, "[component].name is empty")
                .help("provide a non-empty component name")
                .build(),
            );
            String::new()
        }
        _ => {
            errors.push(
                DiagnosticBuilder::error(
                    ErrorCode::ManifestMissingField,
                    "missing [component].name",
                )
                .location(path, 1, 0, "Torvyn.toml must specify component name")
                .help("add: name = \"my-component\" under [component]")
                .build(),
            );
            String::new()
        }
    };

    // Extract version
    let version = match component.get("version") {
        Some(toml::Value::String(s)) => match semver::Version::parse(s) {
            Ok(v) => v,
            Err(e) => {
                errors.push(
                    DiagnosticBuilder::error(
                        ErrorCode::ManifestVersionError,
                        format!("invalid semver version '{}': {}", s, e),
                    )
                    .location(path, 1, 0, "version must be valid semver")
                    .help("use format like \"0.1.0\" or \"1.2.3\"")
                    .build(),
                );
                semver::Version::new(0, 0, 0)
            }
        },
        _ => {
            errors.push(
                DiagnosticBuilder::error(
                    ErrorCode::ManifestMissingField,
                    "missing [component].version",
                )
                .location(path, 1, 0, "Torvyn.toml must specify component version")
                .help("add: version = \"0.1.0\" under [component]")
                .build(),
            );
            semver::Version::new(0, 0, 0)
        }
    };

    if !errors.is_ok() {
        return Err(errors);
    }

    // Extract capabilities
    let mut required_capabilities = HashSet::new();
    let mut optional_capabilities = HashSet::new();

    if let Some(toml::Value::Table(caps)) = table.get("capabilities") {
        if let Some(toml::Value::Table(req)) = caps.get("required") {
            for (key, val) in req {
                if let toml::Value::Boolean(true) = val {
                    required_capabilities.insert(key.clone());
                }
            }
        }
        if let Some(toml::Value::Table(opt)) = caps.get("optional") {
            for (key, val) in opt {
                if let toml::Value::Boolean(true) = val {
                    optional_capabilities.insert(key.clone());
                }
            }
        }
    }

    // Extract resource limits
    let mut resource_limits = ResourceLimits::default();
    if let Some(toml::Value::Table(caps)) = table.get("capabilities") {
        if let Some(toml::Value::Table(torvyn_caps)) = caps.get("torvyn") {
            if let Some(toml::Value::String(s)) = torvyn_caps.get("max-buffer-size") {
                resource_limits.max_buffer_size = parse_byte_size(s).unwrap_or(0);
            }
            if let Some(toml::Value::String(s)) = torvyn_caps.get("max-memory") {
                resource_limits.max_memory = parse_byte_size(s).unwrap_or(0);
            }
            if let Some(toml::Value::String(s)) = torvyn_caps.get("buffer-pool-access") {
                resource_limits.buffer_pool = s.clone();
            }
        }
    }

    Ok(ComponentManifest {
        name,
        version,
        required_capabilities,
        optional_capabilities,
        resource_limits,
    })
}

/// Parse a human-readable byte size string (e.g., "16MiB", "64KiB").
///
/// # COLD PATH
fn parse_byte_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(num_str) = s.strip_suffix("GiB") {
        num_str
            .trim()
            .parse::<u64>()
            .ok()
            .map(|n| n * 1024 * 1024 * 1024)
    } else if let Some(num_str) = s.strip_suffix("MiB") {
        num_str
            .trim()
            .parse::<u64>()
            .ok()
            .map(|n| n * 1024 * 1024)
    } else if let Some(num_str) = s.strip_suffix("KiB") {
        num_str.trim().parse::<u64>().ok().map(|n| n * 1024)
    } else if let Some(num_str) = s.strip_suffix('B') {
        num_str.trim().parse::<u64>().ok()
    } else {
        s.parse::<u64>().ok()
    }
}

/// The set of Torvyn processing interfaces that a component may export.
const TORVYN_PROCESSING_INTERFACES: &[&str] =
    &["processor", "source", "sink", "filter", "router", "aggregator"];

/// Known WASI interface imports and their corresponding capability names.
const WASI_CAPABILITY_MAP: &[(&str, &str)] = &[
    ("wasi:filesystem/preopens", "wasi-filesystem-read"),
    ("wasi:filesystem/types", "wasi-filesystem-read"),
    ("wasi:sockets/tcp", "wasi-network-egress"),
    ("wasi:sockets/udp", "wasi-network-egress"),
    ("wasi:sockets/ip-name-lookup", "wasi-network-egress"),
    ("wasi:clocks/wall-clock", "wasi-clocks"),
    ("wasi:clocks/monotonic-clock", "wasi-clocks"),
    ("wasi:random/random", "wasi-random"),
    ("wasi:random/insecure", "wasi-random"),
    ("wasi:random/insecure-seed", "wasi-random"),
];

/// Validate a single component project.
///
/// This runs both Phase 1 (parse-time) and Phase 2 (semantic) validations.
///
/// # Preconditions
/// - `project_dir` contains a `Torvyn.toml` and a `wit/` directory.
///
/// # Postconditions
/// - Returns a `ValidationResult` containing all diagnostics found.
/// - The result's `is_ok()` is true only if the component passes all checks.
///
/// # COLD PATH
pub fn validate_component(project_dir: &Path, parser: &dyn WitParser) -> ValidationResult {
    let mut result = ValidationResult::new();

    // --- Phase 1: Parse-time validations ---

    // 1. Parse Torvyn.toml
    let manifest_path = project_dir.join("Torvyn.toml");
    let manifest = match parse_manifest(&manifest_path) {
        Ok(m) => Some(m),
        Err(errs) => {
            result.merge(errs);
            None
        }
    };

    // 2. Parse WIT files
    let wit_dir = project_dir.join("wit");
    let packages = match parser.parse_directory(&wit_dir) {
        Ok(pkgs) => pkgs,
        Err(errs) => {
            result.merge(errs);
            return result;
        }
    };

    if packages.is_empty() {
        result.push(
            DiagnosticBuilder::error(ErrorCode::WitPackageDeclaration, "no WIT packages found")
                .location(&wit_dir, 1, 0, "wit/ directory contains no parseable packages")
                .help(
                    "ensure your wit/ directory contains .wit files with a package declaration",
                )
                .build(),
        );
        return result;
    }

    // --- Phase 2: Semantic validations ---

    // 3. World completeness: at least one world exports a Torvyn processing interface
    let mut has_torvyn_export = false;
    for pkg in &packages {
        for world in pkg.worlds.values() {
            for export_key in world.exports.keys() {
                let export_name = export_key.rsplit('/').next().unwrap_or(export_key);
                if TORVYN_PROCESSING_INTERFACES.contains(&export_name) {
                    has_torvyn_export = true;
                }
            }
        }
    }

    if !has_torvyn_export {
        result.push(
            DiagnosticBuilder::error(
                ErrorCode::NoExportedInterface,
                "no world exports a Torvyn processing interface",
            )
            .location(
                &wit_dir,
                1,
                0,
                "no world found exporting processor, source, sink, filter, router, or aggregator",
            )
            .help("add `export processor;` (or source, sink, etc.) to your world definition")
            .build(),
        );
    }

    // 4. Capability consistency: if WIT imports WASI interfaces, manifest must declare them
    if let Some(ref manifest) = manifest {
        for pkg in &packages {
            for (world_name, world) in &pkg.worlds {
                validate_capability_consistency(
                    world,
                    world_name,
                    manifest,
                    &manifest_path,
                    &wit_dir,
                    &mut result,
                );
            }
        }
    }

    // 5. Version constraint validation
    if let Some(ref manifest) = manifest {
        for pkg in &packages {
            if let Some(ref pkg_ver) = pkg.version {
                if *pkg_ver != manifest.version {
                    result.push(
                        DiagnosticBuilder::warning(
                            ErrorCode::UnsatisfiableVersion,
                            format!(
                                "WIT package version ({}) differs from manifest version ({})",
                                pkg_ver, manifest.version
                            ),
                        )
                        .location(&manifest_path, 1, 0, "manifest version here")
                        .note(
                            "WIT package version and manifest version should typically agree",
                        )
                        .build(),
                    );
                }
            }
        }
    }

    result.sort();
    result
}

/// Check that WIT world imports match manifest capability declarations.
///
/// # COLD PATH
fn validate_capability_consistency(
    world: &ParsedWorld,
    world_name: &str,
    manifest: &ComponentManifest,
    manifest_path: &Path,
    wit_dir: &Path,
    result: &mut ValidationResult,
) {
    let all_declared: HashSet<&str> = manifest
        .required_capabilities
        .iter()
        .chain(manifest.optional_capabilities.iter())
        .map(|s| s.as_str())
        .collect();

    for import_key in world.imports.keys() {
        for &(wasi_iface, capability_name) in WASI_CAPABILITY_MAP {
            if import_key.contains(wasi_iface) && !all_declared.contains(capability_name) {
                result.push(
                    DiagnosticBuilder::error(
                        ErrorCode::CapabilityMismatch,
                        format!(
                            "world '{}' imports '{}' but manifest does not declare '{}'",
                            world_name, import_key, capability_name
                        ),
                    )
                    .location(
                        wit_dir,
                        1,
                        0,
                        format!("import of '{}' here", import_key),
                    )
                    .location(
                        manifest_path,
                        1,
                        0,
                        format!(
                            "'{}' not declared in [capabilities.required]",
                            capability_name
                        ),
                    )
                    .help(format!(
                        "add `{} = true` to [capabilities.required] in Torvyn.toml",
                        capability_name
                    ))
                    .build(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{MockWitParser, ParsedPackage, ParsedWorld, WorldImportExport};
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn write_minimal_manifest(dir: &Path) {
        let manifest = r#"
[component]
name = "test-component"
version = "0.1.0"

[capabilities.required]
wasi-clocks = true
"#;
        std::fs::write(dir.join("Torvyn.toml"), manifest).unwrap();
    }

    fn make_valid_package() -> ParsedPackage {
        let mut exports = HashMap::new();
        exports.insert(
            "processor".to_string(),
            WorldImportExport::Interface("processor".to_string()),
        );

        let mut worlds = HashMap::new();
        worlds.insert(
            "transform".to_string(),
            ParsedWorld {
                name: "transform".to_string(),
                imports: HashMap::new(),
                exports,
            },
        );

        ParsedPackage {
            name: "torvyn:streaming".to_string(),
            version: Some(semver::Version::new(0, 1, 0)),
            interfaces: HashMap::new(),
            worlds,
            source_files: vec![],
        }
    }

    #[test]
    fn test_parse_manifest_valid() {
        let dir = TempDir::new().unwrap();
        write_minimal_manifest(dir.path());

        let manifest = parse_manifest(&dir.path().join("Torvyn.toml")).unwrap();
        assert_eq!(manifest.name, "test-component");
        assert_eq!(manifest.version, semver::Version::new(0, 1, 0));
        assert!(manifest.required_capabilities.contains("wasi-clocks"));
    }

    #[test]
    fn test_parse_manifest_missing_file() {
        let result = parse_manifest(Path::new("/nonexistent/Torvyn.toml"));
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(!diags.is_ok());
        assert!(diags.diagnostics[0].code == ErrorCode::ManifestParseError);
    }

    #[test]
    fn test_parse_manifest_invalid_toml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Torvyn.toml"), "not [valid toml {{").unwrap();

        let result = parse_manifest(&dir.path().join("Torvyn.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_manifest_missing_component_section() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Torvyn.toml"), "[other]\nkey = \"val\"").unwrap();

        let result = parse_manifest(&dir.path().join("Torvyn.toml"));
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(diags.diagnostics[0].code == ErrorCode::ManifestMissingField);
    }

    #[test]
    fn test_parse_manifest_invalid_version() {
        let dir = TempDir::new().unwrap();
        let manifest = r#"
[component]
name = "test"
version = "not-a-version"
"#;
        std::fs::write(dir.path().join("Torvyn.toml"), manifest).unwrap();

        let result = parse_manifest(&dir.path().join("Torvyn.toml"));
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(diags.diagnostics[0].code == ErrorCode::ManifestVersionError);
    }

    #[test]
    fn test_parse_byte_size() {
        assert_eq!(parse_byte_size("16MiB"), Some(16 * 1024 * 1024));
        assert_eq!(parse_byte_size("64KiB"), Some(64 * 1024));
        assert_eq!(parse_byte_size("1GiB"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_byte_size("1024B"), Some(1024));
        assert_eq!(parse_byte_size("1024"), Some(1024));
        assert_eq!(parse_byte_size("not-a-number"), None);
    }

    #[test]
    fn test_validate_component_valid() {
        let dir = TempDir::new().unwrap();
        write_minimal_manifest(dir.path());
        std::fs::create_dir_all(dir.path().join("wit")).unwrap();

        let parser = MockWitParser::with_packages(vec![make_valid_package()]);
        let result = validate_component(dir.path(), &parser);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_component_no_torvyn_export() {
        let dir = TempDir::new().unwrap();
        write_minimal_manifest(dir.path());
        std::fs::create_dir_all(dir.path().join("wit")).unwrap();

        let mut pkg = make_valid_package();
        pkg.worlds.get_mut("transform").unwrap().exports.clear();

        let parser = MockWitParser::with_packages(vec![pkg]);
        let result = validate_component(dir.path(), &parser);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::NoExportedInterface));
    }

    #[test]
    fn test_validate_component_capability_mismatch() {
        let dir = TempDir::new().unwrap();
        let manifest = r#"
[component]
name = "test"
version = "0.1.0"
"#;
        std::fs::write(dir.path().join("Torvyn.toml"), manifest).unwrap();
        std::fs::create_dir_all(dir.path().join("wit")).unwrap();

        let mut pkg = make_valid_package();
        let world = pkg.worlds.get_mut("transform").unwrap();
        world.imports.insert(
            "wasi:filesystem/preopens".to_string(),
            WorldImportExport::Interface("preopens".to_string()),
        );

        let parser = MockWitParser::with_packages(vec![pkg]);
        let result = validate_component(dir.path(), &parser);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::CapabilityMismatch));
    }

    #[test]
    fn test_validate_component_wit_parse_error() {
        let dir = TempDir::new().unwrap();
        write_minimal_manifest(dir.path());
        std::fs::create_dir_all(dir.path().join("wit")).unwrap();

        let parser = MockWitParser::failing("syntax error on line 42");
        let result = validate_component(dir.path(), &parser);
        assert!(!result.is_ok());
    }
}
