//! End-to-end test: Component crash isolation.
//!
//! A processor component returns a fatal error after N elements.
//! The flow should transition to `Failed` state.
//!
//! # LLI DEVIATION: Uses `TestInvoker` with `ProcessBehavior::FailAfterN`
//! instead of the design doc's `MockComponentBehavior::FailAfterN`.
//! The design doc also sets `config.error_policy = ErrorPolicy::FailFast`
//! via a field that uses `torvyn_reactor::ErrorPolicy`, which is the default.

use torvyn_integration_tests::*;

#[tokio::test]
async fn test_component_crash_after_50_elements() {
    const ELEMENTS_BEFORE_CRASH: u64 = 50;

    let invoker =
        TestInvoker::new(200).with_process(ProcessBehavior::FailAfterN(ELEMENTS_BEFORE_CRASH));

    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), processor(2), sink(3)],
        connections: vec![conn(0, 1), conn(1, 2)],
    };
    topology.validate().unwrap();

    // ErrorPolicy::FailFast is the default.
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, state, _) = driver.run().await;

    assert_eq!(
        state,
        FlowState::Failed,
        "flow should enter Failed state on fatal process error"
    );
}

#[tokio::test]
async fn test_source_error_transitions_to_failed() {
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

#[tokio::test]
async fn test_crash_state_transitions() {
    let invoker = TestInvoker::new(100).with_process(ProcessBehavior::FailAfterN(10));

    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), processor(2), sink(3)],
        connections: vec![conn(0, 1), conn(1, 2)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, _cancel, mut event_rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_, final_state, _) = driver.run().await;

    assert_eq!(final_state, FlowState::Failed);

    // Verify that state transitions were emitted.
    let mut saw_running = false;
    let mut saw_failed = false;
    while let Ok(event) = event_rx.try_recv() {
        if let ReactorEvent::FlowStateChanged { new_state, .. } = event {
            if new_state == FlowState::Running {
                saw_running = true;
            }
            if new_state == FlowState::Failed {
                saw_failed = true;
            }
        }
    }
    assert!(saw_running, "should have transitioned to Running");
    assert!(saw_failed, "should have transitioned to Failed");
}
