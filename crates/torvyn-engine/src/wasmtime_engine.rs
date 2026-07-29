//! Wasmtime-based implementation of the [`WasmEngine`] trait.
//!
//! This module is gated behind the `wasmtime-backend` feature flag (default: on).
//!
//! # LLI DEVIATIONS from LLI-04 (adapted per spike findings)
//! - Wasmtime v42 instead of v29
//! - `async_support(true)` removed: deprecated no-op in v42
//! - `post_return_async()` removed: deprecated no-op in v42
//! - `wasmtime::Error` is distinct from `anyhow::Error` in v42

use std::sync::Arc;

use async_trait::async_trait;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, StoreLimitsBuilder};

use torvyn_resources::DefaultResourceManager;
use torvyn_security::WasiConfiguration;
use torvyn_types::{ComponentId, FlowId};

use crate::config::WasmtimeEngineConfig;
use crate::error::EngineError;
use crate::host_state::{self, HostState};
use crate::traits::WasmEngine;
use crate::types::{
    CompiledComponent, CompiledComponentInner, ComponentInstance, ComponentInstanceInner,
    ImportBindings, ImportBindingsInner, WasmtimeInstanceState, WitBindings,
};
use crate::wit_bindings;

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

    /// Shared resource manager that owns the buffer pool, ownership
    /// state machine, and copy ledger. Cloned into every `HostState`
    /// at instantiation time. For Session 2.2 this is constructed
    /// internally by [`WasmtimeEngine::new`]; a future host-builder
    /// integration will pass one in via
    /// [`WasmtimeEngine::with_resource_manager`].
    resources: Arc<DefaultResourceManager>,
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

        let engine = Engine::new(&wasmtime_config).map_err(|e| EngineError::Internal {
            reason: format!("Failed to create Wasmtime engine: {e}"),
        })?;

        let resources = Arc::new(DefaultResourceManager::new_for_testing());

        Ok(Self {
            engine,
            config,
            resources,
        })
    }

    /// Create a new `WasmtimeEngine` that shares the supplied resource
    /// manager with the rest of the host.
    ///
    /// This constructor is the preferred shape once the host builder is
    /// updated to construct a single [`DefaultResourceManager`] and share
    /// it across the engine, reactor, and pipeline crates. Session 2.2
    /// keeps [`WasmtimeEngine::new`] backward-compatible (internal
    /// manager) so existing callers don't have to change.
    ///
    /// # COLD PATH — called once at host startup.
    ///
    /// # Errors
    /// Returns [`EngineError::Internal`] if the Wasmtime `Config` is invalid.
    pub fn with_resource_manager(
        config: WasmtimeEngineConfig,
        resources: Arc<DefaultResourceManager>,
    ) -> Result<Self, EngineError> {
        let mut engine = Self::new(config)?;
        engine.resources = resources;
        Ok(engine)
    }

    /// Returns a clone of the shared resource manager handle.
    ///
    /// # COLD PATH — for downstream wiring and diagnostics.
    #[inline]
    pub fn resource_manager(&self) -> Arc<DefaultResourceManager> {
        Arc::clone(&self.resources)
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
    fn create_store(
        &self,
        component_id: ComponentId,
        wasi: &WasiConfiguration,
    ) -> Result<Store<HostState>, EngineError> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_memory_bytes)
            .table_elements(self.config.max_table_elements as usize)
            .instances(self.config.max_instances as usize)
            .trap_on_grow_failure(true) // Per spike finding 2.5
            .build();

        // The real flow identifier is assigned by the reactor at spawn time,
        // which happens *after* instantiation. Start each store with the
        // unassigned sentinel (`FlowId::new(0)`; the reactor's `next_flow_id`
        // begins at 1, so it is never a live flow). The reactor stamps the
        // real `FlowId` onto every store via `ComponentInstance::set_flow_id`
        // before the flow driver runs, so all resource operations — copies,
        // allocations — are attributed to the correct flow rather than a
        // per-component placeholder.
        let flow_id = FlowId::new(0);

        // Build the component's WASI sandbox from its resolved capabilities.
        // A deny-all configuration yields the most restrictive context.
        let wasi_ctx =
            host_state::build_wasi_ctx(wasi).map_err(|reason| EngineError::WasiConfigError {
                component_id,
                reason,
            })?;

        // The store carries its own per-invocation fuel budget so the invoker
        // can refuel before every guest call. Zero means "fuel budgeting is
        // disabled for this store", which is the only state in which
        // `Store::set_fuel` would fail (the engine's `consume_fuel` is off).
        let fuel_budget = if self.config.fuel_enabled {
            self.config.default_fuel
        } else {
            0
        };

        let host_state = HostState::new(
            component_id,
            limits,
            fuel_budget,
            Arc::clone(&self.resources),
            flow_id,
            wasi_ctx,
        );

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

        Ok(store)
    }

    /// Create a new `Linker` with all Torvyn host imports pre-registered,
    /// plus a no-permission WASI Preview-2 sandbox.
    ///
    /// The linker carries two unrelated import groups:
    ///
    /// 1. **Torvyn streaming imports** — the `torvyn:streaming/types`
    ///    and `torvyn:streaming/buffer-allocator` interfaces. We wire
    ///    them via the `data-source` world's bindgen because its
    ///    import set is a strict superset of every other archetype's;
    ///    unused entries are harmless for components that import a
    ///    subset.
    /// 2. **WASI Preview-2 imports** — guest components produced by
    ///    `cargo-component`, TinyGo, and `componentize-py` link to
    ///    WASI Preview-2 via their language runtimes even when the
    ///    guest code never performs I/O (allocator setup, panic
    ///    abort, environment, clocks). `wasmtime_wasi::p2::add_to_linker_async`
    ///    satisfies those with a real Preview-2 implementation
    ///    backed by `HostState::wasi`, which is the most restrictive
    ///    sandbox the WASI builder offers (no stdio inheritance, no
    ///    filesystem preopens, no env vars, no sockets). A component
    ///    that actually invokes a WASI function returns the
    ///    sandbox's failure mode (closed stream, empty env, etc.)
    ///    rather than trapping or leaking host capability.
    ///
    /// Order matters: WASI is added first, then the Torvyn imports.
    /// If the two ever overlapped (they don't today; the Torvyn WIT
    /// declares a distinct package), the Torvyn impls would win.
    ///
    /// # COLD PATH — called during pipeline linking.
    pub(crate) fn create_linker(&self) -> Linker<HostState> {
        let mut linker: Linker<HostState> = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .expect("wasmtime-wasi p2 add_to_linker_async must succeed for an empty linker");
        wit_bindings::data_source::DataSource::add_to_linker::<_, HasSelf<HostState>>(
            &mut linker,
            |state| state,
        )
        .expect("host trait impls cover every imported function declared in the data-source world");
        linker
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
        let component =
            Component::new(&self.engine, bytes).map_err(|e| EngineError::CompilationFailed {
                reason: e.to_string(),
                source_hint: None,
            })?;

        Ok(CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        })
    }

    fn serialize_component(&self, compiled: &CompiledComponent) -> Result<Vec<u8>, EngineError> {
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
        wasi: &WasiConfiguration,
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

        // The linker arriving here was produced by `create_linker`,
        // which has already wired both the Torvyn host imports and the
        // WASI Preview-2 sandbox. No further patching is required —
        // any guest import outside that union is a real configuration
        // error and `instantiate_async` will reject it.

        let mut store = self.create_store(component_id, wasi)?;

        // Instantiate the component asynchronously.
        let instance = linker
            .instantiate_async(&mut store, component)
            .await
            .map_err(|e| EngineError::InstantiationFailed {
                component_id,
                reason: e.to_string(),
            })?;

        // Detect which world the component implements by trying each
        // bindgen-generated wrapper in turn. The four sets of required
        // exports are mutually exclusive in practice, so at most one
        // wrapper succeeds. Components with no exports (e.g., minimal
        // WAT used in unit tests) yield `None`.
        let bindings = detect_world(&mut store, &instance);

        let has_source = matches!(bindings, Some(WitBindings::DataSource(_)));
        let has_sink = matches!(bindings, Some(WitBindings::DataSink(_)));
        let has_processor = matches!(
            bindings,
            Some(WitBindings::Transform(_) | WitBindings::ManagedTransform(_))
        );
        let has_lifecycle = matches!(
            bindings,
            Some(
                WitBindings::DataSource(_)
                    | WitBindings::DataSink(_)
                    | WitBindings::ManagedTransform(_)
            )
        );

        let state = WasmtimeInstanceState {
            store,
            instance,
            bindings,
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
    fn set_fuel(&self, instance: &mut ComponentInstance, fuel: u64) -> Result<(), EngineError> {
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

    fn default_imports(&self) -> ImportBindings {
        Self::import_bindings_from_linker(self.create_linker())
    }
}

/// Try each bindgen-generated world wrapper in turn against `instance`,
/// returning the first that successfully resolves all its exports.
///
/// The four worlds have disjoint required export sets (`pull` vs `push` vs
/// `process` etc.), so at most one wrapper succeeds for any well-formed
/// component. Components with no exports (empty `(component)` WAT used by
/// some unit tests) produce `None`.
///
/// # COLD PATH — called once per component instantiation.
fn detect_world(
    store: &mut Store<HostState>,
    instance: &wasmtime::component::Instance,
) -> Option<WitBindings> {
    if let Ok(b) = wit_bindings::data_source::DataSource::new(&mut *store, instance) {
        return Some(WitBindings::DataSource(b));
    }
    if let Ok(b) = wit_bindings::managed_transform::ManagedTransform::new(&mut *store, instance) {
        return Some(WitBindings::ManagedTransform(b));
    }
    if let Ok(b) = wit_bindings::data_sink::DataSink::new(&mut *store, instance) {
        return Some(WitBindings::DataSink(b));
    }
    if let Ok(b) = wit_bindings::transform::Transform::new(&mut *store, instance) {
        return Some(WitBindings::Transform(b));
    }
    None
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
        let config = WasmtimeEngineConfig {
            fuel_enabled: true,
            default_fuel: 0, // Invalid
            ..WasmtimeEngineConfig::default()
        };
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
        let wat = "(component)";
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

        let component = Component::new(engine.inner(), "(component)").expect("compile WAT");
        let compiled = CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        };

        let bytes = engine
            .serialize_component(&compiled)
            .expect("serialize should work");
        assert!(!bytes.is_empty());

        // SAFETY: bytes were just produced by serialize_component with same engine.
        let deserialized =
            unsafe { engine.deserialize_component(&bytes) }.expect("deserialize should work");
        assert!(deserialized.is_some());
    }

    #[tokio::test]
    async fn test_instantiate_minimal_component() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();

        let component = Component::new(engine.inner(), "(component)").expect("compile WAT");
        let compiled = CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        };

        let imports = WasmtimeEngine::import_bindings_from_linker(engine.create_linker());
        let component_id = ComponentId::new(1);

        let instance = engine
            .instantiate(
                &compiled,
                imports,
                component_id,
                &WasiConfiguration::deny_all(),
            )
            .await;
        assert!(instance.is_ok());

        let inst = instance.unwrap();
        assert_eq!(inst.component_id(), component_id);
        // Minimal component has no exports — no world matches.
        assert!(!inst.has_processor());
        assert!(!inst.has_source());
        assert!(!inst.has_sink());
        assert!(!inst.has_lifecycle());
    }

    #[tokio::test]
    async fn test_fuel_set_and_read() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();

        let component = Component::new(engine.inner(), "(component)").expect("compile WAT");
        let compiled = CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        };

        let imports = WasmtimeEngine::import_bindings_from_linker(engine.create_linker());
        let mut instance = engine
            .instantiate(
                &compiled,
                imports,
                ComponentId::new(1),
                &WasiConfiguration::deny_all(),
            )
            .await
            .unwrap();

        let remaining = engine.fuel_remaining(&instance);
        assert!(remaining.is_some());

        engine.set_fuel(&mut instance, 500).unwrap();
        assert_eq!(engine.fuel_remaining(&instance), Some(500));
    }

    #[test]
    fn test_memory_usage_returns_zero_for_now() {
        let config = WasmtimeEngineConfig::default();
        let engine = WasmtimeEngine::new(config).unwrap();
        let _ = engine;
    }
}
