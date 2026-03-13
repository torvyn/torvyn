//! Integration tests for benchmark report generation.

use std::time::Duration;
use torvyn_observability::bench::*;
use torvyn_observability::metrics::flow_metrics::FlowMetrics;
use torvyn_observability::metrics::snapshot::snapshot_flow;
use torvyn_types::{ComponentId, FlowId, StreamId};

#[test]
fn test_benchmark_report_generation_and_formatting() {
    let fm = FlowMetrics::new(
        FlowId::new(1),
        &[ComponentId::new(1), ComponentId::new(2)],
        &[StreamId::new(1)],
        0,
    );

    let start = snapshot_flow(&fm);

    // Simulate workload.
    for _ in 0..10_000 {
        fm.elements_total.increment(1);
        fm.end_to_end_latency.record(5_000);
        fm.copies_total.increment(1);
        fm.copy_bytes_total.increment(64);
    }

    let end = snapshot_flow(&fm);

    let report = generate_report(
        "source → sink".into(),
        Duration::from_secs(30),
        Duration::from_secs(5),
        1000,
        &start,
        &end,
        15_000,
    );

    assert_eq!(report.elements_processed, 10_000);
    assert!(report.throughput.sustained_elements_per_sec > 0.0);
    assert_eq!(report.data_movement.total_copies, 10_000);
    assert_eq!(report.data_movement.total_copy_bytes, 640_000);

    // Format should not panic.
    let formatted = format_report(&report);
    assert!(formatted.contains("Torvyn Benchmark Report"));

    // JSON roundtrip.
    let json = serde_json::to_string_pretty(&report).unwrap();
    let parsed: BenchmarkReport = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.elements_processed, 10_000);
}

#[test]
fn test_benchmark_report_empty_flow() {
    let fm = FlowMetrics::new(
        FlowId::new(1),
        &[ComponentId::new(1)],
        &[StreamId::new(1)],
        0,
    );

    let start = snapshot_flow(&fm);
    let end = snapshot_flow(&fm);

    let report = generate_report(
        "empty".into(),
        Duration::from_secs(1),
        Duration::ZERO,
        0,
        &start,
        &end,
        0,
    );

    assert_eq!(report.elements_processed, 0);
    assert_eq!(report.throughput.sustained_elements_per_sec, 0.0);
}
