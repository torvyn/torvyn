//! Latency benchmark: measure p50/p95/p99/p99.9 for Source → Sink.
//!
//! Uses criterion for statistical rigor. Measures end-to-end latency
//! of flowing N elements through a Source → Sink topology.
//!
//! # LLI DEVIATION: Uses `TestInvoker` + `build_driver` + `FlowDriver::run()`
//! instead of the design doc's `MockInvoker` / `ReactorCoordinator` / `handle.submit_flow()`,
//! which do not exist in the actual implementation.

use std::sync::{Arc, Mutex};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::runtime::Runtime;

use torvyn_integration_tests::{
    build_driver, conn, sink, source, FlowConfig, FlowId, FlowState, FlowTopology, PushBehavior,
    TestInvoker,
};

fn create_source_sink_topology() -> FlowTopology {
    FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    }
}

fn bench_source_to_sink_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("source_to_sink_latency");
    group.sample_size(50);
    group.measurement_time(std::time::Duration::from_secs(10));

    for &element_count in &[100u64, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::new("elements", element_count),
            &element_count,
            |b, &count| {
                b.to_async(&rt).iter(|| async move {
                    let collected = Arc::new(Mutex::new(Vec::new()));
                    let invoker = TestInvoker::new(count)
                        .with_push(PushBehavior::CollectSequences(collected));

                    let topology = create_source_sink_topology();
                    topology.validate().unwrap();
                    let config = FlowConfig::default_with_topology(topology.clone());
                    let flow_id = FlowId::new(1);

                    let (driver, _cancel, _rx) =
                        build_driver(invoker, flow_id, topology, config).await;
                    let (_id, state, stats) = driver.run().await;

                    assert_eq!(state, FlowState::Completed);
                    assert_eq!(stats.total_elements, count);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_source_to_sink_latency);
criterion_main!(benches);
