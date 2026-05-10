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

use torvyn_resources::OwnerId;
use torvyn_types::{BackpressureSignal, ComponentId, ElementMeta, ProcessError};

use crate::host_state::{HostBuffer, HostState};
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
    /// [`OutputElement`], performing the Component → Host ownership
    /// transfer that hands the underlying buffer to the runtime.
    ///
    /// Steps:
    /// 1. Delete the per-store `HostBuffer` entry — the guest no longer
    ///    holds a handle to it (Component Model semantics: returning an
    ///    owned resource consumes the guest handle).
    /// 2. Look up the manager-wide [`BufferHandle`] and the buffer's
    ///    last-known owner.
    /// 3. Drive the manager's ownership state machine
    ///    `Owned(Component) → Transit → Owned(Host)` via
    ///    `transfer_ownership`.
    /// 4. Return the Torvyn [`OutputElement`] for the reactor.
    ///
    /// On any failure the buffer's manager-side state may be inconsistent;
    /// we propagate as `ProcessError::Internal` and rely on flow-level
    /// cleanup (`Drop for HostState`) to reclaim leaks.
    ///
    /// # HOT PATH — called per element produced by a source/processor.
    fn convert_output_element(
        state: &mut HostState,
        out: wit_types::OutputElement,
        component_id: ComponentId,
    ) -> Result<OutputElement, ProcessError> {
        let entry: HostBuffer = state.table.delete(out.payload).map_err(|e| {
            ProcessError::Internal(format!(
                "Component {component_id} returned an unknown buffer resource: {e}"
            ))
        })?;
        let from = entry.owner;
        let handle = entry.handle;
        state
            .resources
            .transfer_ownership(handle, from, OwnerId::Host)
            .map_err(|e| {
                ProcessError::Internal(format!(
                    "Component {component_id} → host buffer transfer failed: {e}"
                ))
            })?;
        Ok(OutputElement {
            meta: ElementMeta::new(
                out.meta.sequence,
                out.meta.timestamp_ns,
                out.meta.content_type,
            ),
            payload: handle,
        })
    }

    /// Translate a WIT `backpressure-signal` to the Torvyn domain enum.
    #[inline]
    fn convert_backpressure_signal(s: wit_types::BackpressureSignal) -> BackpressureSignal {
        match s {
            wit_types::BackpressureSignal::Ready => BackpressureSignal::Ready,
            wit_types::BackpressureSignal::Pause => BackpressureSignal::Pause,
        }
    }

    /// Build a borrowed flow-context resource. Session 2.2 uses index 0 as
    /// a placeholder — the real flow context is plumbed through by the
    /// reactor in a later session.
    #[inline]
    fn borrow_flow_context() -> Resource<crate::host_state::HostFlowContext> {
        Resource::new_borrow(0)
    }

    /// Build the bindgen `stream-element` record from a Torvyn
    /// [`StreamElement`]. The host inserts a `HostBuffer` entry into the
    /// per-store resource table on the spot, recording that the buffer is
    /// currently `Owned(Host)` and being borrowed for the duration of the
    /// downstream call. The returned [`Resource<HostBuffer>`] is borrowed
    /// (the host retains the table entry); the caller is responsible for
    /// pairing this with `manager.borrow_start` / `borrow_end` calls in
    /// future sessions that exercise process / push end-to-end.
    fn to_wit_stream_element(
        state: &mut HostState,
        element: &StreamElement,
    ) -> wasmtime::Result<wit_types::StreamElement> {
        let wrapper = HostBuffer {
            handle: element.payload,
            owner: OwnerId::Host,
        };
        let resource = state
            .table
            .push(wrapper)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        let rep = resource.rep();
        Ok(wit_types::StreamElement {
            meta: wit_types::ElementMeta {
                sequence: element.meta.sequence,
                timestamp_ns: element.meta.timestamp_ns,
                content_type: element.meta.content_type.clone(),
            },
            payload: Resource::new_borrow(rep),
            context: Self::borrow_flow_context(),
        })
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
        let outer = {
            let bindings = match &state.bindings {
                Some(WitBindings::DataSource(b)) => b,
                _ => {
                    return Err(ProcessError::Internal(format!(
                        "Component {component_id} is not a data-source — `pull` is not exported"
                    )));
                }
            };
            bindings
                .torvyn_streaming_source()
                .call_pull(&mut state.store)
                .await
                .map_err(|e| Self::convert_wasm_error(e, component_id, "pull"))?
        };

        match outer {
            Ok(Some(out)) => {
                let host_state = state.store.data_mut();
                Self::convert_output_element(host_state, out, component_id).map(Some)
            }
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

        let wit_element = Self::to_wit_stream_element(state.store.data_mut(), &element)
            .map_err(|e| ProcessError::Internal(format!("stream-element marshal: {e}")))?;

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
                let host_state = state.store.data_mut();
                Self::convert_output_element(host_state, out, component_id)
                    .map(ProcessResult::Output)
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

        let wit_element = Self::to_wit_stream_element(state.store.data_mut(), &element)
            .map_err(|e| ProcessError::Internal(format!("stream-element marshal: {e}")))?;

        let outer = {
            let bindings = match &state.bindings {
                Some(WitBindings::DataSink(b)) => b,
                _ => {
                    return Err(ProcessError::Internal(format!(
                        "Component {component_id} is not a data-sink — `push` is not exported"
                    )));
                }
            };
            bindings
                .torvyn_streaming_sink()
                .call_push(&mut state.store, &wit_element)
                .await
                .map_err(|e| Self::convert_wasm_error(e, component_id, "push"))?
        };

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

    #[tokio::test]
    async fn test_convert_output_element_transfers_ownership_to_host() {
        use std::sync::Arc;
        use torvyn_resources::DefaultResourceManager;
        use torvyn_types::{FlowId, ResourceState};

        let resources = Arc::new(DefaultResourceManager::new_for_testing());
        let cid = ComponentId::new(7);
        let flow = FlowId::new(7);
        let mut host_state = HostState::new(
            cid,
            wasmtime::StoreLimitsBuilder::new().build(),
            1_000_000,
            Arc::clone(&resources),
            flow,
        );
        let component_owner = OwnerId::Component(cid);

        // Simulate a buffer the guest just allocated and is about to return
        // as part of an output-element.
        let handle = resources
            .allocate(component_owner, 256, flow)
            .expect("allocate");
        let wrapper = HostBuffer {
            handle,
            owner: component_owner,
        };
        let resource = host_state.table.push(wrapper).expect("push");
        let wit_out = wit_types::OutputElement {
            meta: wit_types::ElementMeta {
                sequence: 42,
                timestamp_ns: 1_234_567,
                content_type: "application/json".into(),
            },
            payload: resource,
        };

        let converted =
            WasmtimeInvoker::convert_output_element(&mut host_state, wit_out, cid).expect("ok");

        assert_eq!(converted.meta.sequence, 42);
        assert_eq!(converted.meta.timestamp_ns, 1_234_567);
        assert_eq!(converted.meta.content_type, "application/json");
        assert_eq!(converted.payload, handle);

        let info = resources.inspect(handle).expect("inspect");
        assert_eq!(
            info.owner,
            OwnerId::Host,
            "buffer must now be owned by the host"
        );
        assert_eq!(info.state, ResourceState::Owned);
    }

    #[tokio::test]
    async fn test_to_wit_stream_element_inserts_host_borrow() {
        use std::sync::Arc;
        use torvyn_resources::DefaultResourceManager;
        use torvyn_types::{BufferHandle, FlowId, ResourceId};

        let resources = Arc::new(DefaultResourceManager::new_for_testing());
        let cid = ComponentId::new(7);
        let mut host_state = HostState::new(
            cid,
            wasmtime::StoreLimitsBuilder::new().build(),
            1_000_000,
            Arc::clone(&resources),
            FlowId::new(7),
        );

        let handle = BufferHandle::new(ResourceId::new(99, 1));
        let element = StreamElement {
            meta: ElementMeta::new(10, 2_000, "text/plain".into()),
            payload: handle,
        };
        let wit_element =
            WasmtimeInvoker::to_wit_stream_element(&mut host_state, &element).expect("ok");

        assert_eq!(wit_element.meta.sequence, 10);
        assert_eq!(wit_element.meta.content_type, "text/plain");
        // The host pushed an entry into the per-store table for this call.
        let rep = wit_element.payload.rep();
        let entry = host_state
            .table
            .get(&wasmtime::component::Resource::<HostBuffer>::new_borrow(
                rep,
            ))
            .expect("table entry exists");
        assert_eq!(entry.handle, handle, "BufferHandle preserved on insert");
        assert_eq!(entry.owner, OwnerId::Host, "host-owned for the borrow");
    }
}
