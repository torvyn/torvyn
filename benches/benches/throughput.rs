//! Throughput benchmark: max sustained elements/second.
//!
//! Measures throughput for:
//! - Source → Sink (direct pass-through)
//! - Source → Processor → Sink (single-stage transformation)
//!
//! # LLI DEVIATION: Uses `TestInvoker` + `build_driver` + `FlowDriver::run()`
//! instead of the design doc's `MockInvoker` / `ReactorCoordinator` API.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use tokio::runtime::Runtime;

use torvyn_integration_tests::{
    build_driver, conn, processor, sink, source, FlowConfig, FlowId, FlowState, FlowTopology,
    TestInvoker,
};

fn bench_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("throughput");
    group.sample_size(20);
    group.measurement_time(std::time::Duration::from_secs(15));

    let element_count: u64 = 100_000;
    group.throughput(Throughput::Elements(element_count));

    // --- Source → Sink ---
    group.bench_function("source_sink_100k", |b| {
        b.to_async(&rt).iter(|| async {
            let invoker = TestInvoker::new(element_count);

            let topology = FlowTopology {
                stages: vec![source(1), sink(2)],
                connections: vec![conn(0, 1)],
            };
            topology.validate().unwrap();
            let config = FlowConfig::default_with_topology(topology.clone());
            let flow_id = FlowId::new(1);

            let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
            let (_id, state, stats) = driver.run().await;

            assert_eq!(state, FlowState::Completed);
            assert_eq!(stats.total_elements, element_count);
        });
    });

    // --- Source → Processor → Sink ---
    group.bench_function("source_processor_sink_100k", |b| {
        b.to_async(&rt).iter(|| async {
            let invoker = TestInvoker::new(element_count);

            let topology = FlowTopology {
                stages: vec![source(1), processor(2), sink(3)],
                connections: vec![conn(0, 1), conn(1, 2)],
            };
            topology.validate().unwrap();
            let config = FlowConfig::default_with_topology(topology.clone());
            let flow_id = FlowId::new(1);

            let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
            let (_id, state, stats) = driver.run().await;

            assert_eq!(state, FlowState::Completed);
            // 2 streams × element_count = total stream transfers
            assert_eq!(stats.total_elements, element_count * 2);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_throughput);
criterion_main!(benches);
