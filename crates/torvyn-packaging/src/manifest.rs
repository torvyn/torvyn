//! Artifact manifest — packaging-layer metadata extensions.
//!
//! The `ArtifactManifest` combines build-time metadata (from `torvyn-config`)
//! with packaging-specific fields (contracts, capabilities, compatibility,
//! distribution, deprecation). This is the complete metadata model for a
//! Torvyn artifact as stored in `Torvyn.toml` inside a `.torvyn` archive.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::ManifestError;

// ---------------------------------------------------------------------------
// WitPackageRef
// ---------------------------------------------------------------------------

/// Reference to a WIT package with namespace, name, and version.
///
/// # Invariants
/// - `namespace` and `name` are non-empty.
/// - The combined string form is `{namespace}:{name}@{version}`.
///
/// # Examples
/// ```
/// use torvyn_packaging::manifest::WitPackageRef;
///
/// let pkg = WitPackageRef::parse("torvyn:streaming@0.1.0").unwrap();
/// assert_eq!(pkg.namespace, "torvyn");
/// assert_eq!(pkg.name, "streaming");
/// assert_eq!(pkg.version, "0.1.0");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WitPackageRef {
    /// WIT package namespace (e.g., "torvyn").
    pub namespace: String,
    /// WIT package name (e.g., "streaming").
    pub name: String,
    /// WIT package version (e.g., "0.1.0").
    pub version: String,
}

impl WitPackageRef {
    /// Parse a WIT package reference from the canonical string form.
    ///
    /// Format: `{namespace}:{name}@{version}`
    ///
    /// # Errors
    /// Returns `None` if the format is invalid.
    ///
    /// COLD PATH — called during manifest parsing.
    pub fn parse(s: &str) -> Option<Self> {
        let (ns_name, version) = s.rsplit_once('@')?;
        let (namespace, name) = ns_name.split_once(':')?;
        if namespace.is_empty() || name.is_empty() || version.is_empty() {
            return None;
        }
        Some(Self {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
        })
    }

    /// Format as the canonical string `{namespace}:{name}@{version}`.
    pub fn to_canonical(&self) -> String {
        format!("{}:{}@{}", self.namespace, self.name, self.version)
    }
}

impl std::fmt::Display for WitPackageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}@{}", self.namespace, self.name, self.version)
    }
}

// ---------------------------------------------------------------------------
// CompatibilitySpec
// ---------------------------------------------------------------------------

/// Runtime and platform compatibility requirements.
///
/// Per HLI Doc 08, Section 9.1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilitySpec {
    /// Minimum Torvyn runtime version required (semver).
    #[serde(rename = "min-torvyn-version")]
    pub min_torvyn_version: String,

    /// Maximum Torvyn runtime version allowed (semver). Optional.
    #[serde(
        rename = "max-torvyn-version",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_torvyn_version: Option<String>,

    /// WASI target: "preview2" or "preview3" (future).
    #[serde(rename = "wasi-target", default = "default_wasi_target")]
    pub wasi_target: String,

    /// Target architecture. Currently always "wasm32".
    #[serde(rename = "target-arch", default = "default_target_arch")]
    pub target_arch: String,

    /// Required WASI interface features.
    #[serde(rename = "wasi-features", default)]
    pub wasi_features: Vec<String>,

    /// Required Torvyn host interface features.
    #[serde(rename = "torvyn-features", default)]
    pub torvyn_features: Vec<String>,
}

fn default_wasi_target() -> String {
    "preview2".to_owned()
}

fn default_target_arch() -> String {
    "wasm32".to_owned()
}

impl Default for CompatibilitySpec {
    fn default() -> Self {
        Self {
            min_torvyn_version: "0.1.0".to_owned(),
            max_torvyn_version: None,
            wasi_target: default_wasi_target(),
            target_arch: default_target_arch(),
            wasi_features: Vec::new(),
            torvyn_features: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// CapabilitiesSpec
// ---------------------------------------------------------------------------

/// Capability requirements from the manifest.
///
/// Per HLI Doc 08, Section 11.1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitiesSpec {
    /// Required WASI capabilities (must be granted).
    #[serde(default)]
    pub required: BTreeMap<String, bool>,

    /// Optional WASI capabilities (used if granted).
    #[serde(default)]
    pub optional: BTreeMap<String, bool>,

    /// Torvyn-specific resource constraints.
    #[serde(default)]
    pub torvyn: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// DistributionSpec
// ---------------------------------------------------------------------------

/// Distribution-related metadata.
///
/// Per HLI Doc 08, Section 11.1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionSpec {
    /// Default OCI registry for `torvyn push`.
    #[serde(default)]
    pub registry: String,

    /// Search categories (e.g., "transform", "source").
    #[serde(default)]
    pub categories: Vec<String>,

    /// Free-form search keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
}

// ---------------------------------------------------------------------------
// DeprecationSpec
// ---------------------------------------------------------------------------

/// Deprecation notice for published artifacts.
///
/// Per HLI Doc 08, Section 10.2.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeprecationSpec {
    /// Version since when this component is deprecated.
    #[serde(rename = "deprecated-since")]
    pub deprecated_since: String,

    /// Human-readable deprecation message with migration guidance.
    pub message: String,

    /// Successor component name, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor: Option<String>,
}

// ---------------------------------------------------------------------------
// BuildInfoSpec
// ---------------------------------------------------------------------------

/// Build tool metadata recorded at pack time.
///
/// Per HLI Doc 08, Section 11.1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfoSpec {
    /// Build tool name (e.g., "cargo-component").
    #[serde(default)]
    pub tool: String,

    /// Build tool version.
    #[serde(rename = "tool-version", default)]
    pub tool_version: String,

    /// Language version (e.g., Rust version).
    #[serde(
        rename = "language-version",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub language_version: Option<String>,

    /// Build profile (e.g., "release").
    #[serde(default)]
    pub profile: String,
}

// ---------------------------------------------------------------------------
// ConfigSchemaSpec
// ---------------------------------------------------------------------------

/// Optional configuration schema for the component.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSchemaSpec {
    /// Format of the config blob (e.g., "json").
    #[serde(default)]
    pub format: String,

    /// Example configuration string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

// ---------------------------------------------------------------------------
// ArtifactManifest
// ---------------------------------------------------------------------------

/// The complete packaging-layer manifest for a Torvyn artifact.
///
/// This is the authoritative schema for the `Torvyn.toml` as it exists
/// inside a packaged `.torvyn` archive. It extends the build-time
/// `ComponentManifest` from `torvyn-config` with packaging fields.
///
/// # Invariants
/// - `name` is non-empty and matches `[a-zA-Z0-9_-]+`.
/// - `version` is valid semver.
/// - `contract_packages` is non-empty (at least one WIT package).
///
/// # Examples
/// ```
/// use torvyn_packaging::manifest::ArtifactManifest;
///
/// let toml_str = r#"
/// [component]
/// name = "my-transform"
/// version = "1.2.0"
/// description = "Transforms JSON"
/// license = "Apache-2.0"
///
/// [contracts]
/// packages = ["torvyn:streaming@0.1.0"]
///
/// [compatibility]
/// min-torvyn-version = "0.3.0"
/// "#;
///
/// let manifest = ArtifactManifest::from_toml_str(toml_str).unwrap();
/// assert_eq!(manifest.name(), "my-transform");
/// assert_eq!(manifest.version(), "1.2.0");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    /// Component identity section.
    #[serde(rename = "component")]
    component_section: ComponentSection,

    /// Contract packages this component was compiled against.
    #[serde(rename = "contracts", default)]
    contracts_section: ContractsSection,

    /// Capability requirements.
    #[serde(default)]
    pub capabilities: CapabilitiesSpec,

    /// Runtime compatibility requirements.
    #[serde(default)]
    pub compatibility: CompatibilitySpec,

    /// Build metadata.
    #[serde(default, rename = "build")]
    pub build_info: BuildInfoSpec,

    /// Distribution metadata.
    #[serde(default)]
    pub distribution: DistributionSpec,

    /// Deprecation notice (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation: Option<DeprecationSpec>,

    /// Configuration schema (optional).
    #[serde(
        default,
        rename = "config-schema",
        skip_serializing_if = "Option::is_none"
    )]
    pub config_schema: Option<ConfigSchemaSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ComponentSection {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    authors: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ContractsSection {
    #[serde(default)]
    packages: Vec<String>,
}

// Convenience accessor impls
impl ArtifactManifest {
    // --- Accessors (delegate to internal sections) ---

    /// Returns the component name.
    pub fn name(&self) -> &str {
        &self.component_section.name
    }

    /// Returns the component version.
    pub fn version(&self) -> &str {
        &self.component_section.version
    }

    /// Returns the component description.
    pub fn description(&self) -> &str {
        &self.component_section.description
    }

    /// Returns the license identifier.
    pub fn license(&self) -> &str {
        &self.component_section.license
    }

    /// Returns the component homepage URL.
    pub fn homepage(&self) -> &str {
        &self.component_section.homepage
    }

    /// Returns the source repository URL.
    pub fn repository(&self) -> &str {
        &self.component_section.repository
    }

    /// Returns the list of authors.
    pub fn authors(&self) -> &[String] {
        &self.component_section.authors
    }

    /// Returns parsed WIT package references for the contract packages.
    pub fn contract_packages(&self) -> Vec<WitPackageRef> {
        self.contracts_section
            .packages
            .iter()
            .filter_map(|s| WitPackageRef::parse(s))
            .collect()
    }

    /// Returns the raw contract package strings.
    pub fn contract_package_strings(&self) -> &[String] {
        &self.contracts_section.packages
    }

    // --- Parsing ---

    /// Parse an `ArtifactManifest` from a TOML string.
    ///
    /// # Errors
    /// Returns `ManifestError` if the TOML is invalid or required fields
    /// are missing.
    ///
    /// COLD PATH — called during pack and inspect operations.
    pub fn from_toml_str(s: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(s).map_err(|e| ManifestError::ParseFailed {
            path: "Torvyn.toml".into(),
            detail: e.message().to_owned(),
            line: e
                .span()
                .map(|span| s[..span.start].chars().filter(|c| *c == '\n').count() + 1),
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Parse an `ArtifactManifest` from a TOML file on disk.
    ///
    /// # Errors
    /// Returns `ManifestError` if the file cannot be read or is invalid.
    ///
    /// COLD PATH.
    pub fn from_file(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path).map_err(|e| ManifestError::ParseFailed {
            path: path.to_owned(),
            detail: format!("cannot read file: {e}"),
            line: None,
        })?;
        let manifest: Self = toml::from_str(&content).map_err(|e| ManifestError::ParseFailed {
            path: path.to_owned(),
            detail: e.message().to_owned(),
            line: e
                .span()
                .map(|span| content[..span.start].chars().filter(|c| *c == '\n').count() + 1),
        })?;
        manifest.validate_with_path(path)?;
        Ok(manifest)
    }

    /// Serialize this manifest to a TOML string.
    ///
    /// # Errors
    /// Returns `ManifestError` if serialization fails.
    ///
    /// COLD PATH.
    pub fn to_toml_string(&self) -> Result<String, ManifestError> {
        toml::to_string_pretty(self).map_err(|e| ManifestError::ParseFailed {
            path: "Torvyn.toml".into(),
            detail: format!("serialization failed: {e}"),
            line: None,
        })
    }

    // --- Validation ---

    fn validate(&self) -> Result<(), ManifestError> {
        self.validate_with_path(Path::new("Torvyn.toml"))
    }

    fn validate_with_path(&self, path: &Path) -> Result<(), ManifestError> {
        // Validate component name
        if self.component_section.name.is_empty() {
            return Err(ManifestError::MissingField {
                path: path.to_owned(),
                field: "component.name".into(),
            });
        }
        if !self
            .component_section
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ManifestError::InvalidComponentName {
                name: self.component_section.name.clone(),
            });
        }

        // Validate version is valid semver
        if self.component_section.version.is_empty() {
            return Err(ManifestError::MissingField {
                path: path.to_owned(),
                field: "component.version".into(),
            });
        }
        if semver::Version::parse(&self.component_section.version).is_err() {
            return Err(ManifestError::InvalidVersion {
                path: path.to_owned(),
                field: "component.version".into(),
                value: self.component_section.version.clone(),
                reason: "not a valid semver string".into(),
            });
        }

        // Validate min-torvyn-version is valid semver
        if semver::Version::parse(&self.compatibility.min_torvyn_version).is_err() {
            return Err(ManifestError::InvalidVersion {
                path: path.to_owned(),
                field: "compatibility.min-torvyn-version".into(),
                value: self.compatibility.min_torvyn_version.clone(),
                reason: "not a valid semver string".into(),
            });
        }

        // Validate contract package references parse correctly
        for pkg_str in &self.contracts_section.packages {
            if WitPackageRef::parse(pkg_str).is_none() {
                return Err(ManifestError::InvalidVersion {
                    path: path.to_owned(),
                    field: "contracts.packages".into(),
                    value: pkg_str.clone(),
                    reason: "expected format: namespace:name@version".into(),
                });
            }
        }

        Ok(())
    }

    // --- Builder (for programmatic construction during pack) ---

    /// Create a new `ArtifactManifest` with required fields.
    ///
    /// COLD PATH.
    pub fn new(name: String, version: String) -> Self {
        Self {
            component_section: ComponentSection {
                name,
                version,
                description: String::new(),
                license: String::new(),
                homepage: String::new(),
                repository: String::new(),
                authors: Vec::new(),
            },
            contracts_section: ContractsSection::default(),
            capabilities: CapabilitiesSpec::default(),
            compatibility: CompatibilitySpec::default(),
            build_info: BuildInfoSpec::default(),
            distribution: DistributionSpec::default(),
            deprecation: None,
            config_schema: None,
        }
    }

    /// Set the contract packages.
    pub fn with_contracts(mut self, packages: Vec<String>) -> Self {
        self.contracts_section.packages = packages;
        self
    }

    /// Set the description.
    pub fn with_description(mut self, desc: String) -> Self {
        self.component_section.description = desc;
        self
    }

    /// Set the license.
    pub fn with_license(mut self, license: String) -> Self {
        self.component_section.license = license;
        self
    }

    /// Set the compatibility spec.
    pub fn with_compatibility(mut self, compat: CompatibilitySpec) -> Self {
        self.compatibility = compat;
        self
    }

    /// Set the build info.
    pub fn with_build_info(mut self, build: BuildInfoSpec) -> Self {
        self.build_info = build;
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOML: &str = r#"
[component]
name = "my-transform"
version = "1.2.0"
description = "Transforms JSON payloads"
license = "Apache-2.0"

[contracts]
packages = ["torvyn:streaming@0.1.0"]

[compatibility]
min-torvyn-version = "0.3.0"
wasi-target = "preview2"
target-arch = "wasm32"
"#;

    #[test]
    fn parse_valid_manifest() {
        let m = ArtifactManifest::from_toml_str(VALID_TOML).unwrap();
        assert_eq!(m.name(), "my-transform");
        assert_eq!(m.version(), "1.2.0");
        assert_eq!(m.description(), "Transforms JSON payloads");
        assert_eq!(m.license(), "Apache-2.0");
        assert_eq!(m.contract_packages().len(), 1);
        assert_eq!(m.contract_packages()[0].namespace, "torvyn");
        assert_eq!(m.contract_packages()[0].name, "streaming");
        assert_eq!(m.compatibility.min_torvyn_version, "0.3.0");
    }

    #[test]
    fn parse_minimal_manifest() {
        let toml_str = r#"
[component]
name = "minimal"
version = "0.1.0"
"#;
        let m = ArtifactManifest::from_toml_str(toml_str).unwrap();
        assert_eq!(m.name(), "minimal");
        assert!(m.contract_packages().is_empty());
    }

    #[test]
    fn missing_name_is_error() {
        let toml_str = r#"
[component]
version = "0.1.0"
"#;
        // serde will fail because `name` is not optional in ComponentSection
        let result = ArtifactManifest::from_toml_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_version_is_error() {
        let toml_str = r#"
[component]
name = "test"
version = "not-semver"
"#;
        let result = ArtifactManifest::from_toml_str(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("MAJOR.MINOR.PATCH"));
    }

    #[test]
    fn invalid_component_name_is_error() {
        let toml_str = r#"
[component]
name = "my component!"
version = "0.1.0"
"#;
        let result = ArtifactManifest::from_toml_str(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("[a-zA-Z0-9_-]"));
    }

    #[test]
    fn invalid_contract_package_ref_is_error() {
        let toml_str = r#"
[component]
name = "test"
version = "0.1.0"

[contracts]
packages = ["bad-format"]
"#;
        let result = ArtifactManifest::from_toml_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn round_trip_toml_serialization() {
        let original = ArtifactManifest::from_toml_str(VALID_TOML).unwrap();
        let serialized = original.to_toml_string().unwrap();
        let deserialized = ArtifactManifest::from_toml_str(&serialized).unwrap();
        assert_eq!(original.name(), deserialized.name());
        assert_eq!(original.version(), deserialized.version());
        assert_eq!(
            original.contract_package_strings(),
            deserialized.contract_package_strings()
        );
    }

    #[test]
    fn wit_package_ref_parse_valid() {
        let r = WitPackageRef::parse("torvyn:streaming@0.1.0").unwrap();
        assert_eq!(r.namespace, "torvyn");
        assert_eq!(r.name, "streaming");
        assert_eq!(r.version, "0.1.0");
    }

    #[test]
    fn wit_package_ref_parse_invalid() {
        assert!(WitPackageRef::parse("no-at-sign").is_none());
        assert!(WitPackageRef::parse(":name@1.0").is_none());
        assert!(WitPackageRef::parse("ns:@1.0").is_none());
        assert!(WitPackageRef::parse("ns:name@").is_none());
    }

    #[test]
    fn wit_package_ref_canonical_roundtrip() {
        let r = WitPackageRef::parse("torvyn:streaming@0.1.0").unwrap();
        assert_eq!(r.to_canonical(), "torvyn:streaming@0.1.0");
    }

    #[test]
    fn builder_constructs_valid_manifest() {
        let m = ArtifactManifest::new("test-comp".into(), "0.1.0".into())
            .with_contracts(vec!["torvyn:streaming@0.1.0".into()])
            .with_description("A test component".into())
            .with_license("MIT".into());
        assert_eq!(m.name(), "test-comp");
        assert_eq!(m.description(), "A test component");
        assert_eq!(m.contract_packages().len(), 1);
    }

    #[test]
    fn deprecation_section_parses() {
        let toml_str = r#"
[component]
name = "old-comp"
version = "1.5.0"

[deprecation]
deprecated-since = "1.5.0"
message = "Use v2.x"
successor = "new-comp"
"#;
        let m = ArtifactManifest::from_toml_str(toml_str).unwrap();
        let dep = m.deprecation.unwrap();
        assert_eq!(dep.deprecated_since, "1.5.0");
        assert_eq!(dep.successor.as_deref(), Some("new-comp"));
    }
}
