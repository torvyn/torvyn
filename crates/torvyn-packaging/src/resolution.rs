//! Component resolution: file -> local build -> cache -> registry.
//!
//! Per HLI Doc 08, Section 8.2.

use std::path::{Path, PathBuf};

use crate::cache::ArtifactCache;
use crate::error::ResolutionError;
use crate::oci::OciReference;

// ---------------------------------------------------------------------------
// ResolvedArtifact
// ---------------------------------------------------------------------------

/// A resolved artifact with its source.
#[derive(Debug)]
pub struct ResolvedArtifact {
    /// Where the artifact was resolved from.
    pub source: ResolutionSource,

    /// Path to the artifact on the local filesystem.
    pub path: PathBuf,
}

/// Where a resolved artifact came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionSource {
    /// Resolved from an explicit file path.
    FilePath,
    /// Resolved from the project's local build output.
    LocalBuild,
    /// Resolved from the local cache.
    Cache,
    /// Resolved from a remote OCI registry (and now cached).
    Registry,
}

// ---------------------------------------------------------------------------
// resolve()
// ---------------------------------------------------------------------------

/// Resolve a component reference to a local path.
///
/// Resolution precedence (per HLI Doc 08, Section 8.2):
/// 1. Explicit file paths (`file://...`)
/// 2. Local build output (`target/torvyn/`)
/// 3. Local cache (`~/.torvyn/cache/`)
/// 4. Remote registry (not implemented in Phase 0)
///
/// # Arguments
/// - `reference`: The component reference string.
/// - `project_dir`: The project root (for local build resolution).
/// - `cache`: The local artifact cache.
///
/// # Errors
/// Returns `ResolutionError::NotFound` if the component cannot be found.
///
/// COLD PATH.
pub fn resolve(
    reference: &str,
    project_dir: Option<&Path>,
    cache: &ArtifactCache,
) -> Result<ResolvedArtifact, ResolutionError> {
    // 1. Explicit file path
    if let Some(path_str) = reference.strip_prefix("file://") {
        let path = PathBuf::from(path_str);
        if path.exists() {
            return Ok(ResolvedArtifact {
                source: ResolutionSource::FilePath,
                path,
            });
        }
        return Err(ResolutionError::NotFound {
            reference: reference.to_owned(),
        });
    }

    // 2. Local build output
    if let Some(project) = project_dir {
        let build_dir = project.join("target").join("torvyn");
        if build_dir.exists() {
            // Look for a .torvyn file matching the reference name
            if let Ok(entries) = std::fs::read_dir(&build_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("torvyn") {
                        let filename = path.file_stem().unwrap_or_default().to_string_lossy();
                        // Match by component name prefix
                        if filename.starts_with(reference) {
                            return Ok(ResolvedArtifact {
                                source: ResolutionSource::LocalBuild,
                                path,
                            });
                        }
                    }
                }
            }
        }
    }

    // 3. Cache
    if let Some(oci_ref) = OciReference::parse(reference) {
        if cache.is_cached(&oci_ref) {
            return Ok(ResolvedArtifact {
                source: ResolutionSource::Cache,
                path: cache.artifact_dir(&oci_ref),
            });
        }
    }

    // 4. Registry — not implemented in Phase 0.
    // When implemented, this would call `registry_client.pull()` and
    // store the result in the cache.

    Err(ResolutionError::NotFound {
        reference: reference.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheConfig;
    use tempfile::TempDir;

    #[test]
    fn resolve_file_path_existing() {
        let dir = TempDir::new().unwrap();
        let artifact = dir.path().join("test.torvyn");
        std::fs::write(&artifact, b"fake artifact").unwrap();

        let cache = ArtifactCache::new(CacheConfig {
            root: dir.path().join("cache"),
        });

        let result = resolve(&format!("file://{}", artifact.display()), None, &cache).unwrap();

        assert_eq!(result.source, ResolutionSource::FilePath);
        assert_eq!(result.path, artifact);
    }

    #[test]
    fn resolve_file_path_missing() {
        let dir = TempDir::new().unwrap();
        let cache = ArtifactCache::new(CacheConfig {
            root: dir.path().join("cache"),
        });

        let result = resolve("file:///nonexistent.torvyn", None, &cache);
        assert!(matches!(result, Err(ResolutionError::NotFound { .. })));
    }

    #[test]
    fn resolve_local_build_output() {
        let dir = TempDir::new().unwrap();
        let build_dir = dir.path().join("target").join("torvyn");
        std::fs::create_dir_all(&build_dir).unwrap();
        std::fs::write(build_dir.join("my-component-0.1.0.torvyn"), b"fake").unwrap();

        let cache = ArtifactCache::new(CacheConfig {
            root: dir.path().join("cache"),
        });

        let result = resolve("my-component", Some(dir.path()), &cache).unwrap();
        assert_eq!(result.source, ResolutionSource::LocalBuild);
    }

    #[test]
    fn resolve_from_cache() {
        let dir = TempDir::new().unwrap();
        let cache = ArtifactCache::new(CacheConfig {
            root: dir.path().join("cache"),
        });

        // Pre-populate cache
        let reference = OciReference::parse("ghcr.io/org/comp:1.0.0").unwrap();
        let contents = crate::artifact::ArtifactContents {
            manifest: crate::manifest::ArtifactManifest::new("comp".into(), "1.0.0".into()),
            wasm_bytes: b"\0asm\x01\x00\x00\x00".to_vec(),
            wit_files: std::collections::BTreeMap::new(),
            provenance: None,
            layer_digests: std::collections::BTreeMap::new(),
        };
        let digest = crate::digest::ContentDigest::of_bytes(b"test");
        cache.store(&reference, &contents, &digest).unwrap();

        let result = resolve("ghcr.io/org/comp:1.0.0", None, &cache).unwrap();
        assert_eq!(result.source, ResolutionSource::Cache);
    }

    #[test]
    fn resolve_not_found() {
        let dir = TempDir::new().unwrap();
        let cache = ArtifactCache::new(CacheConfig {
            root: dir.path().join("cache"),
        });

        let result = resolve("nonexistent-component", None, &cache);
        assert!(matches!(result, Err(ResolutionError::NotFound { .. })));
    }
}
