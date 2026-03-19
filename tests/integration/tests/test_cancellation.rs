//! End-to-end test: Cancel a flow mid-execution.
//!
//! Verifies graceful shutdown when a flow is cancelled while running.
//!
//! # LLI DEVIATION: Uses `FlowCancellation::cancel(CancellationReason)`
//! instead of `handle.cancel_flow(flow_id, "reason")`. The actual API
//! uses enum-based cancellation reasons, not string messages.

use std::time::{Duration, Instant};

use torvyn_integration_tests::*;

#[tokio::test]
async fn test_cancellation_infinite_source() {
    let invoker = TestInvoker::infinite();

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
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel(CancellationReason::OperatorRequest);
    });

    let (_, state, _) = driver.run().await;
    let elapsed = start.elapsed();
    cancel_task.await.unwrap();

    assert_eq!(
        state,
        FlowState::Cancelled,
        "flow should be in Cancelled state after cancellation"
    );

    // Cleanup should be fast.
    assert!(
        elapsed < Duration::from_secs(2),
        "cancellation took too long: {elapsed:?}"
    );
}

#[tokio::test]
async fn test_cancellation_with_processor() {
    let invoker = TestInvoker::infinite();

    let flow_id = FlowId::new(1);
    let topology = FlowTopology {
        stages: vec![source(1), processor(2), sink(3)],
        connections: vec![conn(0, 1), conn(1, 2)],
    };
    topology.validate().unwrap();
    let config = FlowConfig::default_with_topology(topology.clone());

    let (driver, cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;

    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel(CancellationReason::OperatorRequest);
    });

    let (_, state, _) = driver.run().await;
    cancel_task.await.unwrap();

    assert_eq!(state, FlowState::Cancelled);
}
