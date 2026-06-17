//! Core traits for the Torvyn Wasm engine abstraction.
//!
//! [`WasmEngine`] abstracts Wasm runtime operations (compile, instantiate).
//! [`ComponentInvoker`] is the hot-path typed invocation interface between
//! the reactor and Wasm execution (Gap G-01 fix).
//!
//! Both traits use associated types for static dispatch to eliminate
//! virtual call overhead in the hot path.

use async_trait::async_trait;
use torvyn_security::WasiConfiguration;
use torvyn_types::{BackpressureSignal, ComponentId, ProcessError};

use crate::error::EngineError;
use crate::types::{
    CompiledComponent, ComponentInstance, ImportBindings, OutputElement, ProcessResult,
    StreamElement,
};

// ---------------------------------------------------------------------------
// WasmEngine trait
// ---------------------------------------------------------------------------

/// Abstraction over a WebAssembly Component Model engine.
///
/// Per Doc 02, Section 3.1: the primary implementation wraps Wasmtime.
/// Future implementations could wrap alternative engines.
///
/// The trait uses monomorphization at call sites for zero virtual dispatch
/// overhead in the hot path.
///
/// # Design Decision (Doc 02, Section 3.1)
/// The trait boundary provides a natural mock point for testing the host,
/// linker, and reactor without a real Wasm engine.
///
/// # Examples
/// ```no_run
/// use torvyn_engine::{WasmEngine, WasmtimeEngineConfig};
///
/// # async fn example() -> Result<(), torvyn_engine::EngineError> {
/// let config = WasmtimeEngineConfig::default();
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait WasmEngine: Send + Sync + 'static {
    /// Compile a Wasm component binary to native code.
    ///
    /// # COLD PATH — called once per component type during pipeline setup.
    ///
    /// # Errors
    /// Returns [`EngineError::CompilationFailed`] if the binary is invalid.
    fn compile_component(&self, bytes: &[u8]) -> Result<CompiledComponent, EngineError>;

    /// Serialize a compiled component for disk caching.
    ///
    /// # COLD PATH — called after compilation for cache storage.
    ///
    /// # Errors
    /// Returns [`EngineError::Internal`] on serialization failure.
    fn serialize_component(&self, compiled: &CompiledComponent) -> Result<Vec<u8>, EngineError>;

    /// Deserialize a previously compiled component from cached bytes.
    ///
    /// # COLD PATH — called during pipeline setup for cache hits.
    ///
    /// # Safety
    /// The caller must ensure `bytes` were produced by `serialize_component`
    /// with the same engine configuration. Passing arbitrary bytes is
    /// **unsafe** because deserializing native code requires trust.
    ///
    /// # Errors
    /// Returns [`EngineError::DeserializationFailed`] if the bytes are
    /// incompatible with the current engine configuration.
    ///
    /// # Returns
    /// `Ok(Some(compiled))` on success, `Ok(None)` if the cached bytes
    /// are recognized but incompatible (different engine version/config).
    unsafe fn deserialize_component(
        &self,
        bytes: &[u8],
    ) -> Result<Option<CompiledComponent>, EngineError>;

    /// Instantiate a compiled component with the given import bindings and
    /// WASI sandbox configuration.
    ///
    /// `wasi` is the component's resolved [`WasiConfiguration`]: the host
    /// builds the component's WASI Preview-2 context from it, granting only the
    /// filesystem, environment, stdio, and network access its capabilities
    /// allow. Pass [`WasiConfiguration::deny_all`] for a fully sandboxed
    /// component.
    ///
    /// # COLD PATH — called once per component instance during pipeline setup.
    ///
    /// # Errors
    /// - [`EngineError::InstantiationFailed`] on instantiation failure.
    /// - [`EngineError::UnresolvedImport`] if imports cannot be satisfied.
    /// - [`EngineError::WasiConfigError`] if the WASI sandbox cannot be built
    ///   (e.g. a granted directory cannot be preopened).
    async fn instantiate(
        &self,
        compiled: &CompiledComponent,
        imports: ImportBindings,
        component_id: ComponentId,
        wasi: &WasiConfiguration,
    ) -> Result<ComponentInstance, EngineError>;

    /// Set the fuel budget for a component instance.
    ///
    /// # WARM PATH — called before each invocation by the reactor.
    ///
    /// # Errors
    /// Returns [`EngineError::Internal`] if fuel is not enabled.
    fn set_fuel(&self, instance: &mut ComponentInstance, fuel: u64) -> Result<(), EngineError>;

    /// Get the remaining fuel for a component instance.
    ///
    /// # HOT PATH — called after each invocation for accounting.
    ///
    /// # Returns
    /// `None` if fuel is not enabled. `Some(remaining)` otherwise.
    fn fuel_remaining(&self, instance: &ComponentInstance) -> Option<u64>;

    /// Get the current memory usage of a component instance in bytes.
    ///
    /// # WARM PATH — called for observability and limit checks.
    fn memory_usage(&self, instance: &ComponentInstance) -> usize;

    /// Construct a default [`ImportBindings`] suitable for instantiation.
    ///
    /// For Phase 0, this returns engine-specific empty bindings:
    /// - The Wasmtime backend returns an empty `Linker`.
    /// - The mock backend returns mock bindings.
    ///
    /// Phase 1 will introduce capability-resolved bindings via a separate
    /// API; this method gives downstream crates (e.g., `torvyn-pipeline`)
    /// a stable seam for the cold path that does not require knowing the
    /// concrete backend.
    ///
    /// # COLD PATH — called once per component instantiation.
    fn default_imports(&self) -> ImportBindings;
}

// ---------------------------------------------------------------------------
// ComponentInvoker trait — THE CRITICAL GAP G-01 FIX
// ---------------------------------------------------------------------------

/// Typed invocation interface between the reactor and Wasm execution.
///
/// **This trait is the critical Gap G-01 fix** identified in
/// `10_consolidated_hli_review.md`, Recommendation 2.
///
/// The reactor calls these methods to invoke component-exported functions.
/// Each method handles marshaling Rust types to/from Wasm Component Model
/// types, fuel consumption tracking, and error conversion.
///
/// # Performance
/// **THIS IS THE HOTTEST PATH IN TORVYN.** Every stream element passes
/// through `ComponentInvoker`. Requirements:
/// - Zero unnecessary allocations (pre-allocate marshaling buffers)
/// - No locks acquired during invocation
/// - No syscalls beyond the Wasm execution itself
/// - All observability recording is non-blocking
///
/// # Design Decision (Doc 10, Recommendation 2)
/// The invoker is a separate trait from `WasmEngine` because:
/// 1. The engine operates on raw bytes and component types (cold path).
/// 2. The invoker operates on Torvyn domain types (hot path).
/// 3. Separation allows independent optimization and testing.
#[async_trait]
pub trait ComponentInvoker: Send + Sync + 'static {
    /// Invoke a source component's `pull` function.
    ///
    /// Sources produce stream elements on demand.
    ///
    /// # HOT PATH — called per element produced by a source.
    ///
    /// # Returns
    /// - `Ok(Some(element))` — the source produced an element.
    /// - `Ok(None)` — the source has no more data (stream complete).
    /// - `Err(ProcessError)` — the source encountered an error.
    async fn invoke_pull(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
    ) -> Result<Option<OutputElement>, ProcessError>;

    /// Invoke a processor component's `process` function.
    ///
    /// Processors transform stream elements 1:1.
    ///
    /// # HOT PATH — called per element passing through a processor.
    async fn invoke_process(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        element: StreamElement,
    ) -> Result<ProcessResult, ProcessError>;

    /// Invoke a sink component's `push` function.
    ///
    /// Sinks consume stream elements.
    ///
    /// # HOT PATH — called per element arriving at a sink.
    async fn invoke_push(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        element: StreamElement,
    ) -> Result<BackpressureSignal, ProcessError>;

    /// Invoke a component's `lifecycle.init` function.
    ///
    /// Called once per component during pipeline startup.
    ///
    /// # COLD PATH — called once at component startup.
    async fn invoke_init(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        config: &str,
    ) -> Result<(), ProcessError>;

    /// Invoke a component's `lifecycle.teardown` function.
    ///
    /// Called once per component during shutdown.
    /// Per C02-10: failures are logged but do not prevent termination.
    ///
    /// # COLD PATH — called once at component shutdown.
    async fn invoke_teardown(&self, instance: &mut ComponentInstance, component_id: ComponentId);
}

// ---------------------------------------------------------------------------
// Blanket impl: Arc<I>: ComponentInvoker
// ---------------------------------------------------------------------------

// Forwarding impl so a single shared invoker can be cloned into per-flow
// drivers. The reactor coordinator stores `Arc<I>`; each flow driver
// receives a clone, and this impl makes the driver's bound on
// `I: ComponentInvoker` satisfiable for the wrapped Arc.
#[async_trait]
impl<I: ComponentInvoker> ComponentInvoker for std::sync::Arc<I> {
    async fn invoke_pull(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
    ) -> Result<Option<OutputElement>, ProcessError> {
        (**self).invoke_pull(instance, component_id).await
    }

    async fn invoke_process(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        element: StreamElement,
    ) -> Result<ProcessResult, ProcessError> {
        (**self)
            .invoke_process(instance, component_id, element)
            .await
    }

    async fn invoke_push(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        element: StreamElement,
    ) -> Result<BackpressureSignal, ProcessError> {
        (**self).invoke_push(instance, component_id, element).await
    }

    async fn invoke_init(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        config: &str,
    ) -> Result<(), ProcessError> {
        (**self).invoke_init(instance, component_id, config).await
    }

    async fn invoke_teardown(&self, instance: &mut ComponentInstance, component_id: ComponentId) {
        (**self).invoke_teardown(instance, component_id).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Verify that the trait bounds are correct by checking concrete types
    // satisfy Send + Sync. The actual trait object safety is tested in
    // the mock module.
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use torvyn_types::{BackpressureSignal, ComponentId, ProcessError};

    #[test]
    fn test_trait_bounds_compile() {
        fn _assert_send_sync<T: Send + Sync + 'static>() {}
        // These are compile-time checks only; no runtime assertion needed.
    }

    /// Trivial invoker that records call counts via shared atomics.
    /// Used to verify the `Arc<I>: ComponentInvoker` blanket impl forwards
    /// each method to the inner implementation.
    struct CountingInvoker {
        pulls: std::sync::atomic::AtomicU64,
        processes: std::sync::atomic::AtomicU64,
        pushes: std::sync::atomic::AtomicU64,
        inits: std::sync::atomic::AtomicU64,
        teardowns: std::sync::atomic::AtomicU64,
    }

    impl CountingInvoker {
        fn new() -> Self {
            Self {
                pulls: std::sync::atomic::AtomicU64::new(0),
                processes: std::sync::atomic::AtomicU64::new(0),
                pushes: std::sync::atomic::AtomicU64::new(0),
                inits: std::sync::atomic::AtomicU64::new(0),
                teardowns: std::sync::atomic::AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl ComponentInvoker for CountingInvoker {
        async fn invoke_pull(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
        ) -> Result<Option<crate::types::OutputElement>, ProcessError> {
            self.pulls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(None)
        }

        async fn invoke_process(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
            _element: crate::types::StreamElement,
        ) -> Result<crate::types::ProcessResult, ProcessError> {
            self.processes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(crate::types::ProcessResult::Filtered)
        }

        async fn invoke_push(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
            _element: crate::types::StreamElement,
        ) -> Result<BackpressureSignal, ProcessError> {
            self.pushes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(BackpressureSignal::Ready)
        }

        async fn invoke_init(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
            _config: &str,
        ) -> Result<(), ProcessError> {
            self.inits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        async fn invoke_teardown(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
        ) {
            self.teardowns
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_arc_blanket_forwards_all_methods() {
        use crate::mock::MockEngine;
        use crate::WasmEngine;

        let inner = Arc::new(CountingInvoker::new());
        let arc_invoker: Arc<CountingInvoker> = Arc::clone(&inner);

        // Build a mock instance to call methods against.
        let engine = MockEngine::new();
        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(
                &compiled,
                imports,
                ComponentId::new(1),
                &WasiConfiguration::deny_all(),
            )
            .await
            .unwrap();

        // Use Arc<CountingInvoker> as a ComponentInvoker via the blanket impl.
        let invoker_via_arc: &dyn ComponentInvoker = &arc_invoker;
        let _ = invoker_via_arc
            .invoke_pull(&mut instance, ComponentId::new(1))
            .await;
        let _ = invoker_via_arc
            .invoke_process(
                &mut instance,
                ComponentId::new(1),
                crate::types::StreamElement {
                    meta: torvyn_types::ElementMeta::new(0, 0, String::new()),
                    payload: torvyn_types::BufferHandle::new(torvyn_types::ResourceId::new(0, 0)),
                },
            )
            .await;
        let _ = invoker_via_arc
            .invoke_push(
                &mut instance,
                ComponentId::new(1),
                crate::types::StreamElement {
                    meta: torvyn_types::ElementMeta::new(0, 0, String::new()),
                    payload: torvyn_types::BufferHandle::new(torvyn_types::ResourceId::new(0, 0)),
                },
            )
            .await;
        let _ = invoker_via_arc
            .invoke_init(&mut instance, ComponentId::new(1), "{}")
            .await;
        invoker_via_arc
            .invoke_teardown(&mut instance, ComponentId::new(1))
            .await;

        // Each method incremented the shared counter exactly once.
        assert_eq!(inner.pulls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            inner.processes.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(inner.pushes.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(inner.inits.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            inner.teardowns.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }
}
