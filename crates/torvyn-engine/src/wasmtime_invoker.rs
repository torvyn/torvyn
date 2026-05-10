//! Wasmtime-based implementation of the [`ComponentInvoker`] trait.
//!
//! **THIS IS THE HOTTEST PATH IN TORVYN.** Every stream element passes
//! through this code. Design goals:
//! - Zero heap allocations beyond what the bindgen-typed call requires.
//! - No locks acquired during invocation.
//! - No syscalls beyond the Wasm execution itself.
//!
//! As of Session 2.1 the invoker uses `wasmtime::component::bindgen!`-typed
//! accessors instead of the flat `Val` array marshaling used in Phase 0. The
//! bindgen-generated `call_*` functions consume strongly-typed records and
//! return strongly-typed results; this module is mostly a thin translator
//! between those types and Torvyn's domain model in `torvyn_types`.
//!
//! # LLI DEVIATIONS from LLI-04 (adapted per spike findings)
//! - `post_return_async()` calls removed: deprecated no-op in Wasmtime 42
//!   (spike finding 3.7); the bindgen-typed `call_*` functions handle
//!   post-return automatically.
//! - `wasmtime::Error` is distinct from `anyhow::Error` (spike finding 3.5).
//!
//! This module is gated behind the `wasmtime-backend` feature flag.

use async_trait::async_trait;
use wasmtime::component::Resource;
use wasmtime::Trap;

use torvyn_types::{
    BackpressureSignal, BufferHandle, ComponentId, ElementMeta, ProcessError, ResourceId,
};

use crate::host_state::HostBuffer;
use crate::traits::ComponentInvoker;
use crate::types::{
    ComponentInstance, ComponentInstanceInner, OutputElement, ProcessResult, StreamElement,
    WasmtimeInstanceState, WitBindings,
};
use crate::wit_bindings::data_sink::torvyn::streaming::types as wit_types;

/// Wasmtime-based component invoker.
///
/// Dispatches each hot-path call to the bindgen-typed wrapper that was
/// detected at instantiation time (stored on the private
/// `WasmtimeInstanceState`). The dispatch enum match is fully
/// monomorphised; there is no virtual dispatch in the hot path.
pub struct WasmtimeInvoker {
    _preallocated: (),
}

impl WasmtimeInvoker {
    /// Create a new `WasmtimeInvoker`.
    ///
    /// # COLD PATH — called once at host startup.
    pub fn new() -> Self {
        Self { _preallocated: () }
    }

    /// Extract the Wasmtime instance state from a [`ComponentInstance`].
    ///
    /// # HOT PATH — inlined helper.
    #[inline]
    fn wasmtime_state(
        instance: &mut ComponentInstance,
    ) -> Result<&mut WasmtimeInstanceState, ProcessError> {
        match &mut instance.inner {
            ComponentInstanceInner::Wasmtime(state) => Ok(state),
            _ => Err(ProcessError::Internal(
                "WasmtimeInvoker called with non-Wasmtime instance".into(),
            )),
        }
    }

    /// Convert a Wasmtime trap or error into a `ProcessError`.
    ///
    /// # WARM PATH — called per error.
    // LLI DEVIATION: wasmtime::Error is distinct from anyhow::Error in v42
    // (spike finding 3.5). We use downcast_ref::<Trap> for trap detection.
    fn convert_wasm_error(
        err: wasmtime::Error,
        component_id: ComponentId,
        function_name: &str,
    ) -> ProcessError {
        if let Some(trap) = err.downcast_ref::<Trap>() {
            match trap {
                Trap::OutOfFuel => ProcessError::DeadlineExceeded,
                _ => ProcessError::Fatal(format!(
                    "Component {component_id} trapped in '{function_name}': {trap}"
                )),
            }
        } else {
            ProcessError::Internal(format!(
                "Component {component_id} error in '{function_name}': {err}"
            ))
        }
    }

    /// Translate the WIT-level `process-error` variant to the Torvyn domain
    /// type. The two enums share variant names; this is a straight 1:1 map.
    ///
    /// # WARM PATH — called per error.
    fn convert_wit_process_error(err: wit_types::ProcessError) -> ProcessError {
        match err {
            wit_types::ProcessError::InvalidInput(s) => ProcessError::InvalidInput(s),
            wit_types::ProcessError::Unavailable(s) => ProcessError::Unavailable(s),
            wit_types::ProcessError::Internal(s) => ProcessError::Internal(s),
            wit_types::ProcessError::DeadlineExceeded => ProcessError::DeadlineExceeded,
            wit_types::ProcessError::Fatal(s) => ProcessError::Fatal(s),
        }
    }

    /// Translate a WIT `output-element` produced by the guest into Torvyn's
    /// [`OutputElement`]. The owned buffer resource stays in the host's
    /// resource table; the table index is encoded as a [`BufferHandle`].
    ///
    /// # HOT PATH — called per element produced by a source/processor.
    #[inline]
    fn convert_output_element(out: wit_types::OutputElement) -> OutputElement {
        let payload_idx = out.payload.rep();
        OutputElement {
            meta: ElementMeta::new(
                out.meta.sequence,
                out.meta.timestamp_ns,
                out.meta.content_type,
            ),
            payload: BufferHandle::new(ResourceId::new(payload_idx, 0)),
        }
    }

    /// Translate a WIT `backpressure-signal` to the Torvyn domain enum.
    #[inline]
    fn convert_backpressure_signal(s: wit_types::BackpressureSignal) -> BackpressureSignal {
        match s {
            wit_types::BackpressureSignal::Ready => BackpressureSignal::Ready,
            wit_types::BackpressureSignal::Pause => BackpressureSignal::Pause,
        }
    }

    /// Build a borrowed `Resource<HostBuffer>` referring to the table entry
    /// at `handle.resource_id().index()`. The resource is borrowed (not
    /// owned), so the host retains responsibility for cleaning the table
    /// entry up after the call returns.
    ///
    /// # HOT PATH — called per element entering a processor/sink.
    #[inline]
    fn borrow_buffer(handle: BufferHandle) -> Resource<HostBuffer> {
        Resource::new_borrow(handle.resource_id().index())
    }

    /// Build a borrowed flow-context resource. Session 2.1 uses index 0 as
    /// a placeholder — the real flow context is plumbed through by the
    /// reactor in Session 2.2.
    #[inline]
    fn borrow_flow_context() -> Resource<crate::host_state::HostFlowContext> {
        Resource::new_borrow(0)
    }

    /// Build the bindgen `stream-element` record from a Torvyn
    /// [`StreamElement`]. The payload is passed as a borrowed buffer
    /// resource; the runtime retains ownership of the underlying host
    /// buffer.
    #[inline]
    fn to_wit_stream_element(element: &StreamElement) -> wit_types::StreamElement {
        wit_types::StreamElement {
            meta: wit_types::ElementMeta {
                sequence: element.meta.sequence,
                timestamp_ns: element.meta.timestamp_ns,
                content_type: element.meta.content_type.clone(),
            },
            payload: Self::borrow_buffer(element.payload),
            context: Self::borrow_flow_context(),
        }
    }
}

impl Default for WasmtimeInvoker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComponentInvoker for WasmtimeInvoker {
    /// # HOT PATH
    async fn invoke_pull(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
    ) -> Result<Option<OutputElement>, ProcessError> {
        let state = Self::wasmtime_state(instance)?;
        let bindings = match &state.bindings {
            Some(WitBindings::DataSource(b)) => b,
            _ => {
                return Err(ProcessError::Internal(format!(
                    "Component {component_id} is not a data-source — `pull` is not exported"
                )));
            }
        };

        let outer = bindings
            .torvyn_streaming_source()
            .call_pull(&mut state.store)
            .await
            .map_err(|e| Self::convert_wasm_error(e, component_id, "pull"))?;

        match outer {
            Ok(Some(out)) => Ok(Some(Self::convert_output_element(out))),
            Ok(None) => Ok(None),
            Err(e) => Err(Self::convert_wit_process_error(e)),
        }
    }

    /// # HOT PATH
    async fn invoke_process(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        element: StreamElement,
    ) -> Result<ProcessResult, ProcessError> {
        let state = Self::wasmtime_state(instance)?;
        let wit_element = Self::to_wit_stream_element(&element);

        let outer = match &state.bindings {
            Some(WitBindings::Transform(b)) => {
                b.torvyn_streaming_processor()
                    .call_process(&mut state.store, &wit_element)
                    .await
            }
            Some(WitBindings::ManagedTransform(b)) => {
                b.torvyn_streaming_processor()
                    .call_process(&mut state.store, &wit_element)
                    .await
            }
            _ => {
                return Err(ProcessError::Internal(format!(
                    "Component {component_id} does not export `process`"
                )));
            }
        };

        let inner = outer.map_err(|e| Self::convert_wasm_error(e, component_id, "process"))?;
        match inner {
            Ok(wit_types::ProcessResult::Emit(out)) => {
                Ok(ProcessResult::Output(Self::convert_output_element(out)))
            }
            Ok(wit_types::ProcessResult::Drop) => Ok(ProcessResult::Filtered),
            Err(e) => Err(Self::convert_wit_process_error(e)),
        }
    }

    /// # HOT PATH
    async fn invoke_push(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        element: StreamElement,
    ) -> Result<BackpressureSignal, ProcessError> {
        let state = Self::wasmtime_state(instance)?;
        let bindings = match &state.bindings {
            Some(WitBindings::DataSink(b)) => b,
            _ => {
                return Err(ProcessError::Internal(format!(
                    "Component {component_id} is not a data-sink — `push` is not exported"
                )));
            }
        };
        let wit_element = Self::to_wit_stream_element(&element);

        let outer = bindings
            .torvyn_streaming_sink()
            .call_push(&mut state.store, &wit_element)
            .await
            .map_err(|e| Self::convert_wasm_error(e, component_id, "push"))?;

        match outer {
            Ok(s) => Ok(Self::convert_backpressure_signal(s)),
            Err(e) => Err(Self::convert_wit_process_error(e)),
        }
    }

    /// # COLD PATH
    async fn invoke_init(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        config: &str,
    ) -> Result<(), ProcessError> {
        let state = Self::wasmtime_state(instance)?;

        let outer = match &state.bindings {
            Some(WitBindings::DataSink(b)) => {
                b.torvyn_streaming_lifecycle()
                    .call_init(&mut state.store, config)
                    .await
            }
            Some(WitBindings::DataSource(b)) => {
                b.torvyn_streaming_lifecycle()
                    .call_init(&mut state.store, config)
                    .await
            }
            Some(WitBindings::ManagedTransform(b)) => {
                b.torvyn_streaming_lifecycle()
                    .call_init(&mut state.store, config)
                    .await
            }
            // `transform` has no lifecycle export, and a component with no
            // recognised bindings exports nothing — both cases are silent
            // success per the trait contract (init is optional).
            Some(WitBindings::Transform(_)) | None => return Ok(()),
        };

        let inner = outer.map_err(|e| Self::convert_wasm_error(e, component_id, "init"))?;
        inner.map_err(Self::convert_wit_process_error)
    }

    /// # COLD PATH
    ///
    /// Per C02-10: failures are logged but do not prevent termination.
    async fn invoke_teardown(&self, instance: &mut ComponentInstance, component_id: ComponentId) {
        let state = match Self::wasmtime_state(instance) {
            Ok(s) => s,
            Err(_) => return,
        };

        let outer = match &state.bindings {
            Some(WitBindings::DataSink(b)) => {
                b.torvyn_streaming_lifecycle()
                    .call_teardown(&mut state.store)
                    .await
            }
            Some(WitBindings::DataSource(b)) => {
                b.torvyn_streaming_lifecycle()
                    .call_teardown(&mut state.store)
                    .await
            }
            Some(WitBindings::ManagedTransform(b)) => {
                b.torvyn_streaming_lifecycle()
                    .call_teardown(&mut state.store)
                    .await
            }
            Some(WitBindings::Transform(_)) | None => return,
        };

        if let Err(e) = outer {
            #[cfg(feature = "tracing-support")]
            tracing::warn!(
                component_id = %component_id,
                error = %e,
                "Component teardown failed"
            );
            let _ = (component_id, e);
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
    fn test_invoker_creation() {
        let _invoker = WasmtimeInvoker::new();
    }

    #[test]
    fn test_invoker_default() {
        let _invoker = WasmtimeInvoker::default();
    }

    #[test]
    fn test_convert_wasm_error_fuel_exhaustion() {
        let err = wasmtime::Error::from(Trap::OutOfFuel);
        let process_err = WasmtimeInvoker::convert_wasm_error(err, ComponentId::new(1), "process");
        assert!(matches!(process_err, ProcessError::DeadlineExceeded));
    }

    #[test]
    fn test_convert_wasm_error_trap() {
        let err = wasmtime::Error::from(Trap::UnreachableCodeReached);
        let process_err = WasmtimeInvoker::convert_wasm_error(err, ComponentId::new(1), "process");
        assert!(matches!(process_err, ProcessError::Fatal(_)));
    }

    #[test]
    fn test_convert_wit_process_error_invalid_input() {
        let pe = WasmtimeInvoker::convert_wit_process_error(wit_types::ProcessError::InvalidInput(
            "bad input".into(),
        ));
        match pe {
            ProcessError::InvalidInput(s) => assert_eq!(s, "bad input"),
            other => panic!("expected InvalidInput, got: {other:?}"),
        }
    }

    #[test]
    fn test_convert_wit_process_error_deadline() {
        let pe =
            WasmtimeInvoker::convert_wit_process_error(wit_types::ProcessError::DeadlineExceeded);
        assert!(matches!(pe, ProcessError::DeadlineExceeded));
    }

    #[test]
    fn test_convert_backpressure_ready() {
        let bp = WasmtimeInvoker::convert_backpressure_signal(wit_types::BackpressureSignal::Ready);
        assert_eq!(bp, BackpressureSignal::Ready);
    }

    #[test]
    fn test_convert_backpressure_pause() {
        let bp = WasmtimeInvoker::convert_backpressure_signal(wit_types::BackpressureSignal::Pause);
        assert_eq!(bp, BackpressureSignal::Pause);
    }

    #[test]
    fn test_convert_wit_process_error_all_variants() {
        let cases: [(wit_types::ProcessError, &str); 5] = [
            (
                wit_types::ProcessError::InvalidInput("bad".into()),
                "invalid_input",
            ),
            (
                wit_types::ProcessError::Unavailable("svc".into()),
                "unavailable",
            ),
            (wit_types::ProcessError::Internal("oops".into()), "internal"),
            (
                wit_types::ProcessError::DeadlineExceeded,
                "deadline_exceeded",
            ),
            (wit_types::ProcessError::Fatal("done".into()), "fatal"),
        ];
        for (wit_err, expected_kind) in cases {
            let pe = WasmtimeInvoker::convert_wit_process_error(wit_err);
            assert_eq!(
                pe.kind(),
                expected_kind,
                "kind() mismatch for translated variant"
            );
        }
    }

    #[test]
    fn test_convert_output_element_preserves_fields() {
        let out = wit_types::OutputElement {
            meta: wit_types::ElementMeta {
                sequence: 42,
                timestamp_ns: 1_234_567,
                content_type: "application/json".into(),
            },
            payload: Resource::new_own(7),
        };
        let converted = WasmtimeInvoker::convert_output_element(out);
        assert_eq!(converted.meta.sequence, 42);
        assert_eq!(converted.meta.timestamp_ns, 1_234_567);
        assert_eq!(converted.meta.content_type, "application/json");
        assert_eq!(converted.payload.resource_id().index(), 7);
    }

    #[test]
    fn test_to_wit_stream_element_borrows_payload() {
        let element = StreamElement {
            meta: ElementMeta::new(10, 2_000, "text/plain".into()),
            payload: BufferHandle::new(ResourceId::new(11, 0)),
        };
        let wit_element = WasmtimeInvoker::to_wit_stream_element(&element);
        assert_eq!(wit_element.meta.sequence, 10);
        assert_eq!(wit_element.meta.content_type, "text/plain");
        // `Resource::rep()` exposes the underlying handle index; a borrowed
        // resource preserves the index the host passed in.
        assert_eq!(wit_element.payload.rep(), 11);
    }
}
