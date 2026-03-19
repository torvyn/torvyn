//! End-to-end test: Source → Sink with 1000 elements.
//!
//! Verifies element ordering, count, and completion state.
//!
//! # LLI DEVIATION: Uses `FlowDriver::run()` directly instead of the design
//! doc's `handle.submit_flow()` / `handle.wait_for_completion()` API, which
//! does not exist. Uses `TestInvoker` instead of `MockInvoker` with
//! `MockComponentBehavior::ProduceN`.

use std::sync::{Arc, Mutex};

use torvyn_integration_tests::*;

#[tokio::test]
async fn test_source_to_sink_1000_elements() {
    const ELEMENT_COUNT: u64 = 1000;

    let collected = Arc::new(Mutex::new(Vec::new()));

    let invoker = TestInvoker::new(ELEMENT_COUNT)
        .with_push(PushBehavior::CollectSequences(collected.clone()));

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
    assert_eq!(stats.total_elements, ELEMENT_COUNT);

    // Verify element count and ordering.
    let elements = collected.lock().unwrap();
    assert_eq!(
        elements.len() as u64,
        ELEMENT_COUNT,
        "sink should receive exactly {ELEMENT_COUNT} elements"
    );

    // Verify ordering: sequences should be monotonically increasing.
    for (i, &seq) in elements.iter().enumerate() {
        assert_eq!(
            seq, i as u64,
            "element {i} should have sequence {i}, got {seq}"
        );
    }
}

#[tokio::test]
async fn test_source_to_sink_small() {
    let invoker = TestInvoker::new(10);

    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, state, stats) = driver.run().await;

    assert_eq!(state, FlowState::Completed);
    assert_eq!(stats.total_elements, 10);
}

#[tokio::test]
async fn test_source_to_sink_zero_elements() {
    let invoker = TestInvoker::new(0);

    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, state, stats) = driver.run().await;

    assert_eq!(state, FlowState::Completed);
    assert_eq!(stats.total_elements, 0);
}
