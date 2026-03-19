//! Engine-specific types for the Torvyn runtime.
//!
//! These types bridge Torvyn's domain model with the underlying Wasm engine.
//! They are designed for the hot path: [`StreamElement`] and [`OutputElement`]
//! are passed through [`ComponentInvoker`](crate::ComponentInvoker) on every element.

#[cfg(feature = "mock")]
use torvyn_types::BackpressureSignal;
use torvyn_types::{BufferHandle, ComponentId, ElementMeta};

// ---------------------------------------------------------------------------
// CompiledComponent
// ---------------------------------------------------------------------------

/// A compiled WebAssembly component, ready for instantiation.
///
/// This is an opaque wrapper around the engine-specific compiled
/// representation. For the Wasmtime backend, this wraps
/// `wasmtime::component::Component`.
///
/// `CompiledComponent` is cheaply cloneable (reference-counted internally
/// by Wasmtime).
///
/// # Invariants
/// - Always produced by a [`WasmEngine::compile_component`](crate::WasmEngine::compile_component) call.
/// - The inner representation is valid native code for the engine
///   configuration that produced it.
///
/// # COLD PATH — created during pipeline setup.
#[derive(Clone)]
pub struct CompiledComponent {
    /// The engine-specific compiled component.
    pub(crate) inner: CompiledComponentInner,
}

/// Backend-specific compiled component storage.
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) enum CompiledComponentInner {
    /// Wasmtime backend.
    #[cfg(feature = "wasmtime-backend")]
    Wasmtime(wasmtime::component::Component),
    /// Mock variant for testing.
    #[cfg(feature = "mock")]
    Mock(MockCompiledComponent),
    /// Placeholder to prevent empty enum when no features are enabled.
    #[allow(dead_code)]
    _Placeholder,
}

/// Mock compiled component data.
#[cfg(feature = "mock")]
#[derive(Clone, Debug)]
pub struct MockCompiledComponent {
    /// Unique identifier for this mock compiled component.
    #[allow(dead_code)]
    pub(crate) id: u64,
}

impl std::fmt::Debug for CompiledComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CompiledComponent")
    }
}

// ---------------------------------------------------------------------------
// ComponentInstance
// ---------------------------------------------------------------------------

/// A live WebAssembly component instance, ready for invocation.
///
/// Wraps the engine-specific instance along with Torvyn metadata
/// (component ID, fuel state).
///
/// # Invariants
/// - Produced by [`WasmEngine::instantiate`](crate::WasmEngine::instantiate).
/// - The instance is ready for function calls.
/// - Fuel and memory limits are applied.
///
/// # WARM PATH — created once per component, used on every invocation.
pub struct ComponentInstance {
    /// The unique identity of this component instance.
    pub(crate) component_id: ComponentId,

    /// The engine-specific instance state.
    pub(crate) inner: ComponentInstanceInner,

    /// Whether the component exports the `lifecycle` interface.
    pub(crate) has_lifecycle: bool,

    /// Whether the component exports the `processor` interface.
    pub(crate) has_processor: bool,

    /// Whether the component exports the `source` interface.
    pub(crate) has_source: bool,

    /// Whether the component exports the `sink` interface.
    pub(crate) has_sink: bool,
}

/// Backend-specific instance state.
pub(crate) enum ComponentInstanceInner {
    /// Wasmtime backend.
    #[cfg(feature = "wasmtime-backend")]
    Wasmtime(WasmtimeInstanceState),
    /// Mock backend for testing.
    #[cfg(feature = "mock")]
    Mock(MockInstanceState),
    /// Placeholder.
    #[allow(dead_code)]
    _Placeholder,
}

/// Wasmtime-specific instance state.
///
/// Holds the `Store` (which contains the instance) and pre-resolved
/// function handles for hot-path invocation.
#[cfg(feature = "wasmtime-backend")]
pub(crate) struct WasmtimeInstanceState {
    /// The Wasmtime store containing the instance.
    pub(crate) store: wasmtime::Store<HostState>,

    /// The component instance. Retained for future use (e.g., dynamic export
    /// discovery when Wasmtime adds Component Model export enumeration).
    #[allow(dead_code)]
    pub(crate) instance: wasmtime::component::Instance,

    /// Pre-resolved function handle for `process` (if exported).
    pub(crate) func_process: Option<wasmtime::component::Func>,

    /// Pre-resolved function handle for `pull` (if exported).
    pub(crate) func_pull: Option<wasmtime::component::Func>,

    /// Pre-resolved function handle for `push` (if exported).
    pub(crate) func_push: Option<wasmtime::component::Func>,

    /// Pre-resolved function handle for `lifecycle.init` (if exported).
    pub(crate) func_init: Option<wasmtime::component::Func>,

    /// Pre-resolved function handle for `lifecycle.teardown` (if exported).
    pub(crate) func_teardown: Option<wasmtime::component::Func>,
}

/// Host state stored in each Wasmtime `Store`.
///
/// This is the `T` in `Store<T>`. It provides Torvyn-specific context
/// that host-defined functions can access during component execution.
#[cfg(feature = "wasmtime-backend")]
pub(crate) struct HostState {
    /// The component ID for this instance. Used by host trait impls
    /// when Torvyn registers host-defined resources.
    #[allow(dead_code)]
    pub(crate) component_id: ComponentId,

    /// Resource limits for this store.
    pub(crate) limits: wasmtime::StoreLimits,

    /// The fuel budget configured for this component. Tracked for
    /// observability/diagnostics.
    #[allow(dead_code)]
    pub(crate) fuel_budget: u64,
}

/// Mock instance state for testing.
#[cfg(feature = "mock")]
pub(crate) struct MockInstanceState {
    /// Component ID. Retained for diagnostic/logging use.
    #[allow(dead_code)]
    pub(crate) component_id: ComponentId,
    /// Remaining fuel.
    pub(crate) fuel: u64,
    /// Simulated memory usage.
    pub(crate) memory_bytes: usize,
    /// How many calls have been made.
    pub(crate) call_count: u64,
    /// Configurable process response.
    pub(crate) process_response: Option<ProcessResult>,
    /// Configurable pull response.
    pub(crate) pull_response: Option<Option<OutputElement>>,
    /// Configurable push response.
    pub(crate) push_response: Option<BackpressureSignal>,
    /// If true, invocations will trap.
    pub(crate) should_trap: bool,
}

impl ComponentInstance {
    /// Returns the component ID for this instance.
    ///
    /// # HOT PATH — zero-cost accessor.
    #[inline]
    pub fn component_id(&self) -> ComponentId {
        self.component_id
    }

    /// Returns whether this component exports the lifecycle interface.
    #[inline]
    pub fn has_lifecycle(&self) -> bool {
        self.has_lifecycle
    }

    /// Returns whether this component exports the processor interface.
    #[inline]
    pub fn has_processor(&self) -> bool {
        self.has_processor
    }

    /// Returns whether this component exports the source interface.
    #[inline]
    pub fn has_source(&self) -> bool {
        self.has_source
    }

    /// Returns whether this component exports the sink interface.
    #[inline]
    pub fn has_sink(&self) -> bool {
        self.has_sink
    }
}

impl std::fmt::Debug for ComponentInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentInstance")
            .field("component_id", &self.component_id)
            .field("has_lifecycle", &self.has_lifecycle)
            .field("has_processor", &self.has_processor)
            .field("has_source", &self.has_source)
            .field("has_sink", &self.has_sink)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ImportBindings
// ---------------------------------------------------------------------------

/// Import bindings for component instantiation.
///
/// This type carries the resolved imports that satisfy a component's
/// import requirements. In the Wasmtime backend, this wraps a
/// configured `wasmtime::component::Linker`.
///
/// # COLD PATH — constructed once during pipeline linking.
pub struct ImportBindings {
    /// Backend-specific import bindings.
    pub(crate) inner: ImportBindingsInner,
}

/// Backend-specific import bindings.
#[allow(dead_code, clippy::large_enum_variant)]
pub(crate) enum ImportBindingsInner {
    /// Wasmtime linker.
    #[cfg(feature = "wasmtime-backend")]
    Wasmtime(wasmtime::component::Linker<HostState>),
    /// Mock bindings for testing.
    #[cfg(feature = "mock")]
    Mock,
    /// Placeholder.
    #[allow(dead_code)]
    _Placeholder,
}

impl std::fmt::Debug for ImportBindings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImportBindings")
    }
}

// ---------------------------------------------------------------------------
// StreamElement
// ---------------------------------------------------------------------------

/// A stream element as seen by the engine layer.
///
/// This is the Rust representation of the WIT `stream-element` record
/// that is marshaled into Wasm Component Model types before invocation.
///
/// # HOT PATH — passed per element through ComponentInvoker.
///
/// # Examples
/// ```
/// use torvyn_types::{BufferHandle, ElementMeta, ResourceId};
/// use torvyn_engine::StreamElement;
///
/// let meta = ElementMeta::new(0, 1_000_000, "application/json".into());
/// let handle = BufferHandle::new(ResourceId::new(5, 0));
/// let element = StreamElement { meta, payload: handle };
/// assert_eq!(element.meta.sequence, 0);
/// ```
#[derive(Clone, Debug)]
pub struct StreamElement {
    /// Metadata for this stream element.
    pub meta: ElementMeta,

    /// Handle to the buffer containing the element's payload.
    pub payload: BufferHandle,
}

// ---------------------------------------------------------------------------
// OutputElement
// ---------------------------------------------------------------------------

/// An output element produced by a component.
///
/// Returned by source `pull` and processor `process` invocations.
///
/// # HOT PATH — returned per element from ComponentInvoker.
#[derive(Clone, Debug)]
pub struct OutputElement {
    /// Metadata for the output element.
    pub meta: ElementMeta,

    /// Handle to the output buffer.
    pub payload: BufferHandle,
}

// ---------------------------------------------------------------------------
// ProcessResult
// ---------------------------------------------------------------------------

/// Result of a processor component's `process` invocation.
///
/// # HOT PATH — returned per element from `invoke_process`.
#[derive(Clone, Debug)]
pub enum ProcessResult {
    /// The processor produced a transformed output element.
    Output(OutputElement),

    /// The processor filtered out the element (no output produced).
    Filtered,

    /// The processor produced multiple output elements (fan-out).
    Multiple(Vec<OutputElement>),
}

impl ProcessResult {
    /// Returns `true` if the processor produced output.
    ///
    /// # HOT PATH
    #[inline]
    pub fn has_output(&self) -> bool {
        !matches!(self, ProcessResult::Filtered)
    }

    /// Returns the number of output elements.
    ///
    /// # HOT PATH
    #[inline]
    pub fn output_count(&self) -> usize {
        match self {
            ProcessResult::Output(_) => 1,
            ProcessResult::Filtered => 0,
            ProcessResult::Multiple(v) => v.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// InvocationResult
// ---------------------------------------------------------------------------

/// Metadata about a completed invocation, for observability.
///
/// # HOT PATH — created after every invocation.
#[derive(Clone, Debug)]
pub struct InvocationResult {
    /// Component that was invoked.
    pub component_id: ComponentId,
    /// Function that was called.
    pub function_name: &'static str,
    /// Fuel consumed by this invocation.
    pub fuel_consumed: u64,
    /// Wall-clock duration of the invocation in nanoseconds.
    pub duration_ns: u64,
    /// Whether the invocation succeeded.
    pub success: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::{BufferHandle, ElementMeta, ResourceId};

    #[test]
    fn test_stream_element_creation() {
        let meta = ElementMeta::new(0, 1000, "text/plain".into());
        let handle = BufferHandle::new(ResourceId::new(1, 0));
        let elem = StreamElement {
            meta,
            payload: handle,
        };
        assert_eq!(elem.meta.sequence, 0);
        assert_eq!(elem.payload.resource_id().index(), 1);
    }

    #[test]
    fn test_process_result_output() {
        let meta = ElementMeta::new(1, 2000, "text/plain".into());
        let handle = BufferHandle::new(ResourceId::new(2, 0));
        let result = ProcessResult::Output(OutputElement {
            meta,
            payload: handle,
        });
        assert!(result.has_output());
        assert_eq!(result.output_count(), 1);
    }

    #[test]
    fn test_process_result_filtered() {
        let result = ProcessResult::Filtered;
        assert!(!result.has_output());
        assert_eq!(result.output_count(), 0);
    }

    #[test]
    fn test_process_result_multiple() {
        let outputs = vec![
            OutputElement {
                meta: ElementMeta::new(0, 100, "a".into()),
                payload: BufferHandle::new(ResourceId::new(0, 0)),
            },
            OutputElement {
                meta: ElementMeta::new(1, 200, "b".into()),
                payload: BufferHandle::new(ResourceId::new(1, 0)),
            },
        ];
        let result = ProcessResult::Multiple(outputs);
        assert!(result.has_output());
        assert_eq!(result.output_count(), 2);
    }

    #[test]
    fn test_component_instance_debug() {
        let id = ComponentId::new(42);
        let debug_str = format!("component_id: {:?}", id);
        assert!(debug_str.contains("42"));
    }

    #[test]
    fn test_compiled_component_debug() {
        // Ensure CompiledComponent's Debug doesn't panic with _Placeholder.
        let cc = CompiledComponent {
            inner: CompiledComponentInner::_Placeholder,
        };
        let s = format!("{:?}", cc);
        assert!(s.contains("CompiledComponent"));
    }

    #[test]
    fn test_import_bindings_debug() {
        let ib = ImportBindings {
            inner: ImportBindingsInner::_Placeholder,
        };
        let s = format!("{:?}", ib);
        assert!(s.contains("ImportBindings"));
    }

    #[test]
    fn test_invocation_result() {
        let result = InvocationResult {
            component_id: ComponentId::new(1),
            function_name: "process",
            fuel_consumed: 100,
            duration_ns: 5000,
            success: true,
        };
        assert!(result.success);
        assert_eq!(result.fuel_consumed, 100);
    }
}
