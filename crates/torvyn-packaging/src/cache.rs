//! Local artifact cache management.
//!
//! Per HLI Doc 08, Section 8.3. Cache layout:
//! ```text
//! ~/.torvyn/cache/oci/<registry>/<namespace>/<name>/<version>/
//!   component.wasm
//!   Torvyn.toml
//!   wit/
//!   provenance.json
//! ```

use std::path::{Path, PathBuf};

use crate::digest::ContentDigest;
use crate::error::CacheError;
use crate::oci::OciReference;

// ---------------------------------------------------------------------------
// CacheConfig
// ---------------------------------------------------------------------------

/// Configuration for the local cache.
#[derive(Clone, Debug)]
pub struct CacheConfig {
    /// Root directory for the cache.
    pub root: PathBuf,
}

impl Default for CacheConfig {
    fn default() -> Self {
        // Default: ~/.torvyn/cache
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "/tmp".to_owned());
        Self {
            root: PathBuf::from(home).join(".torvyn").join("cache"),
        }
    }
}

// ---------------------------------------------------------------------------
// ArtifactCache
// ---------------------------------------------------------------------------

/// Manages the local artifact cache.
///
/// The cache is a directory tree that mirrors OCI registry paths.
/// Each cached artifact is stored as an extracted directory (not a `.torvyn` archive)
/// for fast access.
///
/// # Invariants
/// - The cache root directory is created on first use.
/// - Each cached artifact has a `.digest` file containing the SHA-256 digest.
pub struct ArtifactCache {
    config: CacheConfig,
}

impl ArtifactCache {
    /// Create a new cache manager.
    ///
    /// Does not create the cache directory until first write.
    ///
    /// COLD PATH.
    pub fn new(config: CacheConfig) -> Self {
        Self { config }
    }

    /// Create a cache with the default configuration.
    pub fn with_defaults() -> Self {
        Self::new(CacheConfig::default())
    }

    /// Get the cache root path.
    pub fn root(&self) -> &Path {
        &self.config.root
    }

    /// Compute the cache directory path for an OCI reference.
    ///
    /// COLD PATH.
    pub fn artifact_dir(&self, reference: &OciReference) -> PathBuf {
        let version = reference.tag.as_deref().unwrap_or("latest");
        self.config
            .root
            .join("oci")
            .join(&reference.registry)
            .join(&reference.repository)
            .join(version)
    }

    /// Check if an artifact is cached (by OCI reference).
    ///
    /// Returns `true` if the cache directory exists and contains a manifest.
    ///
    /// COLD PATH.
    pub fn is_cached(&self, reference: &OciReference) -> bool {
        let dir = self.artifact_dir(reference);
        dir.join("Torvyn.toml").exists()
    }

    /// Get the cached digest for an artifact.
    ///
    /// Returns `None` if not cached.
    ///
    /// COLD PATH.
    pub fn cached_digest(&self, reference: &OciReference) -> Option<ContentDigest> {
        let dir = self.artifact_dir(reference);
        let digest_path = dir.with_extension("digest");
        let content = std::fs::read_to_string(digest_path).ok()?;
        ContentDigest::parse(content.trim())
    }

    /// Store artifact contents in the cache.
    ///
    /// Writes extracted artifact files to the cache directory.
    ///
    /// # Errors
    /// Returns `CacheError` if filesystem operations fail.
    ///
    /// COLD PATH.
    pub fn store(
        &self,
        reference: &OciReference,
        contents: &crate::artifact::ArtifactContents,
        digest: &ContentDigest,
    ) -> Result<PathBuf, CacheError> {
        let dir = self.artifact_dir(reference);
        std::fs::create_dir_all(&dir).map_err(|e| CacheError::Io {
            path: dir.clone(),
            source: e,
        })?;

        // Write wasm
        std::fs::write(dir.join("component.wasm"), &contents.wasm_bytes).map_err(|e| {
            CacheError::Io {
                path: dir.join("component.wasm"),
                source: e,
            }
        })?;

        // Write manifest
        let manifest_toml =
            contents
                .manifest
                .to_toml_string()
                .map_err(|e| CacheError::CorruptedIndex {
                    reason: format!("manifest serialization failed: {e}"),
                })?;
        std::fs::write(dir.join("Torvyn.toml"), manifest_toml.as_bytes()).map_err(|e| {
            CacheError::Io {
                path: dir.join("Torvyn.toml"),
                source: e,
            }
        })?;

        // Write WIT files
        let wit_dir = dir.join("wit");
        std::fs::create_dir_all(&wit_dir).map_err(|e| CacheError::Io {
            path: wit_dir.clone(),
            source: e,
        })?;
        for (name, content) in &contents.wit_files {
            std::fs::write(wit_dir.join(name), content.as_bytes()).map_err(|e| CacheError::Io {
                path: wit_dir.join(name),
                source: e,
            })?;
        }

        // Write provenance
        if let Some(ref prov) = contents.provenance {
            if let Ok(json) = prov.to_intoto_json() {
                let _ = std::fs::write(dir.join("provenance.json"), json.as_bytes());
            }
        }

        // Write digest marker
        let digest_path = dir.with_extension("digest");
        std::fs::write(&digest_path, &digest.prefixed).map_err(|e| CacheError::Io {
            path: digest_path,
            source: e,
        })?;

        Ok(dir)
    }

    /// Remove a cached artifact.
    ///
    /// # Errors
    /// Returns `CacheError` if filesystem operations fail.
    ///
    /// COLD PATH.
    pub fn remove(&self, reference: &OciReference) -> Result<(), CacheError> {
        let dir = self.artifact_dir(reference);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| CacheError::Io {
                path: dir.clone(),
                source: e,
            })?;
        }
        // Also remove the .digest file
        let digest_path = dir.with_extension("digest");
        if digest_path.exists() {
            let _ = std::fs::remove_file(&digest_path);
        }
        Ok(())
    }

    /// Remove all cached artifacts.
    ///
    /// # Errors
    /// Returns `CacheError` if filesystem operations fail.
    ///
    /// COLD PATH.
    pub fn clean_all(&self) -> Result<(), CacheError> {
        if self.config.root.exists() {
            std::fs::remove_dir_all(&self.config.root).map_err(|e| CacheError::Io {
                path: self.config.root.clone(),
                source: e,
            })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::ArtifactContents;
    use crate::manifest::ArtifactManifest;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn test_cache(dir: &Path) -> ArtifactCache {
        ArtifactCache::new(CacheConfig {
            root: dir.join("cache"),
        })
    }

    fn test_contents() -> ArtifactContents {
        ArtifactContents {
            manifest: ArtifactManifest::new("cached-comp".into(), "1.0.0".into())
                .with_contracts(vec!["torvyn:streaming@0.1.0".into()]),
            wasm_bytes: b"\0asm\x01\x00\x00\x00".to_vec(),
            wit_files: {
                let mut m = BTreeMap::new();
                m.insert("streaming.wit".into(), "package torvyn:streaming;\n".into());
                m
            },
            provenance: None,
            layer_digests: BTreeMap::new(),
        }
    }

    #[test]
    fn not_cached_initially() {
        let dir = TempDir::new().unwrap();
        let cache = test_cache(dir.path());
        let reference = OciReference::parse("ghcr.io/org/comp:1.0.0").unwrap();
        assert!(!cache.is_cached(&reference));
    }

    #[test]
    fn store_then_is_cached() {
        let dir = TempDir::new().unwrap();
        let cache = test_cache(dir.path());
        let reference = OciReference::parse("ghcr.io/org/comp:1.0.0").unwrap();
        let contents = test_contents();
        let digest = ContentDigest::of_bytes(b"artifact");

        cache.store(&reference, &contents, &digest).unwrap();
        assert!(cache.is_cached(&reference));
    }

    #[test]
    fn store_writes_all_files() {
        let dir = TempDir::new().unwrap();
        let cache = test_cache(dir.path());
        let reference = OciReference::parse("ghcr.io/org/comp:1.0.0").unwrap();
        let contents = test_contents();
        let digest = ContentDigest::of_bytes(b"artifact");

        let artifact_dir = cache.store(&reference, &contents, &digest).unwrap();

        assert!(artifact_dir.join("component.wasm").exists());
        assert!(artifact_dir.join("Torvyn.toml").exists());
        assert!(artifact_dir.join("wit/streaming.wit").exists());
    }

    #[test]
    fn cached_digest_returns_stored_digest() {
        let dir = TempDir::new().unwrap();
        let cache = test_cache(dir.path());
        let reference = OciReference::parse("ghcr.io/org/comp:1.0.0").unwrap();
        let contents = test_contents();
        let digest = ContentDigest::of_bytes(b"artifact");

        cache.store(&reference, &contents, &digest).unwrap();
        let cached = cache.cached_digest(&reference).unwrap();
        assert_eq!(cached, digest);
    }

    #[test]
    fn remove_clears_cache() {
        let dir = TempDir::new().unwrap();
        let cache = test_cache(dir.path());
        let reference = OciReference::parse("ghcr.io/org/comp:1.0.0").unwrap();
        let contents = test_contents();
        let digest = ContentDigest::of_bytes(b"artifact");

        cache.store(&reference, &contents, &digest).unwrap();
        assert!(cache.is_cached(&reference));

        cache.remove(&reference).unwrap();
        assert!(!cache.is_cached(&reference));
    }

    #[test]
    fn clean_all_removes_everything() {
        let dir = TempDir::new().unwrap();
        let cache = test_cache(dir.path());

        let ref1 = OciReference::parse("ghcr.io/org/a:1.0.0").unwrap();
        let ref2 = OciReference::parse("ghcr.io/org/b:2.0.0").unwrap();
        let contents = test_contents();
        let digest = ContentDigest::of_bytes(b"artifact");

        cache.store(&ref1, &contents, &digest).unwrap();
        cache.store(&ref2, &contents, &digest).unwrap();

        cache.clean_all().unwrap();
        assert!(!cache.is_cached(&ref1));
        assert!(!cache.is_cached(&ref2));
    }
}
