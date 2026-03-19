//! Component manifest schema (`Torvyn.toml`).
//!
//! This is the first of two configuration contexts in Torvyn's
//! two-configuration-context model (Doc 10, Recommendation 3):
//!
//! 1. **Component Manifest** (this module) — describes a component project.
//! 2. **Pipeline Definition** (`pipeline` module) — describes flow topology.
//!
//! A single `Torvyn.toml` may contain both: the manifest metadata plus
//! inline `[flow.*]` tables. The manifest parser extracts its portion;
//! the pipeline parser extracts the flow portion.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{ConfigErrors, ConfigParseError};
use crate::runtime::{ObservabilityConfig, RuntimeConfig, SecurityConfig};

// ---------------------------------------------------------------------------
// ProjectMetadata
// ---------------------------------------------------------------------------

/// Top-level project metadata from the `[torvyn]` table.
///
/// # Invariants
/// - `name` is non-empty and contains only `[a-zA-Z0-9_-]`.
/// - `version` is a valid semver string.
/// - `contract_version` is a valid semver string matching an available
///   Torvyn contract package version.
///
/// # Examples
/// ```
/// use torvyn_config::ProjectMetadata;
///
/// let meta = ProjectMetadata {
///     name: "my-transform".into(),
///     version: "0.1.0".into(),
///     contract_version: "0.1.0".into(),
///     ..Default::default()
/// };
/// assert_eq!(meta.name, "my-transform");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    /// Project name. Must match `[a-zA-Z0-9_-]+`.
    #[serde(default)]
    pub name: String,

    /// Project version (semver).
    #[serde(default)]
    pub version: String,

    /// Torvyn contract version this project targets.
    /// Must match an available `torvyn:streaming@<version>` package.
    #[serde(default)]
    pub contract_version: String,

    /// Human-readable description.
    /// Default: `""`.
    #[serde(default)]
    pub description: String,

    /// Author names and/or emails.
    /// Default: `[]`.
    #[serde(default)]
    pub authors: Vec<String>,

    /// SPDX license identifier.
    /// Default: `""`.
    #[serde(default)]
    pub license: String,

    /// Source repository URL.
    /// Default: `""`.
    #[serde(default)]
    pub repository: String,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: "0.1.0".to_owned(),
            contract_version: "0.1.0".to_owned(),
            description: String::new(),
            authors: Vec::new(),
            license: String::new(),
            repository: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ComponentDecl
// ---------------------------------------------------------------------------

/// A component declaration within a multi-component project.
///
/// Corresponds to a `[[component]]` array entry in `Torvyn.toml`.
///
/// # Invariants
/// - `name` is non-empty and contains only `[a-zA-Z0-9_-]`.
/// - `path` is a valid relative directory path.
/// - `language` is one of the supported languages.
///
/// # Examples
/// ```
/// use torvyn_config::ComponentDecl;
///
/// let decl = ComponentDecl {
///     name: "tokenizer".into(),
///     path: "components/tokenizer".into(),
///     ..Default::default()
/// };
/// assert_eq!(decl.language, "rust");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentDecl {
    /// Component name within this project.
    pub name: String,

    /// Path to component source root, relative to the project root.
    pub path: String,

    /// Implementation language.
    /// Default: `"rust"`.
    /// Supported: `"rust"`, `"go"`, `"python"`, `"zig"`.
    #[serde(default = "default_language")]
    pub language: String,

    /// Custom build command override.
    /// If set, `torvyn build` invokes this instead of the default toolchain.
    /// Default: auto-detected based on `language`.
    #[serde(default)]
    pub build_command: Option<String>,

    /// Per-component JSON configuration string passed to `lifecycle.init()`.
    /// This is an opaque JSON blob; the runtime does not interpret it.
    /// Default: `None`.
    #[serde(default)]
    pub config: Option<String>,
}

fn default_language() -> String {
    "rust".to_owned()
}

impl Default for ComponentDecl {
    fn default() -> Self {
        Self {
            name: String::new(),
            path: String::new(),
            language: default_language(),
            build_command: None,
            config: None,
        }
    }
}

// ---------------------------------------------------------------------------
// BuildConfig
// ---------------------------------------------------------------------------

/// Build configuration from the `[build]` table.
///
/// # Examples
/// ```
/// use torvyn_config::BuildConfig;
///
/// let cfg = BuildConfig::default();
/// assert!(cfg.release);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Whether to build in release mode by default.
    /// Default: `true`.
    #[serde(default = "default_true")]
    pub release: bool,

    /// Additional arguments passed to the build toolchain.
    /// Default: `[]`.
    #[serde(default)]
    pub extra_args: Vec<String>,

    /// Target triple override.
    /// Default: `"wasm32-wasip2"`.
    #[serde(default = "default_target")]
    pub target: String,
}

fn default_true() -> bool {
    true
}

fn default_target() -> String {
    "wasm32-wasip2".to_owned()
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            release: true,
            extra_args: Vec::new(),
            target: default_target(),
        }
    }
}

// ---------------------------------------------------------------------------
// TestConfig
// ---------------------------------------------------------------------------

/// Test configuration from the `[test]` table.
///
/// # Examples
/// ```
/// use torvyn_config::TestConfig;
///
/// let cfg = TestConfig::default();
/// assert_eq!(cfg.timeout_seconds, 60);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestConfig {
    /// Test timeout in seconds.
    /// Default: `60`.
    #[serde(default = "default_test_timeout")]
    pub timeout_seconds: u64,

    /// Test fixtures directory relative to the project root.
    /// Default: `"tests/fixtures"`.
    #[serde(default = "default_fixtures_dir")]
    pub fixtures_dir: String,
}

fn default_test_timeout() -> u64 {
    60
}

fn default_fixtures_dir() -> String {
    "tests/fixtures".to_owned()
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_test_timeout(),
            fixtures_dir: default_fixtures_dir(),
        }
    }
}

// ---------------------------------------------------------------------------
// RegistryConfig
// ---------------------------------------------------------------------------

/// Registry configuration from the `[registry]` table.
///
/// # Examples
/// ```
/// use torvyn_config::RegistryConfig;
///
/// let cfg = RegistryConfig {
///     default_url: Some("https://registry.example.com".into()),
/// };
/// assert!(cfg.default_url.is_some());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Default registry URL for `torvyn publish`.
    #[serde(default, rename = "default")]
    pub default_url: Option<String>,
}

// ---------------------------------------------------------------------------
// ComponentManifest
// ---------------------------------------------------------------------------

/// The complete parsed `Torvyn.toml` component manifest.
///
/// This is the top-level struct for a Torvyn project configuration file.
/// It may optionally include inline pipeline definitions (via the `flows`
/// field, parsed by the `pipeline` module).
///
/// # Invariants
/// - `torvyn` metadata is always present with at least `name`, `version`,
///   and `contract_version`.
/// - If `components` is non-empty, this is a multi-component workspace.
///   If empty, this is a single-component project (the project root IS
///   the component).
///
/// # Examples
/// ```
/// use torvyn_config::ComponentManifest;
///
/// let toml_str = r#"
/// [torvyn]
/// name = "my-component"
/// version = "0.1.0"
/// contract_version = "0.1.0"
/// "#;
///
/// let manifest = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml").unwrap();
/// assert_eq!(manifest.torvyn.name, "my-component");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentManifest {
    /// Project metadata (`[torvyn]` table).
    pub torvyn: ProjectMetadata,

    /// Component declarations for multi-component projects.
    /// Empty for single-component projects.
    /// Corresponds to `[[component]]` array-of-tables.
    #[serde(default, rename = "component")]
    pub components: Vec<ComponentDecl>,

    /// Build configuration (`[build]` table).
    #[serde(default)]
    pub build: BuildConfig,

    /// Test configuration (`[test]` table).
    #[serde(default)]
    pub test: TestConfig,

    /// Runtime configuration overrides (`[runtime]` table).
    #[serde(default)]
    pub runtime: RuntimeConfig,

    /// Observability configuration (`[observability]` table).
    #[serde(default)]
    pub observability: ObservabilityConfig,

    /// Security configuration (`[security]` table).
    #[serde(default)]
    pub security: SecurityConfig,

    /// Registry configuration (`[registry]` table).
    #[serde(default)]
    pub registry: RegistryConfig,

    /// Inline flow definitions (`[flow.*]` tables).
    /// These are parsed by the pipeline module but stored here for
    /// round-trip serialization.
    #[serde(default)]
    pub flow: BTreeMap<String, toml::Value>,

    /// Extension fields for forward compatibility.
    /// Unknown top-level tables are captured here rather than rejected,
    /// allowing newer config files to be partially parsed by older versions.
    #[serde(flatten)]
    pub extensions: BTreeMap<String, toml::Value>,
}

impl ComponentManifest {
    /// Parse a `ComponentManifest` from a TOML string.
    ///
    /// Returns the parsed manifest or a list of errors.
    ///
    /// # COLD PATH — called during project loading.
    ///
    /// # Errors
    /// Returns `Err(Vec<ConfigParseError>)` if the TOML is syntactically
    /// invalid or required fields are missing.
    ///
    /// # Examples
    /// ```
    /// use torvyn_config::ComponentManifest;
    ///
    /// let toml_str = r#"
    /// [torvyn]
    /// name = "example"
    /// version = "0.1.0"
    /// contract_version = "0.1.0"
    /// "#;
    ///
    /// let manifest = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml").unwrap();
    /// assert_eq!(manifest.torvyn.name, "example");
    /// ```
    pub fn from_toml_str(toml_str: &str, file_path: &str) -> Result<Self, Vec<ConfigParseError>> {
        let manifest: Self = toml::from_str(toml_str)
            .map_err(|e| vec![ConfigParseError::toml_syntax(file_path, &e)])?;

        let mut errors = ConfigErrors::new();
        manifest.validate_required_fields(file_path, &mut errors);

        errors.into_result()?;
        Ok(manifest)
    }

    /// Serialize this manifest back to a TOML string.
    ///
    /// # COLD PATH — called during config generation or round-trip testing.
    ///
    /// # Errors
    /// Returns `Err` if serialization fails (should not happen for valid manifests).
    pub fn to_toml_string(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Validate that required fields are present and non-empty.
    ///
    /// # COLD PATH — called once during parsing.
    fn validate_required_fields(&self, file: &str, errors: &mut ConfigErrors) {
        if self.torvyn.name.is_empty() {
            errors.push(ConfigParseError::missing_field(
                file,
                "torvyn.name",
                "torvyn",
            ));
        } else if !is_valid_name(&self.torvyn.name) {
            errors.push(ConfigParseError::invalid_value(
                file,
                "torvyn.name",
                &self.torvyn.name,
                "a string matching [a-zA-Z0-9_-]+",
                "Use only alphanumeric characters, hyphens, and underscores.",
            ));
        }

        if self.torvyn.version.is_empty() {
            errors.push(ConfigParseError::missing_field(
                file,
                "torvyn.version",
                "torvyn",
            ));
        } else if !is_valid_semver(&self.torvyn.version) {
            errors.push(ConfigParseError::invalid_value(
                file,
                "torvyn.version",
                &self.torvyn.version,
                "a semver string (e.g., \"0.1.0\")",
                "Use a valid semantic version.",
            ));
        }

        if self.torvyn.contract_version.is_empty() {
            errors.push(ConfigParseError::missing_field(
                file,
                "torvyn.contract_version",
                "torvyn",
            ));
        } else if !is_valid_semver(&self.torvyn.contract_version) {
            errors.push(ConfigParseError::invalid_value(
                file,
                "torvyn.contract_version",
                &self.torvyn.contract_version,
                "a semver string (e.g., \"0.1.0\")",
                "Use a valid semantic version.",
            ));
        }

        // Validate component declarations
        for (i, comp) in self.components.iter().enumerate() {
            let prefix = format!("component[{i}]");
            if comp.name.is_empty() {
                errors.push(ConfigParseError::missing_field(
                    file,
                    &format!("{prefix}.name"),
                    &prefix,
                ));
            } else if !is_valid_name(&comp.name) {
                errors.push(ConfigParseError::invalid_value(
                    file,
                    &format!("{prefix}.name"),
                    &comp.name,
                    "a string matching [a-zA-Z0-9_-]+",
                    "Use only alphanumeric characters, hyphens, and underscores.",
                ));
            }
            if comp.path.is_empty() {
                errors.push(ConfigParseError::missing_field(
                    file,
                    &format!("{prefix}.path"),
                    &prefix,
                ));
            }
            if !is_valid_language(&comp.language) {
                errors.push(ConfigParseError::invalid_value(
                    file,
                    &format!("{prefix}.language"),
                    &comp.language,
                    "one of: rust, go, python, zig",
                    "Use a supported language identifier.",
                ));
            }
        }

        // Check for duplicate component names
        let mut seen_names = std::collections::HashSet::new();
        for comp in &self.components {
            if !comp.name.is_empty() && !seen_names.insert(&comp.name) {
                errors.push(ConfigParseError::duplicate_key(
                    file,
                    &format!("component.name={}", comp.name),
                ));
            }
        }
    }

    /// Returns `true` if this is a multi-component workspace project.
    pub fn is_workspace(&self) -> bool {
        !self.components.is_empty()
    }

    /// Returns `true` if this manifest contains inline flow definitions.
    pub fn has_flows(&self) -> bool {
        !self.flow.is_empty()
    }
}

/// Validate that a name contains only allowed characters: `[a-zA-Z0-9_-]`.
///
/// # COLD PATH — called during validation.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Basic semver validation: three dot-separated numeric components.
///
/// This is a simplified check. For production, consider using a semver
/// parsing crate.
///
/// # COLD PATH — called during validation.
fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| p.parse::<u64>().is_ok())
}

/// Validate that a language identifier is supported.
///
/// # COLD PATH — called during validation.
fn is_valid_language(lang: &str) -> bool {
    matches!(lang, "rust" | "go" | "python" | "zig")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_MANIFEST: &str = r#"
[torvyn]
name = "my-component"
version = "0.1.0"
contract_version = "0.1.0"
"#;

    #[test]
    fn test_manifest_parse_minimal() {
        let manifest = ComponentManifest::from_toml_str(MINIMAL_MANIFEST, "Torvyn.toml").unwrap();
        assert_eq!(manifest.torvyn.name, "my-component");
        assert_eq!(manifest.torvyn.version, "0.1.0");
        assert_eq!(manifest.torvyn.contract_version, "0.1.0");
        assert!(manifest.torvyn.description.is_empty());
        assert!(manifest.torvyn.authors.is_empty());
        assert!(!manifest.is_workspace());
        assert!(!manifest.has_flows());
    }

    #[test]
    fn test_manifest_parse_full_metadata() {
        let toml_str = r#"
[torvyn]
name = "token-pipeline"
version = "1.2.3"
contract_version = "0.1.0"
description = "A streaming token pipeline"
authors = ["Alice <alice@example.com>", "Bob <bob@example.com>"]
license = "Apache-2.0"
repository = "https://github.com/torvyn/torvyn"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml").unwrap();
        assert_eq!(manifest.torvyn.description, "A streaming token pipeline");
        assert_eq!(manifest.torvyn.authors.len(), 2);
        assert_eq!(manifest.torvyn.license, "Apache-2.0");
    }

    #[test]
    fn test_manifest_parse_with_components() {
        let toml_str = r#"
[torvyn]
name = "my-pipeline"
version = "0.1.0"
contract_version = "0.1.0"

[[component]]
name = "tokenizer"
path = "components/tokenizer"

[[component]]
name = "writer"
path = "components/writer"
language = "go"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml").unwrap();
        assert!(manifest.is_workspace());
        assert_eq!(manifest.components.len(), 2);
        assert_eq!(manifest.components[0].name, "tokenizer");
        assert_eq!(manifest.components[0].language, "rust");
        assert_eq!(manifest.components[1].name, "writer");
        assert_eq!(manifest.components[1].language, "go");
    }

    #[test]
    fn test_manifest_missing_name_returns_error() {
        let toml_str = r#"
[torvyn]
version = "0.1.0"
contract_version = "0.1.0"
"#;
        let result = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.key_path == "torvyn.name"));
    }

    #[test]
    fn test_manifest_invalid_name_returns_error() {
        let toml_str = r#"
[torvyn]
name = "my component!"
version = "0.1.0"
contract_version = "0.1.0"
"#;
        let result = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.code == "E0703"));
    }

    #[test]
    fn test_manifest_invalid_version_returns_error() {
        let toml_str = r#"
[torvyn]
name = "valid-name"
version = "not-semver"
contract_version = "0.1.0"
"#;
        let result = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_manifest_invalid_toml_syntax_returns_error() {
        let toml_str = "this is not [valid toml";
        let result = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.code == "E0701"));
    }

    #[test]
    fn test_manifest_duplicate_component_names_returns_error() {
        let toml_str = r#"
[torvyn]
name = "my-pipeline"
version = "0.1.0"
contract_version = "0.1.0"

[[component]]
name = "same-name"
path = "a"

[[component]]
name = "same-name"
path = "b"
"#;
        let result = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.code == "E0706"));
    }

    #[test]
    fn test_manifest_defaults_applied() {
        let manifest = ComponentManifest::from_toml_str(MINIMAL_MANIFEST, "Torvyn.toml").unwrap();
        assert!(manifest.build.release);
        assert_eq!(manifest.build.target, "wasm32-wasip2");
        assert_eq!(manifest.test.timeout_seconds, 60);
    }

    #[test]
    fn test_manifest_round_trip() {
        let original = ComponentManifest::from_toml_str(MINIMAL_MANIFEST, "Torvyn.toml").unwrap();
        let serialized = original.to_toml_string().unwrap();
        let reparsed = ComponentManifest::from_toml_str(&serialized, "Torvyn.toml").unwrap();
        assert_eq!(original.torvyn.name, reparsed.torvyn.name);
        assert_eq!(original.torvyn.version, reparsed.torvyn.version);
        assert_eq!(
            original.torvyn.contract_version,
            reparsed.torvyn.contract_version
        );
    }

    #[test]
    fn test_manifest_unsupported_language_returns_error() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[[component]]
name = "comp"
path = "src"
language = "java"
"#;
        let result = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_valid_name() {
        assert!(is_valid_name("my-component"));
        assert!(is_valid_name("my_component_2"));
        assert!(is_valid_name("A"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("my component"));
        assert!(!is_valid_name("my.component"));
        assert!(!is_valid_name("hello!"));
    }

    #[test]
    fn test_is_valid_semver() {
        assert!(is_valid_semver("0.1.0"));
        assert!(is_valid_semver("1.23.456"));
        assert!(!is_valid_semver("0.1"));
        assert!(!is_valid_semver("abc"));
        assert!(!is_valid_semver("1.2.3.4"));
        assert!(!is_valid_semver("1.2.x"));
    }
}
