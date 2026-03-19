//! End-to-end test: 10 concurrent flows, verify fairness.
//!
//! All flows should complete within a reasonable time factor of each other.
//!
//! # LLI DEVIATION: Uses `FlowDriver::run()` spawned via `tokio::spawn`
//! instead of the design doc's `handle.submit_flow()` / `handle.wait_for_completion()`.
//! The actual reactor does not have a `wait_for_completion` method on the handle.

use std::time::Instant;

use torvyn_integration_tests::*;

#[tokio::test]
async fn test_multiple_flows_10_concurrent() {
    const FLOW_COUNT: u64 = 10;
    const ELEMENTS_PER_FLOW: u64 = 1000;

    let mut handles = Vec::new();

    for i in 0..FLOW_COUNT {
        let flow_id = FlowId::new(i + 1);
        let topology = FlowTopology {
            stages: vec![source(i * 10 + 1), sink(i * 10 + 2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();
        let config = FlowConfig::default_with_topology(topology.clone());
        let invoker = TestInvoker::new(ELEMENTS_PER_FLOW);

        let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
        handles.push(tokio::spawn(async move { driver.run().await }));
    }

    let start = Instant::now();
    let mut completion_times = Vec::new();

    for h in handles {
        let (_, state, stats) = h.await.unwrap();
        assert_eq!(state, FlowState::Completed);
        assert_eq!(stats.total_elements, ELEMENTS_PER_FLOW);
        completion_times.push(start.elapsed());
    }

    // Fairness check: slowest flow should complete within 10x of fastest.
    let fastest = completion_times.iter().min().unwrap();
    let slowest = completion_times.iter().max().unwrap();

    if fastest.as_micros() > 0 {
        let ratio = slowest.as_micros() as f64 / fastest.as_micros() as f64;
        assert!(
            ratio < 10.0,
            "fairness violation: slowest={slowest:?}, fastest={fastest:?}, ratio={ratio:.1}x"
        );
    }
}

#[tokio::test]
async fn test_multiple_flows_different_sizes() {
    let sizes: Vec<u64> = vec![10, 100, 500, 50, 200];

    let mut handles = Vec::new();
    for (i, &size) in sizes.iter().enumerate() {
        let flow_id = FlowId::new(i as u64 + 1);
        let topology = FlowTopology {
            stages: vec![source(i as u64 * 10 + 1), sink(i as u64 * 10 + 2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();
        let config = FlowConfig::default_with_topology(topology.clone());
        let invoker = TestInvoker::new(size);

        let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
        handles.push(tokio::spawn(async move { (driver.run().await, size) }));
    }

    for h in handles {
        let ((_, state, stats), expected_size) = h.await.unwrap();
        assert_eq!(state, FlowState::Completed);
        assert_eq!(stats.total_elements, expected_size);
    }
}
