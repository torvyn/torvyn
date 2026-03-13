//! Integration tests for the metrics subsystem.

use torvyn_observability::metrics::flow_metrics::FlowMetrics;
use torvyn_observability::metrics::pool_metrics::{PoolId, ResourcePoolMetrics};
use torvyn_observability::metrics::*;
use torvyn_types::{ComponentId, FlowId, StreamId};

#[test]
fn test_counter_basic_operations() {
    let c = Counter::new();
    assert_eq!(c.read(), 0);
    c.increment(10);
    c.increment(20);
    assert_eq!(c.read(), 30);
    let prev = c.reset();
    assert_eq!(prev, 30);
    assert_eq!(c.read(), 0);
}

#[test]
fn test_histogram_percentiles() {
    let h = Histogram::new(LATENCY_BUCKETS_NS);

    // Record 1000 values from 100ns to 100000ns.
    for i in 0..1000u64 {
        h.record(100 + i * 100);
    }

    let snap = h.snapshot();
    let p50 = snap.percentile(50.0);
    let p99 = snap.percentile(99.0);

    assert!(p50 < p99, "p50 ({p50}) should be less than p99 ({p99})");
    assert!(p50 > 0);
    assert!(p99 > 0);
}

#[test]
fn test_gauge_operations() {
    let g = Gauge::new();
    g.set(100);
    g.decrement(30);
    g.increment(10);
    assert_eq!(g.read(), 80);
}

#[test]
fn test_flow_metrics_lifecycle() {
    let fm = FlowMetrics::new(
        FlowId::new(1),
        &[
            ComponentId::new(1),
            ComponentId::new(2),
            ComponentId::new(3),
        ],
        &[StreamId::new(1), StreamId::new(2)],
        0,
    );

    assert_eq!(fm.components.len(), 3);
    assert_eq!(fm.streams.len(), 2);

    // Record through components.
    for comp in &fm.components {
        comp.invocations.increment(100);
    }

    for stream in &fm.streams {
        stream.elements.increment(100);
    }

    assert_eq!(
        fm.component(ComponentId::new(1))
            .unwrap()
            .invocations
            .read(),
        100
    );
    assert_eq!(
        fm.stream(StreamId::new(2)).unwrap().elements.read(),
        100
    );
}

#[test]
fn test_pool_metrics_tracking() {
    let pm = ResourcePoolMetrics::new(PoolId::new(0));
    pm.allocations.increment(100);
    pm.deallocations.increment(80);
    pm.reuses.increment(60);
    pm.active_buffers.set(20);
    pm.utilization_permille.set(750);

    assert_eq!(pm.allocations.read(), 100);
    assert_eq!(pm.active_buffers.read(), 20);
    assert!((pm.utilization() - 0.75).abs() < 0.001);
}

#[test]
fn test_metrics_registry_multiple_flows() {
    let reg = MetricsRegistry::new();

    for i in 0..10 {
        reg.register_flow(
            FlowId::new(i),
            &[ComponentId::new(i * 10)],
            &[StreamId::new(i * 10)],
            0,
        )
        .unwrap();
    }

    assert_eq!(reg.system.active_flows.read(), 10);
    assert_eq!(reg.active_flow_ids().len(), 10);

    for i in 0..5 {
        reg.deregister_flow(FlowId::new(i)).unwrap();
    }

    assert_eq!(reg.system.active_flows.read(), 5);
}
