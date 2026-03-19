//! Mock implementations of engine traits for testing.
//!
//! Gated behind the `mock` feature flag. Enables testing the reactor,
//! host, and pipeline crates without compiling real WebAssembly.
//!
//! # Examples
//! ```
//! use torvyn_engine::mock::{MockEngine, MockInvoker};
//!
//! let engine = MockEngine::new();
//! let invoker = MockInvoker::new();
//! ```

use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};

use torvyn_types::{
    BackpressureSignal, BufferHandle, ComponentId, ElementMeta, ProcessError, ResourceId,
};

use crate::error::EngineError;
use crate::traits::{ComponentInvoker, WasmEngine};
use crate::types::{
    CompiledComponent, CompiledComponentInner, ComponentInstance, ComponentInstanceInner,
    ImportBindings, ImportBindingsInner, MockCompiledComponent, MockInstanceState, OutputElement,
    ProcessResult, StreamElement,
};

/// Mock Wasm engine for testing.
///
/// All compile/instantiate operations succeed with predictable results.
/// Useful for testing the reactor, host, and pipeline logic without
/// requiring actual Wasm compilation.
pub struct MockEngine {
    component_counter: AtomicU64,
}

impl MockEngine {
    /// Create a new mock engine.
    pub fn new() -> Self {
        Self {
            component_counter: AtomicU64::new(0),
        }
    }

    /// Create mock import bindings.
    pub fn mock_imports() -> ImportBindings {
        ImportBindings {
            inner: ImportBindingsInner::Mock,
        }
    }
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WasmEngine for MockEngine {
    fn compile_component(&self, _bytes: &[u8]) -> Result<CompiledComponent, EngineError> {
        let id = self.component_counter.fetch_add(1, Ordering::Relaxed);
        Ok(CompiledComponent {
            inner: CompiledComponentInner::Mock(MockCompiledComponent { id }),
        })
    }

    fn serialize_component(&self, _compiled: &CompiledComponent) -> Result<Vec<u8>, EngineError> {
        Ok(vec![0xCA, 0xFE])
    }

    unsafe fn deserialize_component(
        &self,
        bytes: &[u8],
    ) -> Result<Option<CompiledComponent>, EngineError> {
        if bytes == [0xCA, 0xFE] {
            self.compile_component(bytes).map(Some)
        } else {
            Ok(None)
        }
    }

    async fn instantiate(
        &self,
        _compiled: &CompiledComponent,
        _imports: ImportBindings,
        component_id: ComponentId,
    ) -> Result<ComponentInstance, EngineError> {
        Ok(ComponentInstance {
            component_id,
            inner: ComponentInstanceInner::Mock(MockInstanceState {
                component_id,
                fuel: 1_000_000,
                memory_bytes: 0,
                call_count: 0,
                process_response: None,
                pull_response: None,
                push_response: None,
                should_trap: false,
            }),
            has_lifecycle: true,
            has_processor: true,
            has_source: true,
            has_sink: true,
        })
    }

    fn set_fuel(&self, instance: &mut ComponentInstance, fuel: u64) -> Result<(), EngineError> {
        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.fuel = fuel;
            Ok(())
        } else {
            Err(EngineError::Internal {
                reason: "MockEngine::set_fuel called with non-mock instance".into(),
            })
        }
    }

    fn fuel_remaining(&self, instance: &ComponentInstance) -> Option<u64> {
        if let ComponentInstanceInner::Mock(state) = &instance.inner {
            Some(state.fuel)
        } else {
            None
        }
    }

    fn memory_usage(&self, instance: &ComponentInstance) -> usize {
        if let ComponentInstanceInner::Mock(state) = &instance.inner {
            state.memory_bytes
        } else {
            0
        }
    }
}

/// Mock component invoker for testing.
///
/// Returns configurable responses for each invocation method.
/// Default: passthrough (process returns the input, pull returns
/// a test element, push returns Ready).
pub struct MockInvoker {
    invocation_count: AtomicU64,
}

impl MockInvoker {
    /// Create a new mock invoker.
    pub fn new() -> Self {
        Self {
            invocation_count: AtomicU64::new(0),
        }
    }

    /// Returns the total number of invocations across all methods.
    pub fn invocation_count(&self) -> u64 {
        self.invocation_count.load(Ordering::Relaxed)
    }
}

impl Default for MockInvoker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComponentInvoker for MockInvoker {
    async fn invoke_pull(
        &self,
        instance: &mut ComponentInstance,
        _component_id: ComponentId,
    ) -> Result<Option<OutputElement>, ProcessError> {
        self.invocation_count.fetch_add(1, Ordering::Relaxed);

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.call_count += 1;

            if state.should_trap {
                return Err(ProcessError::Fatal("mock trap".into()));
            }

            if let Some(ref response) = state.pull_response {
                return Ok(response.clone());
            }

            // Default: produce a test element.
            let seq = state.call_count - 1;
            Ok(Some(OutputElement {
                meta: ElementMeta::new(seq, seq * 1000, "application/octet-stream".into()),
                payload: BufferHandle::new(ResourceId::new(seq as u32, 0)),
            }))
        } else {
            Err(ProcessError::Internal(
                "mock invoker with non-mock instance".into(),
            ))
        }
    }

    async fn invoke_process(
        &self,
        instance: &mut ComponentInstance,
        _component_id: ComponentId,
        element: StreamElement,
    ) -> Result<ProcessResult, ProcessError> {
        self.invocation_count.fetch_add(1, Ordering::Relaxed);

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.call_count += 1;

            if state.should_trap {
                return Err(ProcessError::Fatal("mock trap".into()));
            }

            if let Some(ref response) = state.process_response {
                return Ok(response.clone());
            }

            // Default: passthrough transform.
            Ok(ProcessResult::Output(OutputElement {
                meta: element.meta,
                payload: element.payload,
            }))
        } else {
            Err(ProcessError::Internal(
                "mock invoker with non-mock instance".into(),
            ))
        }
    }

    async fn invoke_push(
        &self,
        instance: &mut ComponentInstance,
        _component_id: ComponentId,
        _element: StreamElement,
    ) -> Result<BackpressureSignal, ProcessError> {
        self.invocation_count.fetch_add(1, Ordering::Relaxed);

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.call_count += 1;

            if state.should_trap {
                return Err(ProcessError::Fatal("mock trap".into()));
            }

            if let Some(signal) = state.push_response {
                return Ok(signal);
            }

            // Default: always ready.
            Ok(BackpressureSignal::Ready)
        } else {
            Err(ProcessError::Internal(
                "mock invoker with non-mock instance".into(),
            ))
        }
    }

    async fn invoke_init(
        &self,
        instance: &mut ComponentInstance,
        _component_id: ComponentId,
        _config: &str,
    ) -> Result<(), ProcessError> {
        self.invocation_count.fetch_add(1, Ordering::Relaxed);

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.call_count += 1;
            if state.should_trap {
                return Err(ProcessError::Fatal("mock init trap".into()));
            }
            Ok(())
        } else {
            Err(ProcessError::Internal(
                "mock invoker with non-mock instance".into(),
            ))
        }
    }

    async fn invoke_teardown(&self, instance: &mut ComponentInstance, _component_id: ComponentId) {
        self.invocation_count.fetch_add(1, Ordering::Relaxed);

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.call_count += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_engine_compile() {
        let engine = MockEngine::new();
        let compiled = engine.compile_component(b"test");
        assert!(compiled.is_ok());
    }

    #[tokio::test]
    async fn test_mock_engine_instantiate() {
        let engine = MockEngine::new();
        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await;
        assert!(instance.is_ok());
        let inst = instance.unwrap();
        assert_eq!(inst.component_id(), ComponentId::new(1));
        assert!(inst.has_processor());
        assert!(inst.has_source());
        assert!(inst.has_sink());
        assert!(inst.has_lifecycle());
    }

    #[tokio::test]
    async fn test_mock_engine_fuel() {
        let engine = MockEngine::new();
        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        assert_eq!(engine.fuel_remaining(&instance), Some(1_000_000));

        engine.set_fuel(&mut instance, 500).unwrap();
        assert_eq!(engine.fuel_remaining(&instance), Some(500));
    }

    #[tokio::test]
    async fn test_mock_engine_memory_usage() {
        let engine = MockEngine::new();
        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        assert_eq!(engine.memory_usage(&instance), 0);
    }

    #[tokio::test]
    async fn test_mock_invoker_process_passthrough() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        let element = StreamElement {
            meta: ElementMeta::new(42, 1000, "text/plain".into()),
            payload: BufferHandle::new(ResourceId::new(5, 0)),
        };

        let result = invoker
            .invoke_process(&mut instance, ComponentId::new(1), element)
            .await
            .unwrap();

        assert!(result.has_output());
        assert_eq!(result.output_count(), 1);
        assert_eq!(invoker.invocation_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_invoker_pull_produces_elements() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        let result = invoker
            .invoke_pull(&mut instance, ComponentId::new(1))
            .await
            .unwrap();

        assert!(result.is_some());
        let output = result.unwrap();
        assert_eq!(output.meta.sequence, 0);
    }

    #[tokio::test]
    async fn test_mock_invoker_push_returns_ready() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        let element = StreamElement {
            meta: ElementMeta::new(0, 100, "test".into()),
            payload: BufferHandle::new(ResourceId::new(0, 0)),
        };

        let signal = invoker
            .invoke_push(&mut instance, ComponentId::new(1), element)
            .await
            .unwrap();

        assert_eq!(signal, BackpressureSignal::Ready);
    }

    #[tokio::test]
    async fn test_mock_invoker_trap() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        // Configure the mock to trap.
        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.should_trap = true;
        }

        let element = StreamElement {
            meta: ElementMeta::new(0, 100, "test".into()),
            payload: BufferHandle::new(ResourceId::new(0, 0)),
        };

        let result = invoker
            .invoke_process(&mut instance, ComponentId::new(1), element)
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProcessError::Fatal(_)));
    }

    #[tokio::test]
    async fn test_mock_invoker_init_teardown() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        let result = invoker
            .invoke_init(&mut instance, ComponentId::new(1), r#"{"key":"value"}"#)
            .await;
        assert!(result.is_ok());

        invoker
            .invoke_teardown(&mut instance, ComponentId::new(1))
            .await;

        assert_eq!(invoker.invocation_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_invoker_init_trap() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.should_trap = true;
        }

        let result = invoker
            .invoke_init(&mut instance, ComponentId::new(1), "{}")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_invoker_pull_trap() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.should_trap = true;
        }

        let result = invoker
            .invoke_pull(&mut instance, ComponentId::new(1))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_invoker_push_trap() {
        let engine = MockEngine::new();
        let invoker = MockInvoker::new();

        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let mut instance = engine
            .instantiate(&compiled, imports, ComponentId::new(1))
            .await
            .unwrap();

        if let ComponentInstanceInner::Mock(state) = &mut instance.inner {
            state.should_trap = true;
        }

        let element = StreamElement {
            meta: ElementMeta::new(0, 100, "test".into()),
            payload: BufferHandle::new(ResourceId::new(0, 0)),
        };

        let result = invoker
            .invoke_push(&mut instance, ComponentId::new(1), element)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_engine_serialize_deserialize() {
        let engine = MockEngine::new();
        let compiled = engine.compile_component(b"test").unwrap();

        let bytes = engine.serialize_component(&compiled).unwrap();
        assert_eq!(bytes, vec![0xCA, 0xFE]);

        // SAFETY: test bytes from our own serialize.
        let deserialized = unsafe { engine.deserialize_component(&bytes) }.unwrap();
        assert!(deserialized.is_some());

        // Invalid bytes return None.
        let invalid = unsafe { engine.deserialize_component(b"invalid") }.unwrap();
        assert!(invalid.is_none());
    }

    #[test]
    fn test_mock_engine_default() {
        let _engine = MockEngine::default();
    }

    #[test]
    fn test_mock_invoker_default() {
        let _invoker = MockInvoker::default();
    }
}
