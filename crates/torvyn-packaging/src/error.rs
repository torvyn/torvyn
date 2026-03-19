//! Packaging-specific error types with actionable diagnostics.
//!
//! These errors carry full context (file paths, field names, version strings)
//! and remediation advice. The top-level `PackagingError` in `torvyn-types`
//! provides a coarser summary for cross-crate consumption; these types are
//! the authoritative source within the packaging crate.

use std::path::PathBuf;
use thiserror::Error;

// ---------------------------------------------------------------------------
// ManifestError
// ---------------------------------------------------------------------------

/// Errors encountered while parsing or validating a component manifest
/// for packaging purposes.
///
/// # Invariants
/// - Every variant includes the file path or field that caused the error.
/// - Error messages include remediation advice.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// TOML parsing failed.
    #[error(
        "Failed to parse manifest at '{}': {detail}{}",
        path.display(),
        line.map(|l| format!(" (line {l})")).unwrap_or_default()
    )]
    ParseFailed {
        /// Path to the manifest file.
        path: PathBuf,
        /// Detail message from the TOML parser.
        detail: String,
        /// Line number where the error occurred.
        line: Option<usize>,
    },

    /// A required field is missing from the manifest.
    #[error(
        "Missing required field '{field}' in manifest at '{}'. \
         Add this field to your Torvyn.toml [component] section.",
        path.display()
    )]
    MissingField {
        /// Path to the manifest file.
        path: PathBuf,
        /// The missing field name.
        field: String,
    },

    /// A version string is not valid semver.
    #[error(
        "Invalid version string '{value}' in field '{field}' of '{}': {reason}. \
         Use semver format: MAJOR.MINOR.PATCH (e.g. 1.2.3).",
        path.display()
    )]
    InvalidVersion {
        /// Path to the manifest file.
        path: PathBuf,
        /// The field containing the bad version.
        field: String,
        /// The invalid version string.
        value: String,
        /// Why the version is invalid.
        reason: String,
    },

    /// Contract version in manifest does not match the Wasm binary.
    #[error(
        "Contract version mismatch: manifest declares '{manifest_version}' \
         but the Wasm binary was compiled against '{binary_version}'. \
         Re-build the component or update the manifest version."
    )]
    ContractMismatch {
        /// Version declared in the manifest.
        manifest_version: String,
        /// Version found in the Wasm binary.
        binary_version: String,
    },

    /// The component name contains invalid characters.
    #[error(
        "Invalid component name '{name}': must contain only [a-zA-Z0-9_-]. \
         Rename the component in your Torvyn.toml [component] section."
    )]
    InvalidComponentName {
        /// The invalid component name.
        name: String,
    },
}

// ---------------------------------------------------------------------------
// ArtifactError
// ---------------------------------------------------------------------------

/// Errors encountered during artifact assembly or extraction.
///
/// # Invariants
/// - Every variant includes the path to the artifact or source file.
#[derive(Debug, Error)]
pub enum ArtifactError {
    /// The Wasm binary file was not found.
    #[error(
        "Wasm binary not found at '{}'. \
         Run `torvyn build` before `torvyn pack`, or set the wasm-path in Torvyn.toml.",
        path.display()
    )]
    WasmBinaryNotFound {
        /// Path where the Wasm file was expected.
        path: PathBuf,
    },

    /// The Wasm binary is not a valid Component Model binary.
    #[error(
        "File at '{}' is not a valid WebAssembly Component Model binary: {reason}. \
         Ensure you are compiling with `cargo-component` or `wasm-tools component new`.",
        path.display()
    )]
    InvalidWasmBinary {
        /// Path to the invalid Wasm file.
        path: PathBuf,
        /// Why the file is invalid.
        reason: String,
    },

    /// No WIT files found in the expected directory.
    #[error(
        "No .wit files found in '{}'. \
         Ensure your WIT definitions are in the project's wit/ directory.",
        path.display()
    )]
    NoWitFiles {
        /// Path to the WIT directory.
        path: PathBuf,
    },

    /// I/O error during artifact creation or extraction.
    #[error("I/O error during artifact operation on '{}': {source}", path.display())]
    Io {
        /// Path involved in the I/O operation.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// The artifact archive is corrupted or has an unexpected structure.
    #[error(
        "Artifact at '{}' is corrupted or has an invalid structure: {reason}. \
         Re-pack with `torvyn pack` to create a valid artifact.",
        path.display()
    )]
    CorruptedArtifact {
        /// Path to the corrupted artifact.
        path: PathBuf,
        /// Why the artifact is considered corrupted.
        reason: String,
    },

    /// A layer digest does not match the expected value.
    #[error(
        "Digest mismatch for layer '{layer_name}': expected {expected}, got {actual}. \
         The artifact may have been tampered with or corrupted during transfer."
    )]
    DigestMismatch {
        /// Name of the layer with the mismatch.
        layer_name: String,
        /// Expected digest.
        expected: String,
        /// Actual digest.
        actual: String,
    },
}

// ---------------------------------------------------------------------------
// OciError
// ---------------------------------------------------------------------------

/// Errors related to OCI registry operations.
#[derive(Debug, Error)]
pub enum OciError {
    /// Authentication with the registry failed.
    #[error(
        "Registry authentication failed for '{registry}': {detail}. \
         Check your credentials in ~/.torvyn/credentials.json or set \
         TORVYN_REGISTRY_TOKEN environment variable."
    )]
    AuthFailed {
        /// Registry hostname.
        registry: String,
        /// Detail message.
        detail: String,
    },

    /// The requested artifact was not found in the registry.
    #[error(
        "Artifact not found: '{reference}'. \
         Verify the component name, version tag, and registry URL."
    )]
    NotFound {
        /// The OCI reference that was not found.
        reference: String,
    },

    /// The registry rejected the push operation.
    #[error(
        "Registry '{registry}' rejected push: {detail}. \
         The registry may not support custom OCI media types. \
         Try a registry that supports OCI artifacts (e.g., ghcr.io, ECR, ACR)."
    )]
    PushRejected {
        /// Registry hostname.
        registry: String,
        /// Detail message.
        detail: String,
    },

    /// Network communication error.
    #[error(
        "Network error communicating with '{registry}': {detail}. \
         Check your network connection and the registry URL."
    )]
    Network {
        /// Registry hostname.
        registry: String,
        /// Detail message.
        detail: String,
    },

    /// Digest mismatch after pull.
    #[error(
        "Digest mismatch for layer '{layer}' from '{registry}': \
         expected {expected}, got {actual}. \
         The layer may have been tampered with or corrupted during transfer."
    )]
    DigestMismatch {
        /// Registry hostname.
        registry: String,
        /// Layer name.
        layer: String,
        /// Expected digest.
        expected: String,
        /// Actual digest.
        actual: String,
    },

    /// The OCI registry feature is not enabled.
    #[error(
        "OCI registry operations require the 'oci-registry' feature. \
         Rebuild with `cargo build --features oci-registry`."
    )]
    FeatureNotEnabled,
}

// ---------------------------------------------------------------------------
// SigningError
// ---------------------------------------------------------------------------

/// Errors related to signing and signature verification.
#[derive(Debug, Error)]
pub enum SigningError {
    /// Signature verification failed.
    #[error(
        "Signature verification failed for artifact '{artifact}': {reason}. \
         The artifact may have been tampered with, or the signing key has changed."
    )]
    VerificationFailed {
        /// Artifact whose signature failed verification.
        artifact: String,
        /// Reason for the verification failure.
        reason: String,
    },

    /// No signature was found but one was required.
    #[error(
        "No signature found for artifact '{reference}' and --require-signature is set. \
         Ask the publisher to sign the artifact, or pass --skip-verify to proceed."
    )]
    UnsignedArtifact {
        /// The OCI reference for the unsigned artifact.
        reference: String,
    },

    /// The signing provider is not available.
    #[error(
        "Signing provider '{provider}' is not available: {reason}. \
         Set TORVYN_SIGNING_KEY for key-based signing, or enable the 'sigstore' feature."
    )]
    ProviderUnavailable {
        /// Name of the unavailable provider.
        provider: String,
        /// Reason the provider is unavailable.
        reason: String,
    },

    /// The signing key file was not found.
    #[error(
        "Signing key not found at '{}'. \
         Generate a key with `torvyn key generate` or set TORVYN_SIGNING_KEY \
         to the correct path.",
        path.display()
    )]
    KeyNotFound {
        /// Path where the key was expected.
        path: PathBuf,
    },
}

// ---------------------------------------------------------------------------
// CacheError
// ---------------------------------------------------------------------------

/// Errors related to local cache operations.
#[derive(Debug, Error)]
pub enum CacheError {
    /// I/O error in the cache directory.
    #[error("Cache I/O error at '{}': {source}", path.display())]
    Io {
        /// Path involved in the I/O operation.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// The cache index is corrupted.
    #[error(
        "Cache index is corrupted: {reason}. \
         Run `torvyn cache clean --all` to reset the cache."
    )]
    CorruptedIndex {
        /// Why the cache index is corrupted.
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// ResolutionError
// ---------------------------------------------------------------------------

/// Errors related to component resolution.
#[derive(Debug, Error)]
pub enum ResolutionError {
    /// The component could not be found in any source.
    #[error(
        "Component '{reference}' not found in local build output, cache, or registry. \
         Run `torvyn build` for local components or `torvyn pull` for remote components."
    )]
    NotFound {
        /// The reference that could not be resolved.
        reference: String,
    },

    /// A resolution source was unavailable (e.g., network down for registry).
    #[error("Resolution source '{source_name}' unavailable: {reason}")]
    SourceUnavailable {
        /// Name of the unavailable source.
        source_name: String,
        /// Reason the source is unavailable.
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// PackagingDetailError — aggregate
// ---------------------------------------------------------------------------

/// Detailed packaging error type used within this crate.
///
/// This carries full diagnostic context. Convert to `torvyn_types::PackagingError`
/// at the crate boundary for cross-crate propagation.
#[derive(Debug, Error)]
pub enum PackagingDetailError {
    /// Manifest error.
    #[error(transparent)]
    Manifest(#[from] ManifestError),

    /// Artifact error.
    #[error(transparent)]
    Artifact(#[from] ArtifactError),

    /// OCI error.
    #[error(transparent)]
    Oci(#[from] OciError),

    /// Signing error.
    #[error(transparent)]
    Signing(#[from] SigningError),

    /// Cache error.
    #[error(transparent)]
    Cache(#[from] CacheError),

    /// Resolution error.
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
}

impl From<PackagingDetailError> for torvyn_types::PackagingError {
    /// COLD PATH — called at crate boundary during error propagation.
    fn from(err: PackagingDetailError) -> Self {
        match err {
            PackagingDetailError::Manifest(e) => torvyn_types::PackagingError::InvalidArtifact {
                path: format!("{e}"),
                reason: e.to_string(),
            },
            PackagingDetailError::Artifact(e) => torvyn_types::PackagingError::InvalidArtifact {
                path: format!("{e}"),
                reason: e.to_string(),
            },
            PackagingDetailError::Oci(e) => torvyn_types::PackagingError::RegistryError {
                registry: String::new(),
                reason: e.to_string(),
            },
            PackagingDetailError::Signing(e) => torvyn_types::PackagingError::SignatureInvalid {
                artifact: String::new(),
                reason: e.to_string(),
            },
            PackagingDetailError::Cache(e) => torvyn_types::PackagingError::InvalidArtifact {
                path: String::new(),
                reason: e.to_string(),
            },
            PackagingDetailError::Resolution(e) => torvyn_types::PackagingError::MissingMetadata {
                artifact: String::new(),
                field: e.to_string(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_error_parse_failed_includes_path_and_line() {
        let err = ManifestError::ParseFailed {
            path: PathBuf::from("/app/Torvyn.toml"),
            detail: "unexpected key".into(),
            line: Some(42),
        };
        let msg = err.to_string();
        assert!(msg.contains("/app/Torvyn.toml"), "should include path");
        assert!(msg.contains("line 42"), "should include line number");
        assert!(msg.contains("unexpected key"), "should include detail");
    }

    #[test]
    fn manifest_error_parse_failed_no_line() {
        let err = ManifestError::ParseFailed {
            path: PathBuf::from("Torvyn.toml"),
            detail: "bad utf-8".into(),
            line: None,
        };
        let msg = err.to_string();
        assert!(!msg.contains("line"), "should not include line when None");
    }

    #[test]
    fn manifest_error_missing_field_includes_advice() {
        let err = ManifestError::MissingField {
            path: PathBuf::from("Torvyn.toml"),
            field: "version".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("version"), "should include field name");
        assert!(msg.contains("Add this field"), "should include advice");
    }

    #[test]
    fn manifest_error_invalid_version_includes_advice() {
        let err = ManifestError::InvalidVersion {
            path: PathBuf::from("Torvyn.toml"),
            field: "version".into(),
            value: "abc".into(),
            reason: "not a number".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("abc"), "should include bad value");
        assert!(
            msg.contains("MAJOR.MINOR.PATCH"),
            "should include format advice"
        );
    }

    #[test]
    fn artifact_error_wasm_not_found_includes_build_advice() {
        let err = ArtifactError::WasmBinaryNotFound {
            path: PathBuf::from("target/wasm32-wasip2/release/component.wasm"),
        };
        let msg = err.to_string();
        assert!(msg.contains("torvyn build"), "should suggest build command");
    }

    #[test]
    fn oci_error_auth_failed_includes_credential_advice() {
        let err = OciError::AuthFailed {
            registry: "ghcr.io".into(),
            detail: "401 Unauthorized".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("credentials"), "should mention credentials");
        assert!(
            msg.contains("TORVYN_REGISTRY_TOKEN"),
            "should mention env var"
        );
    }

    #[test]
    fn signing_error_unsigned_includes_skip_advice() {
        let err = SigningError::UnsignedArtifact {
            reference: "ghcr.io/org/comp:1.0.0".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("--skip-verify"), "should mention skip flag");
    }

    #[test]
    fn packaging_detail_error_converts_to_torvyn_types() {
        let detail = PackagingDetailError::Artifact(ArtifactError::WasmBinaryNotFound {
            path: PathBuf::from("/missing.wasm"),
        });
        let top_level: torvyn_types::PackagingError = detail.into();
        // Should not panic — conversion succeeds.
        let _ = top_level.to_string();
    }
}
