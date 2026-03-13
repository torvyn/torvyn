//! Compiled component cache for fast instantiation.
//!
//! Caches [`CompiledComponent`] objects by [`ComponentTypeId`] to avoid
//! recompilation. When the `wasmtime-backend` feature is enabled, also
//! supports disk caching via Wasmtime's serialization.
//!
//! Per Doc 02, Section 2.4 and MR-06: `InstancePre` is available for
//! Component Model and is used for pre-resolved import caching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use torvyn_types::ComponentTypeId;

use crate::error::EngineError;
use crate::traits::WasmEngine;
use crate::types::CompiledComponent;

/// Cache for compiled WebAssembly components.
///
/// Provides both in-memory and optional disk caching. Components are
/// keyed by [`ComponentTypeId`] (SHA-256 of the binary).
///
/// Thread-safe: uses `RwLock` for concurrent read access.
///
/// # COLD PATH — all operations are during pipeline setup.
///
/// # Examples
/// ```
/// use torvyn_engine::CompiledComponentCache;
///
/// let cache = CompiledComponentCache::new(None);
/// assert_eq!(cache.len(), 0);
/// ```
pub struct CompiledComponentCache {
    /// In-memory cache of compiled components.
    memory: RwLock<HashMap<ComponentTypeId, CompiledComponent>>,

    /// Optional disk cache directory.
    disk_dir: Option<PathBuf>,
}

impl CompiledComponentCache {
    /// Create a new cache with an optional disk cache directory.
    ///
    /// # COLD PATH
    pub fn new(disk_dir: Option<PathBuf>) -> Self {
        Self {
            memory: RwLock::new(HashMap::new()),
            disk_dir,
        }
    }

    /// Compute the [`ComponentTypeId`] for a component binary.
    ///
    /// Uses SHA-256 to produce a deterministic content hash.
    ///
    /// # COLD PATH
    ///
    /// # Examples
    /// ```
    /// use torvyn_engine::CompiledComponentCache;
    ///
    /// let id = CompiledComponentCache::compute_type_id(b"hello");
    /// assert_eq!(id.as_bytes().len(), 32);
    /// ```
    pub fn compute_type_id(bytes: &[u8]) -> ComponentTypeId {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let hash: [u8; 32] = hasher.finalize().into();
        ComponentTypeId::new(hash)
    }

    /// Look up a compiled component in the cache (memory first, then disk).
    ///
    /// # COLD PATH
    ///
    /// # Returns
    /// - `Ok(Some(compiled))` if found in cache.
    /// - `Ok(None)` if not in cache.
    /// - `Err` if disk cache read failed fatally.
    pub fn get<E: WasmEngine>(
        &self,
        type_id: &ComponentTypeId,
        engine: &E,
    ) -> Result<Option<CompiledComponent>, EngineError> {
        // Check memory cache first (fast path).
        {
            let guard = self.memory.read();
            if let Some(compiled) = guard.get(type_id) {
                return Ok(Some(compiled.clone()));
            }
        }

        // Check disk cache.
        if let Some(ref dir) = self.disk_dir {
            let path = disk_cache_path(dir, type_id);
            if path.exists() {
                if let Ok(bytes) = std::fs::read(&path) {
                    // SAFETY: The cached bytes were produced by our own
                    // serialize_component and stored in a directory we control.
                    match unsafe { engine.deserialize_component(&bytes) } {
                        Ok(Some(compiled)) => {
                            // Promote to memory cache.
                            let mut guard = self.memory.write();
                            guard.insert(*type_id, compiled.clone());
                            return Ok(Some(compiled));
                        }
                        Ok(None) => {
                            // Incompatible cache entry — remove stale file.
                            let _ = std::fs::remove_file(&path);
                        }
                        Err(_e) => {
                            // Corrupt cache entry — remove.
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Insert a compiled component into the cache.
    ///
    /// # COLD PATH
    ///
    /// Stores in memory. If disk caching is enabled, also writes to disk
    /// (failures are non-fatal).
    pub fn insert<E: WasmEngine>(
        &self,
        type_id: ComponentTypeId,
        compiled: CompiledComponent,
        engine: &E,
    ) {
        // Insert into memory cache.
        {
            let mut guard = self.memory.write();
            guard.insert(type_id, compiled.clone());
        }

        // Write to disk cache (best-effort).
        if let Some(ref dir) = self.disk_dir {
            if let Ok(bytes) = engine.serialize_component(&compiled) {
                let path = disk_cache_path(dir, &type_id);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, &bytes);
            }
        }
    }

    /// Returns the number of components in the memory cache.
    pub fn len(&self) -> usize {
        self.memory.read().len()
    }

    /// Returns `true` if the memory cache is empty.
    pub fn is_empty(&self) -> bool {
        self.memory.read().is_empty()
    }

    /// Clear the in-memory cache.
    pub fn clear(&self) {
        self.memory.write().clear();
    }

    /// Compile a component with caching.
    ///
    /// Checks the cache first. If not found, compiles the component
    /// and inserts it into the cache.
    ///
    /// # COLD PATH
    pub fn compile_or_get<E: WasmEngine>(
        &self,
        bytes: &[u8],
        engine: &E,
    ) -> Result<(ComponentTypeId, CompiledComponent), EngineError> {
        let type_id = Self::compute_type_id(bytes);

        // Check cache.
        if let Some(compiled) = self.get(&type_id, engine)? {
            return Ok((type_id, compiled));
        }

        // Cache miss — compile.
        let compiled = engine.compile_component(bytes)?;

        // Insert into cache.
        self.insert(type_id, compiled.clone(), engine);

        Ok((type_id, compiled))
    }
}

/// Compute the disk cache file path for a given component type ID.
///
/// Uses a two-level directory structure: `{dir}/{first_2_hex}/{full_hex}.bin`
fn disk_cache_path(dir: &Path, type_id: &ComponentTypeId) -> PathBuf {
    let hex = format!("{type_id}");
    let prefix = &hex[..2.min(hex.len())];
    dir.join(prefix).join(format!("{hex}.bin"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_type_id_deterministic() {
        let id1 = CompiledComponentCache::compute_type_id(b"hello world");
        let id2 = CompiledComponentCache::compute_type_id(b"hello world");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_compute_type_id_different_inputs() {
        let id1 = CompiledComponentCache::compute_type_id(b"hello");
        let id2 = CompiledComponentCache::compute_type_id(b"world");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_cache_new_empty() {
        let cache = CompiledComponentCache::new(None);
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_clear() {
        let cache = CompiledComponentCache::new(None);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_disk_cache_path_format() {
        let type_id = ComponentTypeId::new([0xab; 32]);
        let path = disk_cache_path(Path::new("/tmp/cache"), &type_id);
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("cache"));
        assert!(path_str.ends_with(".bin"));
    }
}
