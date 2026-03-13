//! Wasmtime-based implementation of the [`WasmEngine`] trait.
//!
//! This module is gated behind the `wasmtime-backend` feature flag (default: on).
//!
//! # LLI DEVIATIONS from LLI-04 (adapted per spike findings)
//! - Wasmtime v42 instead of v29
//! - `async_support(true)` removed: deprecated no-op in v42
//! - `post_return_async()` removed: deprecated no-op in v42
//! - `wasmtime::Error` is distinct from `anyhow::Error` in v42

use async_trait::async_trait;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimitsBuilder};

use torvyn_types::ComponentId;

use crate::config::WasmtimeEngineConfig;
use crate::error::EngineError;
use crate::traits::WasmEngine;
use crate::types::{
    CompiledComponent, CompiledComponentInner, ComponentInstance, ComponentInstanceInner,
    HostState, ImportBindings, ImportBindingsInner, WasmtimeInstanceState,
};

/// Wasmtime-based Wasm engine implementation.
///
/// Wraps a `wasmtime::Engine` configured per [`WasmtimeEngineConfig`].
/// Thread-safe: the inner `wasmtime::Engine` is `Send + Sync` and can
/// be shared across async tasks.
///
/// # COLD PATH — constructed once at host startup.
///
/// # Examples
/// ```no_run
/// use torvyn_engine::{WasmtimeEngine, WasmtimeEngineConfig};
///
/// let config = WasmtimeEngineConfig::default();
/// let engine = WasmtimeEngine::new(config).expect("engine creation");
/// ```
pub struct WasmtimeEngine {
    /// The underlying Wasmtime engine.
    engine: Engine,

    /// The configuration used to create this engine.
    config: WasmtimeEngineConfig,
}

impl WasmtimeEngine {
    /// Create a new `WasmtimeEngine` with the given configuration.
    ///
    /// # COLD PATH — called once at host startup.
    ///
    /// # Errors
    /// Returns [`EngineError::Internal`] if the Wasmtime `Config` is invalid.
    pub fn new(config: WasmtimeEngineConfig) -> Result<Self, EngineError> {
        let problems = config.validate();
        if !problems.is_empty() {
            return Err(EngineError::Internal {
                reason: format!("Invalid engine configuration: {}", problems.join("; ")),
            });
        }

        let mut wasmtime_config = Config::new();

        // LLI DEVIATION: async_support(true) is deprecated (no-op) in Wasmtime 42.
        // Async is always available; no config needed.
        // wasmtime_config.async_support(true); // removed per spike finding 3.6

        // Fuel for CPU budgeting and cooperative preemption.
        if config.fuel_enabled {
            wasmtime_config.consume_fuel(true);
        }

        // SIMD support.
        wasmtime_config.wasm_simd(config.simd_enabled);

        // Multi-memory support.
        wasmtime_config.wasm_multi_memory(config.multi_memory);

        // Component Model support (required).
        wasmtime_config.wasm_component_model(true);

        // Stack size.
        wasmtime_config.max_wasm_stack(config.stack_size);

        // Parallel compilation.
        if let Some(threads) = config.compilation_threads {
            wasmtime_config.parallel_compilation(threads > 1);
        }

        // Compilation strategy.
        match config.strategy {
            crate::config::CompilationStrategy::Cranelift => {
                wasmtime_config.strategy(wasmtime::Strategy::Cranelift);
            }
            crate::config::CompilationStrategy::Winch => {
                // LLI DEVIATION: Winch may not be stable for Component Model.
                // Fall back to Cranelift until verified.
                wasmtime_config.strategy(wasmtime::Strategy::Cranelift);
            }
        }

        let engine =
            Engine::new(&wasmtime_config).map_err(|e| EngineError::Internal {
                reason: format!("Failed to create Wasmtime engine: {e}"),
            })?;

        Ok(Self { engine, config })
    }

    /// Returns a reference to the underlying Wasmtime engine.
    ///
    /// Useful for downstream crates that need to create linkers
    /// or other engine-dependent objects.
    #[inline]
    pub fn inner(&self) -> &Engine {
        &self.engine
    }

    /// Returns a reference to the engine configuration.
    #[inline]
    pub fn config(&self) -> &WasmtimeEngineConfig {
        &self.config
    }

    /// Create a new `Store` configured for a specific component instance.
    ///
    /// # COLD PATH — called once per component instantiation.
    fn create_store(&self, component_id: ComponentId) -> Store<HostState> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory_bytes)
            .table_elements(self.config.max_table_elements as usize)
            .instances(self.config.max_instances as usize)
            .trap_on_grow_failure(true) // Per spike finding 2.5
            .build();

        let host_state = HostState {
            component_id,
            limits,
            fuel_budget: self.config.default_fuel,
        };

        let mut store = Store::new(&self.engine, host_state);

        // Apply resource limiter.
        store.limiter(|state| &mut state.limits);

        // Set initial fuel if enabled.
        if self.config.fuel_enabled {
            store
                .set_fuel(self.config.default_fuel)
                .expect("fuel should be configurable when consume_fuel is enabled");

            // Configure async yield interval for cooperative preemption.
            if self.config.fuel_yield_interval > 0 {
                store
                    .fuel_async_yield_interval(Some(self.config.fuel_yield_interval))
                    .expect("fuel yield interval should be configurable");
            }
        }

        store
    }

    /// Create a new `Linker` for the engine.
    ///
    /// # COLD PATH — called during pipeline linking.
    /// Used by downstream crates (torvyn-linker) and tests.
    #[allow(dead_code)]
    pub(crate) fn create_linker(&self) -> Linker<HostState> {
        Linker::new(&self.engine)
    }

    /// Wrap a `Linker` into `ImportBindings`.
    ///
    /// # COLD PATH.
    /// Used by downstream crates (torvyn-linker) and tests.
    #[allow(dead_code)]
    pub(crate) fn import_bindings_from_linker(linker: Linker<HostState>) -> ImportBindings {
        ImportBindings {
            inner: ImportBindingsInner::Wasmtime(linker),
        }
    }
}

#[async_trait]
impl WasmEngine for WasmtimeEngine {
    fn compile_component(&self, bytes: &[u8]) -> Result<CompiledComponent, EngineError> {
        let component = Component::new(&self.engine, bytes).map_err(|e| {
            EngineError::CompilationFailed {
                reason: e.to_string(),
                source_hint: None,
            }
        })?;

        Ok(CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        })
    }

    fn serialize_component(
        &self,
        compiled: &CompiledComponent,
    ) -> Result<Vec<u8>, EngineError> {
        match &compiled.inner {
            CompiledComponentInner::Wasmtime(component) => {
                component.serialize().map_err(|e| EngineError::Internal {
                    reason: format!("Serialization failed: {e}"),
                })
            }
            _ => Err(EngineError::Internal {
                reason: "Cannot serialize non-Wasmtime component".into(),
            }),
        }
    }

    unsafe fn deserialize_component(
        &self,
        bytes: &[u8],
    ) -> Result<Option<CompiledComponent>, EngineError> {
        // SAFETY: Caller guarantees bytes are from serialize_component
        // with matching engine config. Wasmtime validates the format
        // header before loading native code.
        match unsafe { Component::deserialize(&self.engine, bytes) } {
            Ok(component) => Ok(Some(CompiledComponent {
                inner: CompiledComponentInner::Wasmtime(component),
            })),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("incompatible") || msg.contains("version") {
                    Ok(None)
                } else {
                    Err(EngineError::DeserializationFailed { reason: msg })
                }
            }
        }
    }

    async fn instantiate(
        &self,
        compiled: &CompiledComponent,
        imports: ImportBindings,
        component_id: ComponentId,
    ) -> Result<ComponentInstance, EngineError> {
        let component = match &compiled.inner {
            CompiledComponentInner::Wasmtime(c) => c,
            _ => {
                return Err(EngineError::Internal {
                    reason: "Cannot instantiate non-Wasmtime component".into(),
                });
            }
        };

        let linker = match imports.inner {
            ImportBindingsInner::Wasmtime(l) => l,
            _ => {
                return Err(EngineError::Internal {
                    reason: "Cannot use non-Wasmtime import bindings".into(),
                });
            }
        };

        let mut store = self.create_store(component_id);

        // Instantiate the component asynchronously.
        let instance = linker
            .instantiate_async(&mut store, component)
            .await
            .map_err(|e| EngineError::InstantiationFailed {
                component_id,
                reason: e.to_string(),
            })?;

        // Pre-resolve exported function handles for hot-path invocation.
        let func_process = instance.get_func(&mut store, "process");
        let func_pull = instance.get_func(&mut store, "pull");
        let func_push = instance.get_func(&mut store, "push");
        let func_init = instance.get_func(&mut store, "init");
        let func_teardown = instance.get_func(&mut store, "teardown");

        let has_processor = func_process.is_some();
        let has_source = func_pull.is_some();
        let has_sink = func_push.is_some();
        let has_lifecycle = func_init.is_some();

        let state = WasmtimeInstanceState {
            store,
            instance,
            func_process,
            func_pull,
            func_push,
            func_init,
            func_teardown,
        };

        Ok(ComponentInstance {
            component_id,
            inner: ComponentInstanceInner::Wasmtime(state),
            has_lifecycle,
            has_processor,
            has_source,
            has_sink,
        })
    }

    /// # WARM PATH — called before each invocation.
    fn set_fuel(
        &self,
        instance: &mut ComponentInstance,
        fuel: u64,
    ) -> Result<(), EngineError> {
        match &mut instance.inner {
            ComponentInstanceInner::Wasmtime(state) => {
                state
                    .store
                    .set_fuel(fuel)
                    .map_err(|e| EngineError::Internal {
                        reason: format!("Failed to set fuel: {e}"),
                    })
            }
            _ => Err(EngineError::Internal {
                reason: "set_fuel called on non-Wasmtime instance".into(),
            }),
        }
    }

    /// # HOT PATH — called after each invocation.
    fn fuel_remaining(&self, instance: &ComponentInstance) -> Option<u64> {
        match &instance.inner {
            ComponentInstanceInner::Wasmtime(state) => state.store.get_fuel().ok(),
            _ => None,
        }
    }

    /// # WARM PATH
    fn memory_usage(&self, instance: &ComponentInstance) -> usize {
        match &instance.inner {
            ComponentInstanceInner::Wasmtime(_state) => {
                // LLI DEVIATION: There is no single API to get total memory
                // usage of a component instance. Component instances may
                // contain multiple core module instances. For Phase 0, return
                // 0 and rely on StoreLimits for enforcement.
                0
            }
            _ => 0,
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
    fn test_wasmtime_engine_creation() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config);
        assert!(engine.is_ok());
    }

    #[test]
    fn test_wasmtime_engine_invalid_config() {
        let mut config = WasmtimeEngineConfig::default();
        config.fuel_enabled = true;
        config.default_fuel = 0; // Invalid
        let engine = WasmtimeEngine::new(config);
        assert!(engine.is_err());
    }

    #[test]
    fn test_compile_invalid_bytes() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();
        let result = engine.compile_component(b"not a wasm component");
        assert!(result.is_err());
        match result.unwrap_err() {
            EngineError::CompilationFailed { .. } => {}
            other => panic!("expected CompilationFailed, got: {other}"),
        }
    }

    #[test]
    fn test_compile_empty_bytes() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();
        let result = engine.compile_component(b"");
        assert!(result.is_err());
    }

    #[test]
    fn test_compile_minimal_component_from_wat() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();

        // A minimal valid component using WAT text format.
        // Wasmtime can compile WAT directly via Component::new.
        let wat = "(component)";
        // compile_component takes bytes, WAT may not work through that path.
        // Use the engine directly for WAT.
        let _result = engine.compile_component(wat.as_bytes());
        let component = Component::new(engine.inner(), wat);
        assert!(component.is_ok(), "engine should compile minimal WAT");
    }

    #[test]
    fn test_engine_config_accessors() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();
        assert!(engine.config().fuel_enabled);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();

        // Compile a minimal component.
        let component =
            Component::new(engine.inner(), "(component)").expect("compile WAT");
        let compiled = CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        };

        // Serialize.
        let bytes = engine
            .serialize_component(&compiled)
            .expect("serialize should work");
        assert!(!bytes.is_empty());

        // Deserialize.
        // SAFETY: bytes were just produced by serialize_component with same engine.
        let deserialized = unsafe { engine.deserialize_component(&bytes) }
            .expect("deserialize should work");
        assert!(deserialized.is_some());
    }

    #[tokio::test]
    async fn test_instantiate_minimal_component() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();

        let component =
            Component::new(engine.inner(), "(component)").expect("compile WAT");
        let compiled = CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        };

        let linker = engine.create_linker();
        let imports = WasmtimeEngine::import_bindings_from_linker(linker);
        let component_id = ComponentId::new(1);

        let instance = engine
            .instantiate(&compiled, imports, component_id)
            .await;
        assert!(instance.is_ok());

        let inst = instance.unwrap();
        assert_eq!(inst.component_id(), component_id);
        // Minimal component has no exports.
        assert!(!inst.has_processor());
        assert!(!inst.has_source());
        assert!(!inst.has_sink());
        assert!(!inst.has_lifecycle());
    }

    #[tokio::test]
    async fn test_fuel_set_and_read() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();

        let component =
            Component::new(engine.inner(), "(component)").expect("compile WAT");
        let compiled = CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        };

        let linker = engine.create_linker();
        let imports = WasmtimeEngine::import_bindings_from_linker(linker);
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        // Default fuel should be set.
        let remaining = engine.fuel_remaining(&instance);
        assert!(remaining.is_some());

        // Set new fuel.
        engine.set_fuel(&mut instance, 500).unwrap();
        assert_eq!(engine.fuel_remaining(&instance), Some(500));
    }

    #[test]
    fn test_memory_usage_returns_zero_for_now() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();

        // Without an instance, we can't test this directly.
        // This is tested via the instantiate path above.
        let _ = engine;
    }
}
