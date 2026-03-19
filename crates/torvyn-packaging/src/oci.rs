//! OCI manifest structures, reference parsing, and registry client trait.
//!
//! Per HLI Doc 08, Sections 2.3–3.6. The `RegistryClient` trait abstracts
//! OCI push/pull so the concrete implementation can be swapped (per MR-16).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::artifact::ArtifactContents;
use crate::digest::ContentDigest;
use crate::error::OciError;
use crate::media_types;

// ---------------------------------------------------------------------------
// OciReference
// ---------------------------------------------------------------------------

/// Parsed OCI artifact reference.
///
/// Format: `<registry>/<namespace>/<name>:<tag>` or
///         `<registry>/<namespace>/<name>@sha256:<digest>`
///
/// # Examples
/// ```
/// use torvyn_packaging::oci::OciReference;
///
/// let r = OciReference::parse("ghcr.io/torvyn-community/transforms/json-parser:1.2.0").unwrap();
/// assert_eq!(r.registry, "ghcr.io");
/// assert_eq!(r.repository, "torvyn-community/transforms/json-parser");
/// assert_eq!(r.tag.as_deref(), Some("1.2.0"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OciReference {
    /// Registry hostname (e.g., "ghcr.io", "docker.io").
    pub registry: String,

    /// Repository path (everything between registry and tag/digest).
    pub repository: String,

    /// Version tag (e.g., "1.2.0", "latest").
    pub tag: Option<String>,

    /// Content digest (e.g., "sha256:abc123...").
    pub digest: Option<String>,
}

impl OciReference {
    /// Parse an OCI reference string.
    ///
    /// Supports formats:
    /// - `registry/repo:tag`
    /// - `registry/repo@sha256:digest`
    /// - `oci://registry/repo:tag` (strips `oci://` prefix)
    ///
    /// COLD PATH.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix("oci://").unwrap_or(s);

        // Split off digest if present
        if let Some((path, digest)) = s.split_once('@') {
            let (registry, repository) = split_registry_repo(path)?;
            return Some(Self {
                registry,
                repository,
                tag: None,
                digest: Some(digest.to_owned()),
            });
        }

        // Split off tag if present
        if let Some((path, tag)) = s.rsplit_once(':') {
            // Guard against port numbers (e.g., "localhost:5000/repo")
            // If the part after : contains a /, it's a port, not a tag.
            if !tag.contains('/') {
                let (registry, repository) = split_registry_repo(path)?;
                return Some(Self {
                    registry,
                    repository,
                    tag: Some(tag.to_owned()),
                    digest: None,
                });
            }
        }

        // No tag or digest — use the full path
        let (registry, repository) = split_registry_repo(s)?;
        Some(Self {
            registry,
            repository,
            tag: None,
            digest: None,
        })
    }

    /// Format as a full reference string.
    pub fn to_string_ref(&self) -> String {
        if let Some(ref digest) = self.digest {
            format!("{}/{}@{}", self.registry, self.repository, digest)
        } else if let Some(ref tag) = self.tag {
            format!("{}/{}:{}", self.registry, self.repository, tag)
        } else {
            format!("{}/{}", self.registry, self.repository)
        }
    }
}

impl std::fmt::Display for OciReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string_ref())
    }
}

/// Split "registry/repo/path" into (registry, "repo/path").
fn split_registry_repo(s: &str) -> Option<(String, String)> {
    let idx = s.find('/')?;
    let registry = &s[..idx];
    let repo = &s[idx + 1..];
    if registry.is_empty() || repo.is_empty() {
        return None;
    }
    Some((registry.to_owned(), repo.to_owned()))
}

// ---------------------------------------------------------------------------
// OciImageManifest
// ---------------------------------------------------------------------------

/// OCI Image Manifest structure.
///
/// Per HLI Doc 08, Section 2.3.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OciImageManifest {
    /// Schema version (always 2).
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,

    /// Manifest media type.
    #[serde(rename = "mediaType")]
    pub media_type: String,

    /// OCI config descriptor.
    pub config: OciDescriptor,

    /// Layer descriptors.
    pub layers: Vec<OciDescriptor>,

    /// Annotations.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

/// OCI content descriptor (used for config and layers).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OciDescriptor {
    /// Media type of the content.
    #[serde(rename = "mediaType")]
    pub media_type: String,

    /// Content digest.
    pub digest: String,

    /// Content size in bytes.
    pub size: u64,

    /// Annotations.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// OciConfig
// ---------------------------------------------------------------------------

/// OCI config object — machine-readable metadata summary.
///
/// Per HLI Doc 08, Section 2.4.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OciConfig {
    /// Component name.
    pub component_name: String,
    /// Component version.
    pub component_version: String,
    /// Contract packages.
    pub contract_packages: Vec<String>,
    /// Required capabilities.
    pub capabilities_required: Vec<String>,
    /// Optional capabilities.
    pub capabilities_optional: Vec<String>,
    /// Runtime compatibility.
    pub runtime_compatibility: OciRuntimeCompat,
    /// Build tool name.
    pub build_tool: String,
    /// Build tool version.
    pub build_tool_version: String,
}

/// Runtime compatibility information for OCI config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OciRuntimeCompat {
    /// Minimum Torvyn version.
    pub min_torvyn_version: String,
    /// Maximum Torvyn version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_torvyn_version: Option<String>,
    /// WASI target.
    pub wasi_target: String,
    /// Target architectures.
    pub target_arch: Vec<String>,
}

impl OciImageManifest {
    /// Build an OCI image manifest from artifact contents and layer digests.
    ///
    /// COLD PATH — called during `torvyn push` preparation.
    pub fn from_artifact(
        contents: &ArtifactContents,
        layer_digests: &BTreeMap<String, ContentDigest>,
        config_digest: &ContentDigest,
        config_size: u64,
    ) -> Self {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();

        let mut annotations = BTreeMap::new();
        annotations.insert("org.opencontainers.image.created".into(), now);
        annotations.insert(
            "org.opencontainers.image.title".into(),
            contents.manifest.name().to_owned(),
        );
        annotations.insert(
            "org.opencontainers.image.version".into(),
            contents.manifest.version().to_owned(),
        );
        annotations.insert(
            "dev.torvyn.runtime.min-version".into(),
            contents.manifest.compatibility.min_torvyn_version.clone(),
        );
        if let Some(first_pkg) = contents.manifest.contract_package_strings().first() {
            annotations.insert("dev.torvyn.contract.package".into(), first_pkg.clone());
        }

        let mut layers = Vec::new();

        // Wasm layer
        if let Some(d) = layer_digests.get("component.wasm") {
            let mut ann = BTreeMap::new();
            ann.insert(
                "org.opencontainers.image.title".into(),
                "component.wasm".into(),
            );
            layers.push(OciDescriptor {
                media_type: media_types::WASM_LAYER.to_owned(),
                digest: d.prefixed.clone(),
                size: contents.wasm_bytes.len() as u64,
                annotations: ann,
            });
        }

        // Manifest layer
        if let Some(d) = layer_digests.get("Torvyn.toml") {
            let mut ann = BTreeMap::new();
            ann.insert(
                "org.opencontainers.image.title".into(),
                "Torvyn.toml".into(),
            );
            layers.push(OciDescriptor {
                media_type: media_types::MANIFEST_LAYER.to_owned(),
                digest: d.prefixed.clone(),
                size: 0, // Will be filled by caller
                annotations: ann,
            });
        }

        // Provenance layer
        if let Some(d) = layer_digests.get("provenance.json") {
            let mut ann = BTreeMap::new();
            ann.insert(
                "org.opencontainers.image.title".into(),
                "provenance.json".into(),
            );
            layers.push(OciDescriptor {
                media_type: media_types::PROVENANCE_LAYER.to_owned(),
                digest: d.prefixed.clone(),
                size: 0,
                annotations: ann,
            });
        }

        Self {
            schema_version: 2,
            media_type: media_types::OCI_IMAGE_MANIFEST.to_owned(),
            config: OciDescriptor {
                media_type: media_types::CONFIG.to_owned(),
                digest: config_digest.prefixed.clone(),
                size: config_size,
                annotations: BTreeMap::new(),
            },
            layers,
            annotations,
        }
    }
}

impl OciConfig {
    /// Build from artifact contents.
    ///
    /// COLD PATH.
    pub fn from_artifact(contents: &ArtifactContents) -> Self {
        let m = &contents.manifest;
        Self {
            component_name: m.name().to_owned(),
            component_version: m.version().to_owned(),
            contract_packages: m.contract_package_strings().to_vec(),
            capabilities_required: m.capabilities.required.keys().cloned().collect(),
            capabilities_optional: m.capabilities.optional.keys().cloned().collect(),
            runtime_compatibility: OciRuntimeCompat {
                min_torvyn_version: m.compatibility.min_torvyn_version.clone(),
                max_torvyn_version: m.compatibility.max_torvyn_version.clone(),
                wasi_target: m.compatibility.wasi_target.clone(),
                target_arch: vec![m.compatibility.target_arch.clone()],
            },
            build_tool: m.build_info.tool.clone(),
            build_tool_version: m.build_info.tool_version.clone(),
        }
    }

    /// Serialize to JSON bytes.
    ///
    /// # Errors
    /// Returns a serialization error if JSON encoding fails.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }
}

// ---------------------------------------------------------------------------
// RegistryClient trait
// ---------------------------------------------------------------------------

/// Abstraction over OCI registry operations.
///
/// Per MR-16 (Doc 10): the concrete implementation will be determined
/// after evaluating `oci-client` and `oras-rs`. This trait allows
/// switching implementations without changing the rest of the crate.
///
/// All methods are async to support network I/O.
#[async_trait::async_trait]
pub trait RegistryClient: Send + Sync {
    /// Push an artifact to the registry.
    ///
    /// # Errors
    /// Returns `OciError` on authentication, network, or push rejection failures.
    async fn push(
        &self,
        reference: &OciReference,
        manifest: &OciImageManifest,
        layers: &BTreeMap<String, Vec<u8>>,
        config: &[u8],
    ) -> Result<String, OciError>;

    /// Pull an artifact manifest from the registry.
    ///
    /// # Errors
    /// Returns `OciError` if the artifact is not found or network fails.
    async fn pull_manifest(&self, reference: &OciReference) -> Result<OciImageManifest, OciError>;

    /// Pull a single layer by digest.
    ///
    /// # Errors
    /// Returns `OciError` if the layer is not found or network fails.
    async fn pull_layer(&self, reference: &OciReference, digest: &str)
        -> Result<Vec<u8>, OciError>;

    /// Check if a layer exists in the registry (by digest).
    ///
    /// # Errors
    /// Returns `OciError` on network failures.
    async fn layer_exists(&self, reference: &OciReference, digest: &str) -> Result<bool, OciError>;
}

// ---------------------------------------------------------------------------
// StubRegistryClient (for testing without network)
// ---------------------------------------------------------------------------

/// A stub registry client that stores artifacts in memory.
///
/// Used for testing. Not for production use.
#[derive(Default)]
pub struct StubRegistryClient {
    manifests: std::sync::Mutex<BTreeMap<String, OciImageManifest>>,
    layers: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
}

#[async_trait::async_trait]
impl RegistryClient for StubRegistryClient {
    async fn push(
        &self,
        reference: &OciReference,
        manifest: &OciImageManifest,
        layers: &BTreeMap<String, Vec<u8>>,
        _config: &[u8],
    ) -> Result<String, OciError> {
        let key = reference.to_string_ref();
        self.manifests
            .lock()
            .unwrap()
            .insert(key.clone(), manifest.clone());
        for (digest, data) in layers {
            self.layers
                .lock()
                .unwrap()
                .insert(digest.clone(), data.clone());
        }
        Ok(key)
    }

    async fn pull_manifest(&self, reference: &OciReference) -> Result<OciImageManifest, OciError> {
        let key = reference.to_string_ref();
        self.manifests
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| OciError::NotFound { reference: key })
    }

    async fn pull_layer(
        &self,
        _reference: &OciReference,
        digest: &str,
    ) -> Result<Vec<u8>, OciError> {
        self.layers
            .lock()
            .unwrap()
            .get(digest)
            .cloned()
            .ok_or_else(|| OciError::NotFound {
                reference: digest.to_owned(),
            })
    }

    async fn layer_exists(
        &self,
        _reference: &OciReference,
        digest: &str,
    ) -> Result<bool, OciError> {
        Ok(self.layers.lock().unwrap().contains_key(digest))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reference_with_tag() {
        let r = OciReference::parse("ghcr.io/org/comp:1.2.0").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "org/comp");
        assert_eq!(r.tag.as_deref(), Some("1.2.0"));
        assert!(r.digest.is_none());
    }

    #[test]
    fn parse_reference_with_digest() {
        let r = OciReference::parse("ghcr.io/org/comp@sha256:abc123").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "org/comp");
        assert!(r.tag.is_none());
        assert_eq!(r.digest.as_deref(), Some("sha256:abc123"));
    }

    #[test]
    fn parse_reference_strips_oci_prefix() {
        let r = OciReference::parse("oci://ghcr.io/org/comp:1.0.0").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.tag.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_reference_deep_path() {
        let r =
            OciReference::parse("ghcr.io/torvyn-community/transforms/json-parser:1.2.0").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "torvyn-community/transforms/json-parser");
        assert_eq!(r.tag.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn parse_reference_no_tag() {
        let r = OciReference::parse("ghcr.io/org/comp").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert!(r.tag.is_none());
        assert!(r.digest.is_none());
    }

    #[test]
    fn parse_invalid_reference_returns_none() {
        assert!(OciReference::parse("no-slash").is_none());
        assert!(OciReference::parse("/no-registry").is_none());
    }

    #[test]
    fn reference_display_roundtrip() {
        let r = OciReference::parse("ghcr.io/org/comp:1.0.0").unwrap();
        assert_eq!(r.to_string(), "ghcr.io/org/comp:1.0.0");
    }

    #[test]
    fn oci_config_from_artifact_contents() {
        let manifest = crate::manifest::ArtifactManifest::new("test".into(), "0.1.0".into())
            .with_contracts(vec!["torvyn:streaming@0.1.0".into()]);

        let contents = ArtifactContents {
            manifest,
            wasm_bytes: vec![],
            wit_files: BTreeMap::new(),
            provenance: None,
            layer_digests: BTreeMap::new(),
        };

        let config = OciConfig::from_artifact(&contents);
        assert_eq!(config.component_name, "test");
        assert_eq!(config.component_version, "0.1.0");
        assert_eq!(config.contract_packages, vec!["torvyn:streaming@0.1.0"]);
    }

    #[tokio::test]
    async fn stub_registry_push_then_pull() {
        let client = StubRegistryClient::default();
        let reference = OciReference::parse("test.io/org/comp:1.0.0").unwrap();

        let manifest = OciImageManifest {
            schema_version: 2,
            media_type: media_types::OCI_IMAGE_MANIFEST.to_owned(),
            config: OciDescriptor {
                media_type: media_types::CONFIG.to_owned(),
                digest: "sha256:config".into(),
                size: 100,
                annotations: BTreeMap::new(),
            },
            layers: vec![],
            annotations: BTreeMap::new(),
        };

        let mut layers = BTreeMap::new();
        layers.insert("sha256:abc".into(), b"layer data".to_vec());

        client
            .push(&reference, &manifest, &layers, b"config")
            .await
            .unwrap();

        let pulled = client.pull_manifest(&reference).await.unwrap();
        assert_eq!(pulled.schema_version, 2);

        let layer = client.pull_layer(&reference, "sha256:abc").await.unwrap();
        assert_eq!(layer, b"layer data");
    }

    #[tokio::test]
    async fn stub_registry_pull_nonexistent_returns_error() {
        let client = StubRegistryClient::default();
        let reference = OciReference::parse("test.io/org/missing:1.0.0").unwrap();

        let result = client.pull_manifest(&reference).await;
        assert!(matches!(result, Err(OciError::NotFound { .. })));
    }
}
