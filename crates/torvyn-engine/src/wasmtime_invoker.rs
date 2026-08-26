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

    /// Reset a component's fuel to its per-invocation budget.
    ///
    /// Called before every guest entry point. `default_fuel` is documented as a
    /// budget *per component invocation*: it bounds how much a single call may
    /// execute before Wasmtime preempts it. Wasmtime's fuel counter is
    /// monotonically consumed and is never replenished on its own — the async
    /// yield interval only yields, it does not add fuel — so without this reset
    /// the initial allocation would act as a per-*lifetime* cap. A healthy
    /// component would then trap with `Trap::OutOfFuel` once its cumulative
    /// consumption crossed the budget, ending a long-running flow after a
    /// bounded number of elements.
    ///
    /// A zero budget means fuel budgeting is disabled for this store, in which
    /// case this is a no-op.
    ///
    /// # HOT PATH — one `set_fuel` per invocation (a single store field write).
    #[inline]
    fn refuel(state: &mut WasmtimeInstanceState) {
        let budget = state.store.data().fuel_budget;
        if budget == 0 {
            return;
        }

        // `set_fuel` fails only when the engine was built without
        // `consume_fuel`; `create_store` sets the budget to zero in exactly
        // that case, so a non-zero budget cannot fail here.
        let refuelled = state.store.set_fuel(budget);
        debug_assert!(
            refuelled.is_ok(),
            "a non-zero fuel budget implies the engine enabled consume_fuel",
        );
        let _ = refuelled;
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
    /// we propagate as `ProcessError::Internal` and rely on the reactor's
    /// terminal-flow sweep (`reclaim_flow_buffers`, which walks the flow
    /// index) to reclaim what is left behind.
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

    /// The WIT form of a backpressure signal, for handing one *to* a guest.
    ///
    /// # WARM PATH — one per backpressure transition.
    #[inline]
    fn to_wit_backpressure_signal(s: BackpressureSignal) -> wit_types::BackpressureSignal {
        match s {
            BackpressureSignal::Ready => wit_types::BackpressureSignal::Ready,
            BackpressureSignal::Pause => wit_types::BackpressureSignal::Pause,
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
    /// downstream call. The handle handed to the guest is a *borrow*; the
    /// host retains the owning table entry.
    ///
    /// Returns the marshalled element together with the `rep` of that
    /// owning entry. The caller **must** pass the `rep` to
    /// [`Self::reclaim_input_element`] once the guest call returns —
    /// otherwise both the table entry and the underlying pool buffer leak,
    /// once per element, for the lifetime of the flow.
    fn to_wit_stream_element(
        state: &mut HostState,
        element: &StreamElement,
    ) -> wasmtime::Result<(wit_types::StreamElement, u32)> {
        let wrapper = HostBuffer {
            handle: element.payload,
            owner: OwnerId::Host,
        };
        let resource = state
            .table
            .push(wrapper)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        let rep = resource.rep();
        Ok((
            wit_types::StreamElement {
                meta: wit_types::ElementMeta {
                    sequence: element.meta.sequence,
                    timestamp_ns: element.meta.timestamp_ns,
                    content_type: element.meta.content_type.clone(),
                },
                payload: Resource::new_borrow(rep),
                context: Self::borrow_flow_context(),
            },
            rep,
        ))
    }

    /// Reclaim the input element handed to a `process` / `push` call.
    ///
    /// Deletes the owning `HostBuffer` entry that
    /// [`Self::to_wit_stream_element`] pushed into the store's resource
    /// table, then returns the underlying buffer to the pool. This is the
    /// point at which a consumed element's memory is actually freed: the
    /// guest received only a borrow, so no guest-side drop ever fires for
    /// it, and the buffer is owned by the host rather than by any
    /// component, so component-keyed cleanup cannot see it either.
    ///
    /// # Preconditions
    /// The runtime delivers each element to exactly one consumer, so the
    /// buffer is dead once the call returns. Broadcast fan-out — the same
    /// buffer handed to several consumers — would need reference counting
    /// before this is called.
    ///
    /// # HOT PATH — called once per element per stage.
    fn reclaim_input_element(
        state: &mut HostState,
        input_rep: u32,
        component_id: ComponentId,
    ) -> Result<(), ProcessError> {
        let entry: HostBuffer = state
            .table
            .delete(Resource::<HostBuffer>::new_own(input_rep))
            .map_err(|e| {
                ProcessError::Internal(format!(
                    "Component {component_id}: input buffer resource missing at reclaim: {e}"
                ))
            })?;
        state
            .resources
            .release(entry.handle, entry.owner)
            .map_err(|e| {
                ProcessError::Internal(format!(
                    "Component {component_id}: input buffer release failed: {e}"
                ))
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
        Self::refuel(state);
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
        Self::refuel(state);

        // Reject unsupported archetypes before marshalling, so no
        // resource-table entry is created for a call that cannot be made.
        if !matches!(
            state.bindings,
            Some(WitBindings::Transform(_)) | Some(WitBindings::ManagedTransform(_))
        ) {
            return Err(ProcessError::Internal(format!(
                "Component {component_id} does not export `process`"
            )));
        }

        let (wit_element, input_rep) =
            Self::to_wit_stream_element(state.store.data_mut(), &element)
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
            _ => unreachable!("archetype was checked before marshalling"),
        };

        // The guest's borrow ends when the call returns, so reclaim the
        // input on every path — a component error must not leak the
        // element that caused it. Reclaim before interpreting the result,
        // so a guest that returns the input's own `rep` as its "owned"
        // output finds the entry already gone rather than double-freeing.
        let reclaimed =
            Self::reclaim_input_element(state.store.data_mut(), input_rep, component_id);

        let inner = outer.map_err(|e| Self::convert_wasm_error(e, component_id, "process"))?;
        reclaimed?;

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
        Self::refuel(state);

        // Reject unsupported archetypes before marshalling, so no
        // resource-table entry is created for a call that cannot be made.
        if !matches!(state.bindings, Some(WitBindings::DataSink(_))) {
            return Err(ProcessError::Internal(format!(
                "Component {component_id} is not a data-sink — `push` is not exported"
            )));
        }

        let (wit_element, input_rep) =
            Self::to_wit_stream_element(state.store.data_mut(), &element)
                .map_err(|e| ProcessError::Internal(format!("stream-element marshal: {e}")))?;

        let outer = match &state.bindings {
            Some(WitBindings::DataSink(b)) => {
                b.torvyn_streaming_sink()
                    .call_push(&mut state.store, &wit_element)
                    .await
            }
            _ => unreachable!("archetype was checked before marshalling"),
        };

        // A sink is the end of the line: nothing downstream can observe the
        // element, so this is where its buffer returns to the pool. Reclaim
        // on both the success and failure paths.
        let reclaimed =
            Self::reclaim_input_element(state.store.data_mut(), input_rep, component_id);

        let inner = outer.map_err(|e| Self::convert_wasm_error(e, component_id, "push"))?;
        reclaimed?;

        match inner {
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
        Self::refuel(state);

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
    /// # WARM PATH — one per backpressure transition, not per element.
    async fn invoke_notify_backpressure(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        signal: BackpressureSignal,
    ) -> Result<(), ProcessError> {
        let state = Self::wasmtime_state(instance)?;

        Self::refuel(state);

        // Only a source exports this. Anything else is a stage the driver
        // should not have selected, and silently doing nothing is right: the
        // scheduler has already applied the backpressure that matters.
        let outer = {
            let Some(WitBindings::DataSource(bindings)) = &state.bindings else {
                // Not a source, so it exports no hook. This is not a failure:
                // the scheduler has already applied the backpressure that
                // matters, and the driver only calls this for source stages.
                return Ok(());
            };
            bindings
                .torvyn_streaming_source()
                .call_notify_backpressure(
                    &mut state.store,
                    Self::to_wit_backpressure_signal(signal),
                )
                .await
        };

        outer.map_err(|e| Self::convert_wasm_error(e, component_id, "notify-backpressure"))
    }

    async fn invoke_teardown(&self, instance: &mut ComponentInstance, component_id: ComponentId) {
        let state = match Self::wasmtime_state(instance) {
            Ok(s) => s,
            Err(_) => return,
        };
        Self::refuel(state);

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

    /// Build a real (export-less) Wasmtime instance for fuel tests.
    ///
    /// The empty `(component)` exports nothing, so a guest entry point fails
    /// *after* the refuel step — which is exactly what lets these tests observe
    /// refuelling in isolation from any guest execution.
    #[cfg(feature = "wasmtime-backend")]
    async fn instance_with_config(
        config: crate::WasmtimeEngineConfig,
    ) -> (crate::WasmtimeEngine, ComponentInstance) {
        use crate::traits::WasmEngine;
        use crate::types::{CompiledComponent, CompiledComponentInner};
        use crate::ComponentLimits;
        use torvyn_security::WasiConfiguration;
        use wasmtime::component::Component;

        let engine = crate::WasmtimeEngine::new(config).expect("engine must initialise");
        let component = Component::new(engine.inner(), "(component)").expect("compile WAT");
        let compiled = CompiledComponent {
            inner: CompiledComponentInner::Wasmtime(component),
        };
        let imports = crate::WasmtimeEngine::import_bindings_from_linker(engine.create_linker());
        let instance = engine
            .instantiate(
                &compiled,
                imports,
                ComponentId::new(1),
                &WasiConfiguration::deny_all(),
                &ComponentLimits::inherit(),
            )
            .await
            .expect("instantiate must succeed");
        (engine, instance)
    }

    /// Every guest invocation must start from the full per-invocation fuel
    /// budget. Without this, `default_fuel` would act as a per-*lifetime* cap
    /// and a healthy component would trap with `Trap::OutOfFuel` once its
    /// cumulative consumption crossed the budget.
    #[tokio::test]
    async fn test_invocation_refuels_to_per_invocation_budget() {
        use crate::traits::WasmEngine;

        let config = crate::WasmtimeEngineConfig::default();
        let budget = config.default_fuel;
        let (engine, mut instance) = instance_with_config(config).await;

        // Simulate a component that has nearly exhausted its fuel.
        engine.set_fuel(&mut instance, 1).expect("set_fuel");
        assert_eq!(engine.fuel_remaining(&instance), Some(1));

        let invoker = WasmtimeInvoker::new();
        let _ = invoker
            .invoke_pull(&mut instance, ComponentId::new(1))
            .await;

        assert_eq!(
            engine.fuel_remaining(&instance),
            Some(budget),
            "each invocation must begin with the full per-invocation budget",
        );
    }

    /// Refuelling applies to every guest entry point, not just `pull`.
    #[tokio::test]
    async fn test_all_guest_entry_points_refuel() {
        use crate::traits::WasmEngine;

        let config = crate::WasmtimeEngineConfig::default();
        let budget = config.default_fuel;
        let (engine, mut instance) = instance_with_config(config).await;
        let invoker = WasmtimeInvoker::new();
        let cid = ComponentId::new(1);

        let element = || StreamElement {
            meta: ElementMeta::new(0, 0, String::new()),
            payload: torvyn_types::BufferHandle::new(torvyn_types::ResourceId::new(0, 0)),
        };

        // Drain fuel, invoke, and confirm the budget was restored — once per
        // entry point.
        engine.set_fuel(&mut instance, 1).expect("set_fuel");
        let _ = invoker.invoke_process(&mut instance, cid, element()).await;
        assert_eq!(engine.fuel_remaining(&instance), Some(budget), "process");

        engine.set_fuel(&mut instance, 1).expect("set_fuel");
        let _ = invoker.invoke_push(&mut instance, cid, element()).await;
        assert_eq!(engine.fuel_remaining(&instance), Some(budget), "push");

        engine.set_fuel(&mut instance, 1).expect("set_fuel");
        let _ = invoker.invoke_init(&mut instance, cid, "{}").await;
        assert_eq!(engine.fuel_remaining(&instance), Some(budget), "init");

        engine.set_fuel(&mut instance, 1).expect("set_fuel");
        invoker.invoke_teardown(&mut instance, cid).await;
        assert_eq!(engine.fuel_remaining(&instance), Some(budget), "teardown");
    }

    /// With fuel budgeting disabled the refuel step is a no-op and must not
    /// panic or attempt to set fuel on a store that does not track it.
    #[tokio::test]
    async fn test_no_refuel_when_fuel_disabled() {
        use crate::traits::WasmEngine;

        let config = crate::WasmtimeEngineConfig {
            fuel_enabled: false,
            ..crate::WasmtimeEngineConfig::default()
        };
        let (engine, mut instance) = instance_with_config(config).await;

        assert_eq!(
            engine.fuel_remaining(&instance),
            None,
            "fuel is not tracked when budgeting is disabled",
        );

        let invoker = WasmtimeInvoker::new();
        let _ = invoker
            .invoke_pull(&mut instance, ComponentId::new(1))
            .await;

        assert_eq!(engine.fuel_remaining(&instance), None);
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
            crate::host_state::deny_all_wasi_ctx(),
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
            crate::host_state::deny_all_wasi_ctx(),
        );

        let handle = BufferHandle::new(ResourceId::new(99, 1));
        let element = StreamElement {
            meta: ElementMeta::new(10, 2_000, "text/plain".into()),
            payload: handle,
        };
        let (wit_element, input_rep) =
            WasmtimeInvoker::to_wit_stream_element(&mut host_state, &element).expect("ok");

        assert_eq!(wit_element.meta.sequence, 10);
        assert_eq!(wit_element.meta.content_type, "text/plain");
        // The host pushed an entry into the per-store table for this call.
        let rep = wit_element.payload.rep();
        assert_eq!(
            rep, input_rep,
            "the returned rep must identify the entry backing the guest's borrow"
        );
        let entry = host_state
            .table
            .get(&wasmtime::component::Resource::<HostBuffer>::new_borrow(
                rep,
            ))
            .expect("table entry exists");
        assert_eq!(entry.handle, handle, "BufferHandle preserved on insert");
        assert_eq!(entry.owner, OwnerId::Host, "host-owned for the borrow");
    }

    /// Marshalling an element pushes an owning entry into the per-store
    /// resource table; reclaiming must remove it again. Without this the
    /// table grows by one entry per element for the lifetime of the flow,
    /// since the guest only ever receives a borrow and so never drops it.
    #[tokio::test]
    async fn test_reclaim_input_element_removes_table_entry_and_frees_buffer() {
        use std::sync::Arc;
        use torvyn_resources::DefaultResourceManager;
        use torvyn_types::FlowId;

        let resources = Arc::new(DefaultResourceManager::new_for_testing());
        let cid = ComponentId::new(11);
        let flow = FlowId::new(11);
        resources.register_flow(flow);

        let mut host_state = HostState::new(
            cid,
            wasmtime::StoreLimitsBuilder::new().build(),
            1_000_000,
            Arc::clone(&resources),
            flow,
            crate::host_state::deny_all_wasi_ctx(),
        );

        // A real host-owned buffer, as the reactor would hand to a stage.
        let handle = resources
            .allocate(OwnerId::Host, 64, flow)
            .expect("allocation must succeed");
        assert_eq!(resources.live_resource_count(), 1);

        let element = StreamElement {
            meta: ElementMeta::new(0, 0, "application/octet-stream".into()),
            payload: handle,
        };
        let (_wit_element, input_rep) =
            WasmtimeInvoker::to_wit_stream_element(&mut host_state, &element).expect("marshal");

        WasmtimeInvoker::reclaim_input_element(&mut host_state, input_rep, cid)
            .expect("reclaim must succeed for a host-owned buffer with no borrows");

        assert!(
            host_state
                .table
                .get(&wasmtime::component::Resource::<HostBuffer>::new_borrow(
                    input_rep
                ))
                .is_err(),
            "the per-invocation table entry must be gone",
        );
        assert_eq!(
            resources.live_resource_count(),
            0,
            "the buffer must be returned to the pool",
        );
    }

    /// Reclaiming twice must fail rather than double-free. This is the
    /// guard against a guest that returns the `rep` of its own borrowed
    /// input as an owned output.
    #[tokio::test]
    async fn test_reclaim_input_element_is_not_double_free() {
        use std::sync::Arc;
        use torvyn_resources::DefaultResourceManager;
        use torvyn_types::FlowId;

        let resources = Arc::new(DefaultResourceManager::new_for_testing());
        let cid = ComponentId::new(12);
        let flow = FlowId::new(12);
        resources.register_flow(flow);

        let mut host_state = HostState::new(
            cid,
            wasmtime::StoreLimitsBuilder::new().build(),
            1_000_000,
            Arc::clone(&resources),
            flow,
            crate::host_state::deny_all_wasi_ctx(),
        );

        let handle = resources
            .allocate(OwnerId::Host, 64, flow)
            .expect("allocation must succeed");
        let element = StreamElement {
            meta: ElementMeta::new(0, 0, String::new()),
            payload: handle,
        };
        let (_wit_element, input_rep) =
            WasmtimeInvoker::to_wit_stream_element(&mut host_state, &element).expect("marshal");

        WasmtimeInvoker::reclaim_input_element(&mut host_state, input_rep, cid).expect("first");
        let second = WasmtimeInvoker::reclaim_input_element(&mut host_state, input_rep, cid);

        assert!(second.is_err(), "second reclaim must be rejected");
        assert_eq!(resources.live_resource_count(), 0);
    }
}
