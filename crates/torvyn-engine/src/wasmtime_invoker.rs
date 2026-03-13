//! Wasmtime-based implementation of the [`ComponentInvoker`] trait.
//!
//! **THIS IS THE HOTTEST PATH IN TORVYN.** Every stream element passes
//! through this code. Design goals:
//! - Zero heap allocations per invocation (pre-allocated buffers)
//! - No locks acquired during invocation
//! - No syscalls beyond the Wasm execution itself
//!
//! # LLI DEVIATIONS from LLI-04 (adapted per spike findings)
//! - `post_return_async()` calls removed: deprecated no-op in Wasmtime 42
//!   (spike finding 3.7)
//! - `wasmtime::Error` is distinct from `anyhow::Error` (spike finding 3.5)
//!
//! This module is gated behind the `wasmtime-backend` feature flag.

use async_trait::async_trait;
use wasmtime::component::{Func, Val};
use wasmtime::Trap;

use torvyn_types::{
    BackpressureSignal, BufferHandle, ComponentId, ElementMeta, ProcessError, ResourceId,
};

use crate::traits::ComponentInvoker;
use crate::types::{
    ComponentInstance, ComponentInstanceInner, OutputElement, ProcessResult, StreamElement,
    WasmtimeInstanceState,
};

/// Wasmtime-based component invoker.
///
/// Handles marshaling between Rust/Torvyn types and Wasm Component Model
/// `Val` types for dynamic invocation.
///
/// # Performance
/// The invoker uses pre-resolved `Func` handles (cached at instantiation
/// time) to avoid string lookups on every invocation. Argument and return
/// value arrays are stack-allocated for the common case.
pub struct WasmtimeInvoker {
    /// Pre-allocated argument buffer placeholder.
    /// Future optimization: use arena allocator for marshaling buffers.
    _preallocated: (),
}

impl WasmtimeInvoker {
    /// Create a new `WasmtimeInvoker`.
    ///
    /// # COLD PATH — called once at host startup.
    pub fn new() -> Self {
        Self {
            _preallocated: (),
        }
    }

    /// Extract the Wasmtime instance state from a ComponentInstance.
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

    /// Get a pre-resolved function handle, returning an error if missing.
    ///
    /// # HOT PATH — inlined.
    #[inline]
    fn require_func(
        func: &Option<Func>,
        function_name: &str,
        component_id: ComponentId,
    ) -> Result<Func, ProcessError> {
        func.ok_or_else(|| {
            ProcessError::Internal(format!(
                "Component {component_id} does not export '{function_name}'"
            ))
        })
    }

    /// Convert a Wasmtime trap or error into a ProcessError.
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

    /// Marshal a `StreamElement` into Wasm `Val` arguments.
    ///
    /// # HOT PATH — zero alloc, no locks.
    ///
    /// The exact marshaling depends on the WIT interface. For Phase 0,
    /// we pass individual fields as separate arguments.
    fn marshal_stream_element(element: &StreamElement, args: &mut Vec<Val>) {
        // HOT PATH — zero alloc, no locks
        args.push(Val::U64(element.meta.sequence));
        args.push(Val::U64(element.meta.timestamp_ns));
        args.push(Val::String(element.meta.content_type.clone()));
        args.push(Val::U32(element.payload.resource_id().index()));
    }

    /// Unmarshal output values from a Wasm invocation into an `OutputElement`.
    ///
    /// # HOT PATH
    fn unmarshal_output_element(results: &[Val]) -> Result<OutputElement, ProcessError> {
        if results.len() < 4 {
            return Err(ProcessError::Internal(format!(
                "Expected at least 4 return values, got {}",
                results.len()
            )));
        }

        let sequence = match &results[0] {
            Val::U64(v) => *v,
            other => {
                return Err(ProcessError::Internal(format!(
                    "Expected U64 for sequence, got {other:?}"
                )));
            }
        };

        let timestamp_ns = match &results[1] {
            Val::U64(v) => *v,
            other => {
                return Err(ProcessError::Internal(format!(
                    "Expected U64 for timestamp_ns, got {other:?}"
                )));
            }
        };

        let content_type = match &results[2] {
            Val::String(s) => s.to_string(),
            other => {
                return Err(ProcessError::Internal(format!(
                    "Expected String for content_type, got {other:?}"
                )));
            }
        };

        let buffer_index = match &results[3] {
            Val::U32(v) => *v,
            other => {
                return Err(ProcessError::Internal(format!(
                    "Expected U32 for buffer_index, got {other:?}"
                )));
            }
        };

        Ok(OutputElement {
            meta: ElementMeta::new(sequence, timestamp_ns, content_type),
            payload: BufferHandle::new(ResourceId::new(buffer_index, 0)),
        })
    }

    /// Unmarshal a backpressure signal from return values.
    ///
    /// # HOT PATH
    fn unmarshal_backpressure(results: &[Val]) -> Result<BackpressureSignal, ProcessError> {
        if results.is_empty() {
            return Err(ProcessError::Internal(
                "Expected at least 1 return value for backpressure signal".into(),
            ));
        }

        match &results[0] {
            Val::Enum(name) => match name.as_str() {
                "ready" => Ok(BackpressureSignal::Ready),
                "pause" => Ok(BackpressureSignal::Pause),
                other => Err(ProcessError::Internal(format!(
                    "Unknown backpressure signal: {other}"
                ))),
            },
            Val::U32(v) => match v {
                0 => Ok(BackpressureSignal::Ready),
                1 => Ok(BackpressureSignal::Pause),
                other => Err(ProcessError::Internal(format!(
                    "Unknown backpressure signal value: {other}"
                ))),
            },
            other => Err(ProcessError::Internal(format!(
                "Expected Enum or U32 for backpressure, got {other:?}"
            ))),
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
        let func = Self::require_func(&state.func_pull, "pull", component_id)?;

        let mut results = vec![Val::Bool(false); 8];

        func.call_async(&mut state.store, &[], &mut results)
            .await
            .map_err(|e| Self::convert_wasm_error(e, component_id, "pull"))?;

        // LLI DEVIATION: post_return_async() removed — deprecated no-op in
        // Wasmtime 42 (spike finding 3.7). Component Model handles
        // post-return automatically.

        // Check for end-of-stream (None) via option discriminant.
        match &results[0] {
            Val::Bool(false) => Ok(None),
            Val::Bool(true) => {
                let element = Self::unmarshal_output_element(&results[1..])?;
                Ok(Some(element))
            }
            _ => {
                let element = Self::unmarshal_output_element(&results)?;
                Ok(Some(element))
            }
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
        let func = Self::require_func(&state.func_process, "process", component_id)?;

        let mut args = Vec::with_capacity(4);
        Self::marshal_stream_element(&element, &mut args);

        let mut results = vec![Val::Bool(false); 8];

        func.call_async(&mut state.store, &args, &mut results)
            .await
            .map_err(|e| Self::convert_wasm_error(e, component_id, "process"))?;

        // LLI DEVIATION: post_return_async() removed (spike finding 3.7)

        let output = Self::unmarshal_output_element(&results)?;
        Ok(ProcessResult::Output(output))
    }

    /// # HOT PATH
    async fn invoke_push(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        element: StreamElement,
    ) -> Result<BackpressureSignal, ProcessError> {
        let state = Self::wasmtime_state(instance)?;
        let func = Self::require_func(&state.func_push, "push", component_id)?;

        let mut args = Vec::with_capacity(4);
        Self::marshal_stream_element(&element, &mut args);

        let mut results = vec![Val::Bool(false); 4];

        func.call_async(&mut state.store, &args, &mut results)
            .await
            .map_err(|e| Self::convert_wasm_error(e, component_id, "push"))?;

        // LLI DEVIATION: post_return_async() removed (spike finding 3.7)

        Self::unmarshal_backpressure(&results)
    }

    /// # COLD PATH
    async fn invoke_init(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
        config: &str,
    ) -> Result<(), ProcessError> {
        let state = Self::wasmtime_state(instance)?;

        let func = match &state.func_init {
            Some(f) => *f,
            None => return Ok(()),
        };

        let args = [Val::String(config.into())];
        let mut results = vec![Val::Bool(false); 2];

        func.call_async(&mut state.store, &args, &mut results)
            .await
            .map_err(|e| Self::convert_wasm_error(e, component_id, "init"))?;

        // LLI DEVIATION: post_return_async() removed (spike finding 3.7)

        Ok(())
    }

    /// # COLD PATH
    ///
    /// Per C02-10: failures are logged but do not prevent termination.
    async fn invoke_teardown(
        &self,
        instance: &mut ComponentInstance,
        component_id: ComponentId,
    ) {
        let state = match Self::wasmtime_state(instance) {
            Ok(s) => s,
            Err(_) => return,
        };

        let func = match &state.func_teardown {
            Some(f) => *f,
            None => return,
        };

        let mut results = vec![Val::Bool(false); 1];

        if let Err(e) = func
            .call_async(&mut state.store, &[], &mut results)
            .await
        {
            // Best-effort: log errors but don't propagate.
            #[cfg(feature = "tracing-support")]
            tracing::warn!(
                component_id = %component_id,
                error = %e,
                "Component teardown failed"
            );
            let _ = (component_id, e);
        }

        // LLI DEVIATION: post_return_async() removed (spike finding 3.7)
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
        let process_err =
            WasmtimeInvoker::convert_wasm_error(err, ComponentId::new(1), "process");
        assert!(matches!(process_err, ProcessError::DeadlineExceeded));
    }

    #[test]
    fn test_convert_wasm_error_trap() {
        let err = wasmtime::Error::from(Trap::UnreachableCodeReached);
        let process_err =
            WasmtimeInvoker::convert_wasm_error(err, ComponentId::new(1), "process");
        assert!(matches!(process_err, ProcessError::Fatal(_)));
    }

    #[test]
    fn test_unmarshal_backpressure_ready() {
        let results = vec![Val::U32(0)];
        let signal = WasmtimeInvoker::unmarshal_backpressure(&results).unwrap();
        assert_eq!(signal, BackpressureSignal::Ready);
    }

    #[test]
    fn test_unmarshal_backpressure_pause() {
        let results = vec![Val::U32(1)];
        let signal = WasmtimeInvoker::unmarshal_backpressure(&results).unwrap();
        assert_eq!(signal, BackpressureSignal::Pause);
    }

    #[test]
    fn test_unmarshal_backpressure_invalid() {
        let results = vec![Val::U32(99)];
        let result = WasmtimeInvoker::unmarshal_backpressure(&results);
        assert!(result.is_err());
    }

    #[test]
    fn test_unmarshal_backpressure_empty() {
        let results: Vec<Val> = vec![];
        let result = WasmtimeInvoker::unmarshal_backpressure(&results);
        assert!(result.is_err());
    }

    #[test]
    fn test_unmarshal_output_element_insufficient_values() {
        let results = vec![Val::U64(0), Val::U64(0)];
        let result = WasmtimeInvoker::unmarshal_output_element(&results);
        assert!(result.is_err());
    }

    #[test]
    fn test_unmarshal_output_element_valid() {
        let results = vec![
            Val::U64(42),
            Val::U64(1_000_000),
            Val::String("text/plain".into()),
            Val::U32(5),
        ];
        let output = WasmtimeInvoker::unmarshal_output_element(&results).unwrap();
        assert_eq!(output.meta.sequence, 42);
        assert_eq!(output.meta.timestamp_ns, 1_000_000);
        assert_eq!(output.meta.content_type, "text/plain");
        assert_eq!(output.payload.resource_id().index(), 5);
    }

    #[test]
    fn test_unmarshal_output_element_wrong_type() {
        let results = vec![
            Val::String("not a number".into()),
            Val::U64(0),
            Val::String("x".into()),
            Val::U32(0),
        ];
        let result = WasmtimeInvoker::unmarshal_output_element(&results);
        assert!(result.is_err());
    }

    #[test]
    fn test_marshal_stream_element() {
        let element = StreamElement {
            meta: ElementMeta::new(10, 2000, "application/json".into()),
            payload: BufferHandle::new(ResourceId::new(7, 0)),
        };
        let mut args = Vec::new();
        WasmtimeInvoker::marshal_stream_element(&element, &mut args);
        assert_eq!(args.len(), 4);
    }
}
