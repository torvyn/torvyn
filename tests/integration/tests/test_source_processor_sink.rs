//! End-to-end test: Source → Processor → Sink with element transformation.
//!
//! Verifies that the processor stage participates in the pipeline and
//! all elements flow through correctly.
//!
//! # LLI DEVIATION: Uses `TestInvoker` with `ProcessBehavior::Passthrough`
//! instead of the design doc's `MockComponentBehavior::DoublePayload`.
//! The actual mock invoker does not support payload transformation at the
//! BufferHandle level. Tests verify correct element count through 3 stages.

use torvyn_integration_tests::*;

#[tokio::test]
async fn test_source_processor_sink_100_elements() {
    const ELEMENT_COUNT: u64 = 100;

    let invoker = TestInvoker::new(ELEMENT_COUNT);

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
    // Each element traverses 2 streams (source→proc, proc→sink),
    // so total_elements = ELEMENT_COUNT * 2.
    assert_eq!(stats.total_elements, ELEMENT_COUNT * 2);
}

#[tokio::test]
async fn test_source_processor_sink_500_elements() {
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
    assert_eq!(stats.total_elements, 1000);
}

#[tokio::test]
async fn test_two_processors_in_chain() {
    let invoker = TestInvoker::new(50);

    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), processor(2), processor(3), sink(4)],
        connections: vec![conn(0, 1), conn(1, 2), conn(2, 3)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, state, stats) = driver.run().await;

    assert_eq!(state, FlowState::Completed);
    // 3 streams × 50 elements = 150 total
    assert_eq!(stats.total_elements, 150);
}
