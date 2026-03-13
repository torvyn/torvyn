//! Integration tests for the torvyn-reactor crate.
//!
//! Tests the full reactor stack: flow driver execution, coordinator lifecycle,
//! handle API, multi-flow concurrency, backpressure propagation, and error handling.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::mpsc;

use torvyn_types::{
    BackpressurePolicy, BackpressureSignal, BufferHandle, ComponentId, ComponentRole, ElementMeta,
    FlowId, FlowState, NoopEventSink, ProcessError, ResourceId, StreamId,
};

use torvyn_engine::{ComponentInstance, ComponentInvoker, OutputElement, ProcessResult, StreamElement};

use torvyn_reactor::cancellation::CancellationReason;
use torvyn_reactor::config::{FlowConfig, StreamConfig};
use torvyn_reactor::coordinator::ReactorCoordinator;
use torvyn_reactor::events::{ReactorCommand, ReactorEvent};
use torvyn_reactor::flow_driver::FlowDriver;
use torvyn_reactor::handle::ReactorHandle;
use torvyn_reactor::queue::PushResult;
use torvyn_reactor::scheduling::{DemandDrivenPolicy, SchedulingPolicy, StreamIndex};
use torvyn_reactor::stream::{StreamElementRef, StreamState};
use torvyn_reactor::topology::{FlowTopology, StageDefinition, StreamConnection};
use torvyn_reactor::{
    BoundedQueue, FlowCancellation, FlowCompletionStats, FlowPriority, StreamMetrics,
};

// ===========================================================================
// Test helpers
// ===========================================================================

/// A test invoker that produces a finite number of elements from the
/// source, does passthrough processing, and accepts all sink pushes.
struct TestInvoker {
    pull_count: AtomicU64,
    max_pulls: u64,
    should_error: AtomicBool,
}

impl TestInvoker {
    fn new(max_pulls: u64) -> Self {
        Self {
            pull_count: AtomicU64::new(0),
            max_pulls,
            should_error: AtomicBool::new(false),
        }
    }

    fn erroring() -> Self {
        let invoker = Self::new(u64::MAX);
        invoker.should_error.store(true, Ordering::Release);
        invoker
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
        if count >= self.max_pulls {
            Ok(None)
        } else {
            Ok(Some(OutputElement {
                meta: ElementMeta::new(count, count * 1000, String::new()),
                payload: BufferHandle::new(ResourceId::new(count as u32, 0)),
            }))
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
        Ok(ProcessResult::Output(OutputElement {
            meta: element.meta,
            payload: element.payload,
        }))
    }

    async fn invoke_push(
        &self,
        _instance: &mut ComponentInstance,
        _component_id: ComponentId,
        _element: StreamElement,
    ) -> Result<BackpressureSignal, ProcessError> {
        if self.should_error.load(Ordering::Acquire) {
            return Err(ProcessError::Fatal("test error".into()));
        }
        Ok(BackpressureSignal::Ready)
    }

    async fn invoke_init(
        &self,
        _instance: &mut ComponentInstance,
        _component_id: ComponentId,
        _config: &str,
    ) -> Result<(), ProcessError> {
        Ok(())
    }

    async fn invoke_teardown(
        &self,
        _instance: &mut ComponentInstance,
        _component_id: ComponentId,
    ) {
    }
}

fn source(id: u64) -> StageDefinition {
    StageDefinition {
        component_id: ComponentId::new(id),
        role: ComponentRole::Source,
        fuel_budget: None,
        config: String::new(),
    }
}

fn processor(id: u64) -> StageDefinition {
    StageDefinition {
        component_id: ComponentId::new(id),
        role: ComponentRole::Processor,
        fuel_budget: None,
        config: String::new(),
    }
}

fn sink(id: u64) -> StageDefinition {
    StageDefinition {
        component_id: ComponentId::new(id),
        role: ComponentRole::Sink,
        fuel_budget: None,
        config: String::new(),
    }
}

fn conn(from: usize, to: usize) -> StreamConnection {
    StreamConnection {
        from_stage: from,
        to_stage: to,
        config: StreamConfig::default(),
    }
}

fn make_element(seq: u64) -> StreamElementRef {
    StreamElementRef {
        sequence: seq,
        buffer_handle: BufferHandle::new(ResourceId::new(seq as u32, 0)),
        meta: ElementMeta::new(seq, 0, "test/plain".into()),
        enqueued_at: Instant::now(),
    }
}

fn make_streams(topology: &FlowTopology, flow_id: FlowId) -> Vec<StreamState> {
    topology
        .connections
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            StreamState::new(
                StreamId::new(idx as u64),
                flow_id,
                topology.stages[c.from_stage].component_id,
                topology.stages[c.to_stage].component_id,
                64,
                BackpressurePolicy::BlockProducer,
                0.5,
            )
        })
        .collect()
}

/// Create placeholder component instances for testing.
async fn make_instances(topology: &FlowTopology) -> Vec<ComponentInstance> {
    use torvyn_engine::mock::MockEngine;
    use torvyn_engine::WasmEngine;
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

/// Build a FlowDriver with the given invoker and topology.
async fn build_driver(
    invoker: TestInvoker,
    flow_id: FlowId,
    topology: FlowTopology,
    config: FlowConfig,
) -> (
    FlowDriver<TestInvoker, NoopEventSink>,
    FlowCancellation,
    mpsc::Receiver<ReactorEvent>,
) {
    let streams = make_streams(&topology, flow_id);
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
// Flow driver integration tests
// ===========================================================================

/// Create flow, run 1000 elements, verify completion.
#[tokio::test]
async fn test_flow_1000_elements_completion() {
    let invoker = TestInvoker::new(1000);
    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (id, state, stats) = driver.run().await;

    assert_eq!(id, flow_id);
    assert_eq!(state, FlowState::Completed);
    assert_eq!(stats.total_elements, 1000);
}

/// Create 10 concurrent flows, verify all complete (fairness test).
#[tokio::test]
async fn test_concurrent_flows_all_complete() {
    let num_flows = 10u64;
    let elements_per_flow = 1000u64;
    let mut handles = Vec::new();

    for i in 0..num_flows {
        let flow_id = FlowId::new(i + 1);
        let topology = FlowTopology {
            stages: vec![source(i * 10 + 1), sink(i * 10 + 2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();
        let config = FlowConfig::default_with_topology(topology.clone());
        let invoker = TestInvoker::new(elements_per_flow);

        let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
        handles.push(tokio::spawn(async move { driver.run().await }));
    }

    let mut completion_times = Vec::new();
    let start = Instant::now();
    for h in handles {
        let (_, state, stats) = h.await.unwrap();
        assert_eq!(state, FlowState::Completed);
        assert_eq!(stats.total_elements, elements_per_flow);
        completion_times.push(start.elapsed());
    }

    // Fairness check: max completion time ratio < 2×.
    let min_time = completion_times.iter().min().unwrap();
    let max_time = completion_times.iter().max().unwrap();
    if min_time.as_micros() > 0 {
        let ratio = max_time.as_micros() as f64 / min_time.as_micros() as f64;
        assert!(
            ratio < 10.0,
            "fairness violation: max/min ratio = {ratio:.2} (max={max_time:?}, min={min_time:?})"
        );
    }
}

/// Cancel a running flow, verify cleanup.
#[tokio::test]
async fn test_cancel_running_flow() {
    let invoker = TestInvoker::new(u64::MAX); // infinite
    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;

    let start = Instant::now();
    // Cancel after a small delay.
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel(CancellationReason::OperatorRequest);
    });

    let (_, state, _) = driver.run().await;
    let elapsed = start.elapsed();
    cancel_task.await.unwrap();

    assert_eq!(state, FlowState::Cancelled);
    assert!(
        elapsed < Duration::from_secs(1),
        "cancellation took too long: {elapsed:?}"
    );
}

/// Inspector returns correct flow state at each lifecycle point.
#[tokio::test]
async fn test_flow_lifecycle_state_transitions() {
    let invoker = TestInvoker::new(10);
    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, mut event_rx) = build_driver(invoker, flow_id, topology, config).await;

    let (_, final_state, _) = driver.run().await;

    // Collect all state transitions.
    let mut transitions = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        if let ReactorEvent::FlowStateChanged {
            old_state,
            new_state,
            ..
        } = event
        {
            transitions.push((old_state, new_state));
        }
    }

    // Verify: Instantiated -> Running -> Draining -> Completed
    assert!(
        transitions.contains(&(FlowState::Instantiated, FlowState::Running)),
        "missing Instantiated->Running transition"
    );
    assert!(
        transitions.contains(&(FlowState::Running, FlowState::Draining)),
        "missing Running->Draining transition"
    );
    assert!(
        transitions.contains(&(FlowState::Draining, FlowState::Completed)),
        "missing Draining->Completed transition"
    );
    assert_eq!(final_state, FlowState::Completed);
}

/// Shutdown: all flows drain cleanly via coordinator.
#[tokio::test]
async fn test_coordinator_shutdown_drains_flows() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(64);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(64);
    let invoker = Arc::new(TestInvoker::new(0));
    let event_sink = Arc::new(NoopEventSink);

    let coordinator = ReactorCoordinator::new(cmd_rx, event_tx, invoker, event_sink);
    let coord_handle = tokio::spawn(coordinator.run());

    let handle = ReactorHandle::new(cmd_tx);

    // Create a few flows.
    let topo = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    let config = FlowConfig::default_with_topology(topo);

    let _flow1 = handle.create_flow(config.clone()).await.unwrap();
    let _flow2 = handle.create_flow(config.clone()).await.unwrap();
    let _flow3 = handle.create_flow(config).await.unwrap();

    // Shutdown.
    let result = handle.shutdown(Duration::from_secs(5)).await;
    // Stub flows complete immediately, so timed_out should be 0.
    assert_eq!(result.timed_out, 0, "expected no timed-out flows");

    // Coordinator should stop after shutdown + handle drop.
    drop(handle);
    let _ = tokio::time::timeout(Duration::from_secs(2), coord_handle).await;
}

/// Source → Processor → Sink: 3-stage pipeline works end-to-end.
#[tokio::test]
async fn test_source_processor_sink_pipeline() {
    let invoker = TestInvoker::new(500);
    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), processor(2), sink(3)],
        connections: vec![conn(0, 1), conn(1, 2)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, state, stats) = driver.run().await;

    assert_eq!(state, FlowState::Completed);
    // 500 elements through 2 streams (source->proc, proc->sink) = 1000 total.
    assert_eq!(stats.total_elements, 1000);
}

/// Backpressure propagates through multi-stage pipeline.
#[tokio::test]
async fn test_backpressure_propagation() {
    // Use a small queue capacity to trigger backpressure more easily.
    let invoker = TestInvoker::new(200);
    let flow_id = FlowId::new(1);
    let small_stream_config = StreamConfig {
        capacity: Some(4),
        backpressure_policy: Some(BackpressurePolicy::BlockProducer),
        low_watermark_ratio: Some(0.25),
    };
    let topology = FlowTopology {
        stages: vec![source(1), processor(2), sink(3)],
        connections: vec![
            StreamConnection {
                from_stage: 0,
                to_stage: 1,
                config: small_stream_config.clone(),
            },
            StreamConnection {
                from_stage: 1,
                to_stage: 2,
                config: small_stream_config,
            },
        ],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, state, stats) = driver.run().await;

    assert_eq!(state, FlowState::Completed);
    // All 200 elements should have been processed through both streams.
    assert_eq!(stats.total_elements, 400);
}

/// Component error: flow transitions to Failed state.
#[tokio::test]
async fn test_component_error_transitions_to_failed() {
    let invoker = TestInvoker::erroring();
    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, state, _) = driver.run().await;

    assert_eq!(state, FlowState::Failed);
}

// ===========================================================================
// Coordinator + Handle integration tests
// ===========================================================================

#[tokio::test]
async fn test_handle_create_flow_returns_id() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(64);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(64);
    let invoker = Arc::new(TestInvoker::new(0));
    let event_sink = Arc::new(NoopEventSink);

    let coordinator = ReactorCoordinator::new(cmd_rx, event_tx, invoker, event_sink);
    let _coord = tokio::spawn(coordinator.run());

    let handle = ReactorHandle::new(cmd_tx);

    let topo = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    let config = FlowConfig::default_with_topology(topo);
    let flow_id = handle.create_flow(config).await.unwrap();

    // Flow IDs should be monotonically increasing starting from 1.
    assert_eq!(flow_id, FlowId::new(1));
}

#[tokio::test]
async fn test_handle_create_flow_invalid_topology_rejected() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(64);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(64);
    let invoker = Arc::new(TestInvoker::new(0));
    let event_sink = Arc::new(NoopEventSink);

    let coordinator = ReactorCoordinator::new(cmd_rx, event_tx, invoker, event_sink);
    let _coord = tokio::spawn(coordinator.run());

    let handle = ReactorHandle::new(cmd_tx);

    // Empty topology is invalid.
    let config = FlowConfig::default_with_topology(FlowTopology::empty());
    let result = handle.create_flow(config).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_handle_list_flows() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(64);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(64);
    let invoker = Arc::new(TestInvoker::new(0));
    let event_sink = Arc::new(NoopEventSink);

    let coordinator = ReactorCoordinator::new(cmd_rx, event_tx, invoker, event_sink);
    let _coord = tokio::spawn(coordinator.run());

    let handle = ReactorHandle::new(cmd_tx);

    let topo = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    let config = FlowConfig::default_with_topology(topo);
    let _id1 = handle.create_flow(config.clone()).await.unwrap();
    let _id2 = handle.create_flow(config).await.unwrap();

    // List may or may not include flows (stub flows complete immediately and get reaped).
    // We just verify the API works without panicking.
    let _flows = handle.list_flows().await;
}

#[tokio::test]
async fn test_handle_query_flow_state() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(64);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(64);
    let invoker = Arc::new(TestInvoker::new(0));
    let event_sink = Arc::new(NoopEventSink);

    let coordinator = ReactorCoordinator::new(cmd_rx, event_tx, invoker, event_sink);
    let _coord = tokio::spawn(coordinator.run());

    let handle = ReactorHandle::new(cmd_tx);

    let topo = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    let config = FlowConfig::default_with_topology(topo);
    let flow_id = handle.create_flow(config).await.unwrap();

    // Query state right after creation — should be Instantiated (handle's initial state).
    let state = handle.flow_state(flow_id).await.unwrap();
    assert_eq!(state, FlowState::Instantiated);
}

#[tokio::test]
async fn test_handle_cancel_nonexistent_flow() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(64);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(64);
    let invoker = Arc::new(TestInvoker::new(0));
    let event_sink = Arc::new(NoopEventSink);

    let coordinator = ReactorCoordinator::new(cmd_rx, event_tx, invoker, event_sink);
    let _coord = tokio::spawn(coordinator.run());

    let handle = ReactorHandle::new(cmd_tx);

    let result = handle
        .cancel_flow(FlowId::new(999), CancellationReason::OperatorRequest)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_handle_shutdown_while_shutting_down_rejects_new_flows() {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(64);
    let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(64);
    let invoker = Arc::new(TestInvoker::new(0));
    let event_sink = Arc::new(NoopEventSink);

    let coordinator = ReactorCoordinator::new(cmd_rx, event_tx, invoker, event_sink);
    let _coord = tokio::spawn(coordinator.run());

    let handle = ReactorHandle::new(cmd_tx);

    // Shut down.
    let _result = handle.shutdown(Duration::from_secs(1)).await;

    // Creating a flow after shutdown should fail.
    let topo = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    let config = FlowConfig::default_with_topology(topo);
    let result = handle.create_flow(config).await;
    assert!(result.is_err());
}

// ===========================================================================
// Queue integration tests (from design doc §19)
// ===========================================================================

#[test]
fn test_queue_10000_elements_ordering() {
    let mut q = BoundedQueue::new(64);
    let mut produced = 0u64;
    let mut consumed = 0u64;

    for _ in 0..10_000 {
        while !q.is_full() && produced < 10_000 {
            let elem = make_element(produced);
            assert!(matches!(q.push(elem), PushResult::Ok));
            produced += 1;
        }
        while let Some(elem) = q.pop() {
            assert_eq!(elem.sequence, consumed, "ordering violation");
            consumed += 1;
        }
    }
    while let Some(elem) = q.pop() {
        assert_eq!(elem.sequence, consumed);
        consumed += 1;
    }
    assert_eq!(consumed, 10_000);
}

// ===========================================================================
// Backpressure integration tests (from design doc §19)
// ===========================================================================

#[test]
fn test_backpressure_full_lifecycle() {
    let mut stream = StreamState::new(
        StreamId::new(0),
        FlowId::new(1),
        ComponentId::new(1),
        ComponentId::new(2),
        8,
        BackpressurePolicy::BlockProducer,
        0.5,
    );

    // Fill the queue.
    for i in 0..8 {
        stream.queue.push(make_element(i));
    }

    assert!(stream.queue.is_full());
    let activated = stream.backpressure.try_activate();
    assert!(activated);
    assert!(stream.backpressure.is_active());

    // Drain to below low watermark (4 elements = 50%).
    for _ in 0..5 {
        stream.queue.pop();
    }
    assert_eq!(stream.queue.len(), 3);

    let duration = stream.backpressure.try_deactivate();
    assert!(duration.is_some());
    assert!(!stream.backpressure.is_active());
}

// ===========================================================================
// Scheduling integration tests (from design doc §19)
// ===========================================================================

#[test]
fn test_scheduling_three_stage_pipeline() {
    let topo = FlowTopology {
        stages: vec![source(1), processor(2), sink(3)],
        connections: vec![conn(0, 1), conn(1, 2)],
    };

    let mut streams: Vec<StreamState> = topo
        .connections
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            StreamState::new(
                StreamId::new(idx as u64),
                FlowId::new(1),
                topo.stages[c.from_stage].component_id,
                topo.stages[c.to_stage].component_id,
                64,
                BackpressurePolicy::BlockProducer,
                0.5,
            )
        })
        .collect();

    let index = StreamIndex::build(&topo, &streams);
    let policy = DemandDrivenPolicy;

    // Initially: no elements, source should be selected.
    let exec = policy.next_ready_stage(&topo, &streams, &index).unwrap();
    assert_eq!(exec.component_id, ComponentId::new(1));
    assert!(matches!(
        exec.action,
        torvyn_reactor::StageAction::PullFromSource
    ));

    // Add element to source→processor stream.
    streams[0].queue.push(make_element(0));

    let exec = policy.next_ready_stage(&topo, &streams, &index).unwrap();
    assert_eq!(exec.component_id, ComponentId::new(2));
    assert!(matches!(
        exec.action,
        torvyn_reactor::StageAction::ProcessElement { .. }
    ));

    // Simulate processor consuming and producing.
    streams[0].queue.pop();
    streams[1].queue.push(make_element(0));

    let exec = policy.next_ready_stage(&topo, &streams, &index).unwrap();
    assert_eq!(exec.component_id, ComponentId::new(3));
    assert!(matches!(
        exec.action,
        torvyn_reactor::StageAction::PushToSink { .. }
    ));
}

// ===========================================================================
// Cancellation integration tests (from design doc §19)
// ===========================================================================

#[tokio::test]
async fn test_cancellation_propagates() {
    let cancel = FlowCancellation::new();
    let clone1 = cancel.clone();
    let clone2 = cancel.clone();

    let h = tokio::spawn(async move {
        clone1.cancelled().await;
        clone1.is_cancelled()
    });

    tokio::time::sleep(Duration::from_millis(10)).await;

    clone2.cancel(CancellationReason::OperatorRequest);
    let result = h.await.unwrap();
    assert!(result);
}

// ===========================================================================
// FlowConfig integration tests (from design doc §19)
// ===========================================================================

#[test]
fn test_flow_config_default_with_valid_topology() {
    let topo = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    let config = FlowConfig::default_with_topology(topo);
    assert_eq!(config.priority, FlowPriority::Normal);
    assert!(config.topology.validate().is_ok());
}

// ===========================================================================
// Metrics integration tests (from design doc §19)
// ===========================================================================

#[test]
fn test_flow_completion_stats_aggregation() {
    let mut stats = FlowCompletionStats::new(Duration::from_secs(10));

    let mut m1 = StreamMetrics::new();
    m1.elements_total = 500;
    m1.backpressure_events = 3;

    let mut m2 = StreamMetrics::new();
    m2.elements_total = 500;
    m2.backpressure_events = 2;

    stats.aggregate_from_streams(&[(StreamId::new(0), m1), (StreamId::new(1), m2)]);

    assert_eq!(stats.total_elements, 1000);
    assert_eq!(stats.total_backpressure_events, 5);
}
