//! OCI media type constants for Torvyn artifacts.
//!
//! Per HLI Doc 08, Section 2.5, Torvyn uses vendor-specific media types
//! with the `vnd.torvyn` prefix to distinguish Torvyn components from
//! generic Wasm binaries and container images.
//!
//! All constants follow IANA conventions for vendor-specific types.
//! The `v1` in each type allows future format evolution.

/// OCI config object media type.
/// Contains machine-readable metadata summary (name, version, capabilities).
pub const CONFIG: &str = "application/vnd.torvyn.component.config.v1+json";

/// Compiled WebAssembly Component Model binary.
pub const WASM_LAYER: &str = "application/vnd.torvyn.component.wasm.v1+wasm";

/// Component manifest (`Torvyn.toml`).
pub const MANIFEST_LAYER: &str = "application/vnd.torvyn.component.manifest.v1+toml";

/// Tar archive of WIT definition files.
pub const WIT_LAYER: &str = "application/vnd.torvyn.component.wit.v1+tar";

/// SLSA-compatible build provenance record.
pub const PROVENANCE_LAYER: &str = "application/vnd.torvyn.component.provenance.v1+json";

/// Detached signature over the artifact digest.
pub const SIGNATURE: &str = "application/vnd.torvyn.component.signature.v1+json";

/// Standard OCI image manifest media type.
pub const OCI_IMAGE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";

/// Returns the human-readable name for a given media type, or `"unknown"`.
///
/// # Examples
/// ```
/// use torvyn_packaging::media_types;
/// assert_eq!(media_types::display_name(media_types::WASM_LAYER), "Wasm binary");
/// ```
///
/// COLD PATH — called during inspection output formatting.
pub fn display_name(media_type: &str) -> &'static str {
    match media_type {
        CONFIG => "OCI config",
        WASM_LAYER => "Wasm binary",
        MANIFEST_LAYER => "Torvyn manifest",
        WIT_LAYER => "WIT definitions",
        PROVENANCE_LAYER => "Provenance record",
        SIGNATURE => "Signature",
        OCI_IMAGE_MANIFEST => "OCI image manifest",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_media_types_have_vnd_prefix() {
        let types = [
            CONFIG,
            WASM_LAYER,
            MANIFEST_LAYER,
            WIT_LAYER,
            PROVENANCE_LAYER,
            SIGNATURE,
        ];
        for mt in types {
            assert!(
                mt.starts_with("application/vnd.torvyn."),
                "media type '{mt}' should have vnd.torvyn prefix"
            );
        }
    }

    #[test]
    fn all_media_types_contain_v1() {
        let types = [
            CONFIG,
            WASM_LAYER,
            MANIFEST_LAYER,
            WIT_LAYER,
            PROVENANCE_LAYER,
            SIGNATURE,
        ];
        for mt in types {
            assert!(
                mt.contains(".v1"),
                "media type '{mt}' should contain version marker"
            );
        }
    }

    #[test]
    fn display_name_returns_known_names() {
        assert_eq!(display_name(WASM_LAYER), "Wasm binary");
        assert_eq!(display_name(CONFIG), "OCI config");
        assert_eq!(display_name(PROVENANCE_LAYER), "Provenance record");
    }

    #[test]
    fn display_name_returns_unknown_for_unrecognized() {
        assert_eq!(display_name("application/octet-stream"), "unknown");
    }
}
