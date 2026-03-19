//! Shared test harness for workspace-level integration tests.
//!
//! Provides a configurable [`TestInvoker`] and topology-building helpers
//! that mirror the patterns in `torvyn-reactor`'s own integration tests,
//! extended with richer behavior modes for cross-crate testing.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

pub use torvyn_types::{
    BackpressurePolicy, BackpressureSignal, BufferHandle, ComponentId, ComponentRole, ElementMeta,
    FlowId, FlowState, NoopEventSink, ProcessError, ResourceId, StreamId,
};

pub use torvyn_engine::mock::MockEngine;
pub use torvyn_engine::{
    ComponentInstance, ComponentInvoker, OutputElement, ProcessResult, StreamElement, WasmEngine,
};

pub use torvyn_reactor::cancellation::CancellationReason;
pub use torvyn_reactor::config::{FlowConfig, StreamConfig};
pub use torvyn_reactor::coordinator::ReactorCoordinator;
pub use torvyn_reactor::events::{ReactorCommand, ReactorEvent};
pub use torvyn_reactor::flow_driver::FlowDriver;
pub use torvyn_reactor::handle::ReactorHandle;
pub use torvyn_reactor::stream::{StreamElementRef, StreamState};
pub use torvyn_reactor::topology::{FlowTopology, StageDefinition, StreamConnection};
pub use torvyn_reactor::{ErrorPolicy, FlowCancellation, FlowCompletionStats};

// ===========================================================================
// Pull behavior configuration
// ===========================================================================

/// Configures how the test invoker handles `invoke_pull` calls.
pub enum PullBehavior {
    /// Produce exactly `n` elements, then return `None`.
    ProduceN(u64),
    /// Produce elements indefinitely (until cancelled).
    Infinite,
}

// ===========================================================================
// Process behavior configuration
// ===========================================================================

/// Configures how the test invoker handles `invoke_process` calls.
pub enum ProcessBehavior {
    /// Pass elements through unchanged (identity).
    Passthrough,
    /// Fail with a fatal error after `n` successful elements.
    FailAfterN(u64),
}

// ===========================================================================
// Push behavior configuration
// ===========================================================================

/// Configures how the test invoker handles `invoke_push` calls.
pub enum PushBehavior {
    /// Accept all elements, returning `Ready`.
    AcceptAll,
    /// Accept all elements and record sequence numbers in the collector.
    CollectSequences(Arc<Mutex<Vec<u64>>>),
    /// Accept elements with an artificial delay (spin loop) per element.
    SlowAccept {
        /// Number of spin-loop iterations per element.
        spin_iterations: u64,
    },
}

// ===========================================================================
// TestInvoker
// ===========================================================================

/// A configurable mock [`ComponentInvoker`] for integration tests.
///
/// Supports various pull, process, and push behaviors to test different
/// pipeline scenarios (finite sources, failing processors, slow sinks, etc.).
///
/// # LLI DEVIATION: The design doc assumes `MockInvoker` with builder-pattern
/// `MockComponentBehavior` enums. The actual `MockInvoker` in `torvyn-engine`
/// is a simple passthrough. Workspace tests use this richer `TestInvoker`
/// instead, following the pattern from `torvyn-reactor/tests/integration_tests.rs`.
pub struct TestInvoker {
    pull_count: AtomicU64,
    pull_behavior: PullBehavior,
    process_count: AtomicU64,
    process_behavior: ProcessBehavior,
    push_behavior: PushBehavior,
    should_error: AtomicBool,
}

impl TestInvoker {
    /// Create a new `TestInvoker` that produces `n` elements and accepts all pushes.
    pub fn new(max_pulls: u64) -> Self {
        Self {
            pull_count: AtomicU64::new(0),
            pull_behavior: PullBehavior::ProduceN(max_pulls),
            process_count: AtomicU64::new(0),
            process_behavior: ProcessBehavior::Passthrough,
            push_behavior: PushBehavior::AcceptAll,
            should_error: AtomicBool::new(false),
        }
    }

    /// Create a `TestInvoker` that immediately errors on any invocation.
    pub fn erroring() -> Self {
        let invoker = Self::new(u64::MAX);
        invoker.should_error.store(true, Ordering::Release);
        invoker
    }

    /// Create a `TestInvoker` that produces elements indefinitely.
    pub fn infinite() -> Self {
        Self {
            pull_count: AtomicU64::new(0),
            pull_behavior: PullBehavior::Infinite,
            process_count: AtomicU64::new(0),
            process_behavior: ProcessBehavior::Passthrough,
            push_behavior: PushBehavior::AcceptAll,
            should_error: AtomicBool::new(false),
        }
    }

    /// Set the pull behavior.
    pub fn with_pull(mut self, behavior: PullBehavior) -> Self {
        self.pull_behavior = behavior;
        self
    }

    /// Set the process behavior.
    pub fn with_process(mut self, behavior: ProcessBehavior) -> Self {
        self.process_behavior = behavior;
        self
    }

    /// Set the push behavior.
    pub fn with_push(mut self, behavior: PushBehavior) -> Self {
        self.push_behavior = behavior;
        self
    }
}

#[async_trait]
impl ComponentInvoker for TestInvoker {
    async fn invoke_pull(
        &self,
        _instance: &mut ComponentInstance,
        _component_id: ComponentId,
    ) -> Result<Option<OutputElement>, ProcessError> {
        if self.should_error.load(Ordering::Acquire) {
            return Err(ProcessError::Fatal("test error".into()));
        }
        let count = self.pull_count.fetch_add(1, Ordering::Relaxed);
        match &self.pull_behavior {
            PullBehavior::ProduceN(max) => {
                if count >= *max {
                    Ok(None)
                } else {
                    Ok(Some(OutputElement {
                        meta: ElementMeta::new(count, count * 1000, String::new()),
                        payload: BufferHandle::new(ResourceId::new(count as u32, 0)),
                    }))
                }
            }
            PullBehavior::Infinite => Ok(Some(OutputElement {
                meta: ElementMeta::new(count, count * 1000, String::new()),
                payload: BufferHandle::new(ResourceId::new(count as u32, 0)),
            })),
        }
    }

    async fn invoke_process(
        &self,
        _instance: &mut ComponentInstance,
        _component_id: ComponentId,
        element: StreamElement,
    ) -> Result<ProcessResult, ProcessError> {
        if self.should_error.load(Ordering::Acquire) {
            return Err(ProcessError::Fatal("test error".into()));
        }

        let count = self.process_count.fetch_add(1, Ordering::Relaxed);
        match &self.process_behavior {
            ProcessBehavior::Passthrough => Ok(ProcessResult::Output(OutputElement {
                meta: element.meta,
                payload: element.payload,
            })),
            ProcessBehavior::FailAfterN(limit) => {
                if count >= *limit {
                    Err(ProcessError::Fatal(format!(
                        "intentional failure after {limit} elements"
                    )))
                } else {
                    Ok(ProcessResult::Output(OutputElement {
                        meta: element.meta,
                        payload: element.payload,
                    }))
                }
            }
        }
    }

    async fn invoke_push(
        &self,
        _instance: &mut ComponentInstance,
        _component_id: ComponentId,
        element: StreamElement,
    ) -> Result<BackpressureSignal, ProcessError> {
        if self.should_error.load(Ordering::Acquire) {
            return Err(ProcessError::Fatal("test error".into()));
        }

        match &self.push_behavior {
            PushBehavior::AcceptAll => Ok(BackpressureSignal::Ready),
            PushBehavior::CollectSequences(collector) => {
                let seq = element.meta.sequence;
                collector.lock().unwrap().push(seq);
                Ok(BackpressureSignal::Ready)
            }
            PushBehavior::SlowAccept { spin_iterations } => {
                // Spin loop to simulate slow processing.
                let mut x: u64 = 0;
                for i in 0..*spin_iterations {
                    x = x.wrapping_add(i);
                }
                let _ = std::hint::black_box(x);
                Ok(BackpressureSignal::Ready)
            }
        }
    }

    async fn invoke_init(
        &self,
        _instance: &mut ComponentInstance,
        _component_id: ComponentId,
        _config: &str,
    ) -> Result<(), ProcessError> {
        Ok(())
    }

    async fn invoke_teardown(&self, _instance: &mut ComponentInstance, _component_id: ComponentId) {
    }
}

// ===========================================================================
// Topology helpers
// ===========================================================================

/// Create a [`StageDefinition`] for a source component.
pub fn source(id: u64) -> StageDefinition {
    StageDefinition {
        component_id: ComponentId::new(id),
        role: ComponentRole::Source,
        fuel_budget: None,
        config: String::new(),
    }
}

/// Create a [`StageDefinition`] for a processor component.
pub fn processor(id: u64) -> StageDefinition {
    StageDefinition {
        component_id: ComponentId::new(id),
        role: ComponentRole::Processor,
        fuel_budget: None,
        config: String::new(),
    }
}

/// Create a [`StageDefinition`] for a sink component.
pub fn sink(id: u64) -> StageDefinition {
    StageDefinition {
        component_id: ComponentId::new(id),
        role: ComponentRole::Sink,
        fuel_budget: None,
        config: String::new(),
    }
}

/// Create a [`StreamConnection`] between two stages (by index).
pub fn conn(from: usize, to: usize) -> StreamConnection {
    StreamConnection {
        from_stage: from,
        to_stage: to,
        config: StreamConfig::default(),
    }
}

/// Create a [`StreamConnection`] with custom configuration.
pub fn conn_with_config(from: usize, to: usize, config: StreamConfig) -> StreamConnection {
    StreamConnection {
        from_stage: from,
        to_stage: to,
        config,
    }
}

// ===========================================================================
// Driver helpers
// ===========================================================================

/// Create [`StreamState`] instances for each connection in a topology.
///
/// Uses the `FlowConfig`'s defaults with per-connection overrides from
/// `StreamConfig`. This matches the actual flow driver's behavior.
pub fn make_streams(
    topology: &FlowTopology,
    flow_id: FlowId,
    config: &FlowConfig,
) -> Vec<StreamState> {
    topology
        .connections
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let capacity = c.config.capacity.unwrap_or(config.default_queue_capacity);
            let bp_policy = c
                .config
                .backpressure_policy
                .unwrap_or(config.default_backpressure_policy);
            let lw_ratio = c
                .config
                .low_watermark_ratio
                .unwrap_or(config.default_low_watermark_ratio);
            StreamState::new(
                StreamId::new(idx as u64),
                flow_id,
                topology.stages[c.from_stage].component_id,
                topology.stages[c.to_stage].component_id,
                capacity,
                bp_policy,
                lw_ratio,
            )
        })
        .collect()
}

/// Create placeholder [`ComponentInstance`]s for testing (via `MockEngine`).
pub async fn make_instances(topology: &FlowTopology) -> Vec<ComponentInstance> {
    let engine = MockEngine::new();
    let mut instances = Vec::new();
    for stage in &topology.stages {
        let compiled = engine.compile_component(b"test").unwrap();
        let imports = MockEngine::mock_imports();
        let instance = engine
            .instantiate(&compiled, imports, stage.component_id)
            .await
            .unwrap();
        instances.push(instance);
    }
    instances
}

/// Build a [`FlowDriver`] with the given invoker and topology.
///
/// Returns the driver, a cancellation handle, and the event receiver.
pub async fn build_driver(
    invoker: TestInvoker,
    flow_id: FlowId,
    topology: FlowTopology,
    config: FlowConfig,
) -> (
    FlowDriver<TestInvoker, NoopEventSink>,
    FlowCancellation,
    mpsc::Receiver<ReactorEvent>,
) {
    let streams = make_streams(&topology, flow_id, &config);
    let instances = make_instances(&topology).await;
    let cancellation = FlowCancellation::new();
    let cancel_clone = cancellation.clone();
    let (event_tx, event_rx) = mpsc::channel(1024);

    let driver = FlowDriver::new(
        flow_id,
        config,
        instances,
        streams,
        invoker,
        NoopEventSink,
        cancellation,
        event_tx,
    );

    (driver, cancel_clone, event_rx)
}

// ===========================================================================
// Coordinator helpers
// ===========================================================================

/// Spawn a [`ReactorCoordinator`] and return its [`ReactorHandle`].
///
/// The coordinator runs in a background Tokio task. The returned handle
/// can create flows, query state, and shut down.
pub fn spawn_coordinator(invoker: TestInvoker) -> (ReactorHandle, tokio::task::JoinHandle<()>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(256);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(256);

    let coordinator =
        ReactorCoordinator::new(cmd_rx, event_tx, Arc::new(invoker), Arc::new(NoopEventSink));

    let join = tokio::spawn(coordinator.run());
    let handle = ReactorHandle::new(cmd_tx);

    (handle, join)
}
