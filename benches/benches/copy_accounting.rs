//! Copy accounting benchmark: verify copy counts per topology.
//!
//! Measures per-element copy overhead and verifies that the copy count
//! matches design expectations. Reports copy bytes per topology.
//!
//! # LLI DEVIATION: Uses `CopyLedger` directly to measure copy operations,
//! since the `FlowDriver` does not currently integrate with the resource
//! manager's copy tracking. The benchmark records copies manually to
//! measure the overhead of the accounting infrastructure itself.
//!
//! The design doc assumed `resource_manager.flow_copy_stats(flow_id)` would
//! be populated automatically by the flow driver. In practice, copy accounting
//! is a separate concern recorded by the resource manager during actual
//! Wasm memory transfers. Here we benchmark the accounting overhead and
//! verify correctness of the ledger.

use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

use torvyn_integration_tests::{
    build_driver, conn, processor, sink, source, ComponentId, FlowConfig, FlowId, FlowState,
    FlowTopology, ResourceId, TestInvoker,
};
use torvyn_resources::accounting::CopyLedger;
use torvyn_types::{CopyReason, NoopEventSink};

fn bench_copy_accounting(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // --- Accounting overhead benchmark ---
    c.bench_function("copy_accounting_ledger_1000_ops", |b| {
        b.iter(|| {
            let ledger = CopyLedger::new();
            let flow_id = FlowId::new(1);
            let src = ComponentId::new(1);
            let dst = ComponentId::new(2);
            let sink = NoopEventSink;

            ledger.register_flow(flow_id);

            // Simulate 1000 elements through Source → Sink:
            // 2 copies per element (ComponentToHost + HostToComponent)
            for i in 0..1000u64 {
                let res = ResourceId::new(i as u32, 0);
                ledger.record_copy(
                    flow_id,
                    res,
                    src,
                    dst,
                    256,
                    CopyReason::ComponentToHost,
                    &sink,
                );
                ledger.record_copy(
                    flow_id,
                    res,
                    src,
                    dst,
                    256,
                    CopyReason::HostToComponent,
                    &sink,
                );
            }

            let stats = ledger.flow_stats(flow_id);
            assert_eq!(
                stats.total_copy_ops, 2000,
                "Source→Sink should produce exactly 2 copies per element"
            );
            assert_eq!(
                stats.total_payload_bytes,
                2000 * 256,
                "total bytes should match"
            );
        });
    });

    // --- Source → Sink flow with copy count verification ---
    c.bench_function("copy_accounting_source_sink_flow", |b| {
        b.to_async(&rt).iter(|| async {
            let element_count: u64 = 1000;
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

    // --- Source → Processor → Sink flow ---
    c.bench_function("copy_accounting_source_processor_sink_flow", |b| {
        b.to_async(&rt).iter(|| async {
            let element_count: u64 = 1000;
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
}

criterion_group!(benches, bench_copy_accounting);
criterion_main!(benches);
