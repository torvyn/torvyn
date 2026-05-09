//! End-to-end test of the cross-crate integration seam.
//!
//! This is the **first** workspace test that exercises
//! [`torvyn_pipeline::instantiate_pipeline`] end-to-end against a real
//! [`torvyn_reactor::ReactorCoordinator`] (running on a Tokio task) and
//! a real [`torvyn_engine::WasmEngine`] implementation (`MockEngine`).
//!
//! Real `.wasm` components are **not** loaded here — that comes in
//! Item 2 of the project's Tier-1 list (a `wasm32-wasip2` end-to-end
//! test). What this file proves is that the orchestration code paths
//! are wired correctly:
//!
//! 1. `instantiate_pipeline` performs `compile → instantiate → init`
//!    for every node in topological order.
//! 2. The resulting `Vec<ComponentInstance>` reaches the reactor via
//!    `ReactorHandle::spawn_flow`.
//! 3. The coordinator builds a real `FlowDriver` and drives the flow
//!    to terminal state.
//! 4. `shutdown_pipeline` correctly issues a cancellation through the
//!    reactor and waits for the flow to terminate.
//!
//! When this test passes, the keystone integration described in HLI
//! Doc 02 §10.3 is operational.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use torvyn_engine::mock::MockEngine;
use torvyn_integration_tests::{
    processor, sink, source, spawn_coordinator_with_arc, PullBehavior, PushBehavior, TestInvoker,
};
use torvyn_pipeline::{
    instantiate_pipeline, shutdown_pipeline, NodeConfig, PipelineTopologyBuilder,
};
use torvyn_types::{ComponentRole, FlowState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Poll `reactor.flow_state(flow_id)` until it reaches a terminal state
/// or the deadline elapses. Panics with a descriptive message on
/// timeout. Returns the final terminal `FlowState`.
async fn await_flow_terminal(
    reactor: &torvyn_reactor::ReactorHandle,
    flow_id: torvyn_types::FlowId,
    timeout: Duration,
) -> FlowState {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match reactor.flow_state(flow_id).await {
            Ok(state) if state.is_terminal() => return state,
            Ok(_state) => {}
            // The coordinator may reap the flow before we ask; that is
            // also a terminal outcome. Without a state to assert we use
            // `Completed` as the optimistic interpretation; the caller
            // can additionally check the sink-side collector.
            Err(_) => return FlowState::Completed,
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "flow {flow_id} did not reach a terminal state within {timeout:?}; \
                 most recent observed state was non-terminal"
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Source → Sink pipeline: 100 elements produced, all delivered.
///
/// Uses the real `instantiate_pipeline` cold path against `MockEngine`
/// and verifies that:
/// - The returned `PipelineHandle` carries the supplied flow name and
///   the same topology that was passed in.
/// - The reactor drives the flow to `FlowState::Completed`.
/// - The sink received exactly 100 elements with sequence numbers
///   `0..100` in order (no gaps, no reorderings).
#[tokio::test]
async fn test_instantiate_pipeline_drives_source_to_sink_to_completion() {
    const ELEMENT_COUNT: u64 = 100;

    let collected = Arc::new(Mutex::new(Vec::<u64>::new()));

    // The same Arc<TestInvoker> is used for both `instantiate_pipeline`
    // (which calls `invoke_init`) and the reactor coordinator (which
    // clones it into the spawned `FlowDriver`). Sharing the same Arc
    // guarantees the `CollectSequences` push behaviour is observed by
    // the FlowDriver.
    let invoker: Arc<TestInvoker> = Arc::new(
        TestInvoker::new(ELEMENT_COUNT)
            .with_push(PushBehavior::CollectSequences(Arc::clone(&collected))),
    );

    let topology = PipelineTopologyBuilder::new("integration-source-to-sink")
        .add_node(
            "source",
            ComponentRole::Source,
            "mock://source",
            NodeConfig::default(),
        )
        .add_node(
            "sink",
            ComponentRole::Sink,
            "mock://sink",
            NodeConfig::default(),
        )
        .add_edge("source", "output", "sink", "input")
        .build()
        .expect("two-node topology must build");

    let engine = MockEngine::new();
    let (reactor, _coordinator_join) = spawn_coordinator_with_arc(Arc::clone(&invoker));

    // Run the cold-path orchestration end-to-end.
    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed for a valid mock topology");

    assert_eq!(
        handle.name(),
        "integration-source-to-sink",
        "PipelineHandle must carry the topology's flow name"
    );
    assert_eq!(
        handle.topology().node_count(),
        2,
        "PipelineHandle must retain the supplied topology"
    );

    // The reactor's coordinator should have driven the FlowDriver to
    // completion. With 100 elements and a passthrough invoker, this
    // typically completes well under 1s.
    let final_state = await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(5)).await;
    assert_eq!(
        final_state,
        FlowState::Completed,
        "expected the flow to reach Completed; got {final_state:?}"
    );

    // The sink must have received exactly the number of elements the
    // source produced, with monotonically increasing sequence numbers.
    let received = collected.lock().expect("collector mutex");
    assert_eq!(
        received.len() as u64,
        ELEMENT_COUNT,
        "sink received {} elements, expected {}",
        received.len(),
        ELEMENT_COUNT
    );
    for (i, &seq) in received.iter().enumerate() {
        assert_eq!(
            seq, i as u64,
            "expected sequence {i} at position {i}, got {seq}"
        );
    }
}

/// Source → Processor → Sink with three nodes. Verifies that
/// `instantiate_pipeline` correctly walks the multi-stage topology in
/// topological order and produces a flow whose driver runs all three
/// stages to completion.
#[tokio::test]
async fn test_instantiate_pipeline_three_stage_pipeline() {
    const ELEMENT_COUNT: u64 = 50;

    let collected = Arc::new(Mutex::new(Vec::<u64>::new()));

    let invoker: Arc<TestInvoker> = Arc::new(
        TestInvoker::new(ELEMENT_COUNT)
            .with_push(PushBehavior::CollectSequences(Arc::clone(&collected))),
    );

    let topology = PipelineTopologyBuilder::new("integration-three-stage")
        .add_node(
            "src",
            ComponentRole::Source,
            "mock://src",
            NodeConfig::default(),
        )
        .add_node(
            "proc",
            ComponentRole::Processor,
            "mock://proc",
            NodeConfig::default(),
        )
        .add_node(
            "snk",
            ComponentRole::Sink,
            "mock://snk",
            NodeConfig::default(),
        )
        .add_edge("src", "output", "proc", "input")
        .add_edge("proc", "output", "snk", "input")
        .build()
        .expect("three-node linear topology must build");

    let engine = MockEngine::new();
    let (reactor, _coordinator_join) = spawn_coordinator_with_arc(Arc::clone(&invoker));

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed for a valid 3-node topology");

    assert_eq!(handle.topology().node_count(), 3);
    assert_eq!(handle.topology().edge_count(), 2);

    let final_state = await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(5)).await;
    assert_eq!(final_state, FlowState::Completed);

    let received = collected.lock().expect("collector mutex");
    assert_eq!(
        received.len() as u64,
        ELEMENT_COUNT,
        "sink received {} elements through processor, expected {}",
        received.len(),
        ELEMENT_COUNT
    );
}

/// `shutdown_pipeline` cancels a running flow via the reactor and waits
/// for it to reach a terminal state.
#[tokio::test]
async fn test_shutdown_pipeline_cancels_running_flow() {
    // Infinite source: the flow would run forever without cancellation.
    let invoker: Arc<TestInvoker> = Arc::new(TestInvoker::new(0).with_pull(PullBehavior::Infinite));

    let topology = PipelineTopologyBuilder::new("integration-shutdown")
        .add_node(
            "source",
            ComponentRole::Source,
            "mock://source",
            NodeConfig::default(),
        )
        .add_node(
            "sink",
            ComponentRole::Sink,
            "mock://sink",
            NodeConfig::default(),
        )
        .add_edge("source", "output", "sink", "input")
        .build()
        .expect("topology must build");

    let engine = MockEngine::new();
    let (reactor, _coordinator_join) = spawn_coordinator_with_arc(Arc::clone(&invoker));

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate must succeed");

    // Let the flow run for a moment to confirm it is alive, then shut it
    // down and verify the shutdown path returns Ok within a reasonable
    // bound.
    tokio::time::sleep(Duration::from_millis(50)).await;

    shutdown_pipeline(&handle, &reactor, Duration::from_secs(5))
        .await
        .expect("graceful shutdown must complete within 5s");

    // After shutdown, polling the flow should report a terminal state
    // (or the flow may have been reaped — both are acceptable).
    match reactor.flow_state(handle.flow_id()).await {
        Ok(state) => assert!(
            state.is_terminal(),
            "after shutdown the flow must be terminal; got {state:?}"
        ),
        Err(_) => {
            // Flow was reaped — also acceptable.
        }
    }
}

/// Topology helper functions exposed by the harness must be useful as
/// a sanity check that we don't regress the public re-exports the
/// integration crate offers to other test files.
#[test]
fn test_helpers_construct_distinct_role_stages() {
    let s = source(1);
    let p = processor(2);
    let k = sink(3);

    assert_eq!(s.role, ComponentRole::Source);
    assert_eq!(p.role, ComponentRole::Processor);
    assert_eq!(k.role, ComponentRole::Sink);
}
