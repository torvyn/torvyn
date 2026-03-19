//! Benchmark report generation.
//!
//! Per HLI Doc 05 §6.

use crate::metrics::snapshot::{delta, FlowMetricsSnapshot};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Complete benchmark report.
///
/// Per HLI Doc 05 §9.4.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Human-readable pipeline description (e.g., "source → transform → sink").
    pub pipeline_description: String,
    /// Total measurement duration (excluding warmup).
    pub duration: Duration,
    /// Total elements processed during measurement.
    pub elements_processed: u64,
    /// Duration of warmup phase.
    pub warmup_duration: Duration,
    /// Elements processed during warmup.
    pub warmup_elements: u64,
    /// Latency statistics.
    pub latency: LatencyReport,
    /// Throughput statistics.
    pub throughput: ThroughputReport,
    /// Data movement statistics.
    pub data_movement: DataMovementReport,
    /// Per-stream queue pressure statistics.
    pub queue_pressure: Vec<QueuePressureReport>,
    /// Resource usage statistics.
    pub resource_usage: ResourceUsageReport,
    /// Per-component breakdown.
    pub component_breakdown: Vec<ComponentBreakdownEntry>,
}

/// End-to-end latency percentiles.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatencyReport {
    /// 50th percentile (median) latency in nanoseconds.
    pub p50_ns: u64,
    /// 95th percentile latency in nanoseconds.
    pub p95_ns: u64,
    /// 99th percentile latency in nanoseconds.
    pub p99_ns: u64,
    /// 99.9th percentile latency in nanoseconds.
    pub p999_ns: u64,
    /// Minimum observed latency in nanoseconds.
    pub min_ns: u64,
    /// Maximum observed latency in nanoseconds.
    pub max_ns: u64,
    /// Mean latency in nanoseconds.
    pub mean_ns: f64,
}

/// Throughput statistics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThroughputReport {
    /// Sustained elements per second.
    pub sustained_elements_per_sec: f64,
}

/// Data movement statistics (copies).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataMovementReport {
    /// Total copy operations.
    pub total_copies: u64,
    /// Total bytes copied.
    pub total_copy_bytes: u64,
    /// Average copies per element.
    pub avg_copies_per_element: f64,
}

/// Per-stream queue pressure statistics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuePressureReport {
    /// Stream identifier.
    pub stream_id: torvyn_types::StreamId,
    /// Average queue depth.
    pub avg_depth: f64,
    /// Peak queue depth observed.
    pub peak_depth: u64,
    /// Number of backpressure activation events.
    pub backpressure_events: u64,
}

/// Resource usage statistics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceUsageReport {
    /// Total scheduler wakeups during measurement.
    pub scheduler_wakeups: u64,
}

/// Per-component performance breakdown entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentBreakdownEntry {
    /// Component instance identifier.
    pub component_id: torvyn_types::ComponentId,
    /// Average processing time per invocation in nanoseconds.
    pub avg_processing_time_ns: f64,
    /// 99th percentile processing time in nanoseconds.
    pub p99_processing_time_ns: u64,
    /// Total error count.
    pub error_count: u64,
    /// Percentage contribution to total pipeline latency.
    pub latency_contribution_pct: f64,
}

/// Generate a benchmark report from start and end snapshots.
///
/// # COLD PATH
pub fn generate_report(
    pipeline_description: String,
    duration: Duration,
    warmup_duration: Duration,
    warmup_elements: u64,
    start_snapshot: &FlowMetricsSnapshot,
    end_snapshot: &FlowMetricsSnapshot,
    scheduler_wakeups: u64,
) -> BenchmarkReport {
    let d = delta(start_snapshot, end_snapshot);

    let elements = d.elements_total;
    let duration_secs = duration.as_secs_f64();

    let sustained = if duration_secs > 0.0 {
        elements as f64 / duration_secs
    } else {
        0.0
    };

    let avg_copies = if elements > 0 {
        d.copies_total as f64 / elements as f64
    } else {
        0.0
    };

    let total_latency: f64 = d.components.iter().map(|c| c.processing_time_mean_ns).sum();

    let component_breakdown = d
        .components
        .iter()
        .map(|c| {
            let pct = if total_latency > 0.0 {
                c.processing_time_mean_ns / total_latency * 100.0
            } else {
                0.0
            };
            ComponentBreakdownEntry {
                component_id: c.component_id,
                avg_processing_time_ns: c.processing_time_mean_ns,
                p99_processing_time_ns: c.processing_time_p99_ns,
                error_count: c.errors,
                latency_contribution_pct: pct,
            }
        })
        .collect();

    let queue_pressure = d
        .streams
        .iter()
        .map(|s| QueuePressureReport {
            stream_id: s.stream_id,
            avg_depth: s.queue_depth as f64,
            peak_depth: s.queue_depth_peak,
            backpressure_events: s.backpressure_events,
        })
        .collect();

    BenchmarkReport {
        pipeline_description,
        duration,
        elements_processed: elements,
        warmup_duration,
        warmup_elements,
        latency: LatencyReport {
            p50_ns: end_snapshot.latency_p50_ns,
            p95_ns: end_snapshot.latency_p95_ns,
            p99_ns: end_snapshot.latency_p99_ns,
            p999_ns: end_snapshot.latency_p999_ns,
            min_ns: end_snapshot.latency_min_ns,
            max_ns: end_snapshot.latency_max_ns,
            mean_ns: end_snapshot.latency_mean_ns,
        },
        throughput: ThroughputReport {
            sustained_elements_per_sec: sustained,
        },
        data_movement: DataMovementReport {
            total_copies: d.copies_total,
            total_copy_bytes: d.copy_bytes_total,
            avg_copies_per_element: avg_copies,
        },
        queue_pressure,
        resource_usage: ResourceUsageReport { scheduler_wakeups },
        component_breakdown,
    }
}

/// Format a benchmark report as a human-readable table.
///
/// # COLD PATH
pub fn format_report(report: &BenchmarkReport) -> String {
    let mut out = String::with_capacity(2048);

    out.push_str("╔══════════════════════════════════════════════════════════════╗\n");
    out.push_str("║                    Torvyn Benchmark Report                   ║\n");
    out.push_str(&format!(
        "║  Pipeline: {:<49}║\n",
        &report.pipeline_description
    ));
    out.push_str(&format!(
        "║  Duration: {:.2}s | Elements: {:<30}║\n",
        report.duration.as_secs_f64(),
        report.elements_processed
    ));
    out.push_str("╠══════════════════════════════════════════════════════════════╣\n");

    // Latency
    out.push_str("║  LATENCY (end-to-end per element)                            ║\n");
    out.push_str(&format!(
        "║    p50:   {:<53}║\n",
        format_ns(report.latency.p50_ns)
    ));
    out.push_str(&format!(
        "║    p95:   {:<53}║\n",
        format_ns(report.latency.p95_ns)
    ));
    out.push_str(&format!(
        "║    p99:   {:<53}║\n",
        format_ns(report.latency.p99_ns)
    ));
    out.push_str(&format!(
        "║    p99.9: {:<53}║\n",
        format_ns(report.latency.p999_ns)
    ));

    // Throughput
    out.push_str("║  THROUGHPUT                                                  ║\n");
    out.push_str(&format!(
        "║    Sustained: {:.0} elements/sec{:<30}║\n",
        report.throughput.sustained_elements_per_sec, ""
    ));

    // Data movement
    out.push_str("║  DATA MOVEMENT                                               ║\n");
    out.push_str(&format!(
        "║    Total copies: {:<45}║\n",
        report.data_movement.total_copies
    ));
    out.push_str(&format!(
        "║    Avg copies/element: {:.1}{:<40}║\n",
        report.data_movement.avg_copies_per_element, ""
    ));

    out.push_str("╚══════════════════════════════════════════════════════════════╝\n");
    out
}

/// Format nanoseconds as a human-readable string.
fn format_ns(ns: u64) -> String {
    if ns < 1_000 {
        format!("{ns} ns")
    } else if ns < 1_000_000 {
        format!("{:.1} μs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.1} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.2} s", ns as f64 / 1_000_000_000.0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::flow_metrics::FlowMetrics;
    use crate::metrics::snapshot::snapshot_flow;
    use torvyn_types::{ComponentId, FlowId, StreamId};

    #[test]
    fn test_generate_report() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(1), ComponentId::new(2)],
            &[StreamId::new(1)],
            0,
        );

        let start = snapshot_flow(&fm);

        // Simulate some activity.
        for _ in 0..1000 {
            fm.elements_total.increment(1);
            fm.end_to_end_latency.record(5000);
            fm.copies_total.increment(1);
            fm.copy_bytes_total.increment(64);
        }
        if let Some(comp) = fm.component(ComponentId::new(1)) {
            for _ in 0..500 {
                comp.invocations.increment(1);
                comp.processing_time.record(3000);
            }
        }
        if let Some(comp) = fm.component(ComponentId::new(2)) {
            for _ in 0..500 {
                comp.invocations.increment(1);
                comp.processing_time.record(2000);
            }
        }

        let end = snapshot_flow(&fm);

        let report = generate_report(
            "source → transform → sink".into(),
            Duration::from_secs(10),
            Duration::from_secs(2),
            100,
            &start,
            &end,
            10000,
        );

        assert_eq!(report.elements_processed, 1000);
        assert!((report.throughput.sustained_elements_per_sec - 100.0).abs() < 0.01);
        assert_eq!(report.data_movement.total_copies, 1000);
        assert_eq!(report.component_breakdown.len(), 2);
    }

    #[test]
    fn test_format_report_does_not_panic() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(1)],
            &[StreamId::new(1)],
            0,
        );
        fm.elements_total.increment(100);
        fm.end_to_end_latency.record(5000);

        let start = snapshot_flow(&fm);
        let end = snapshot_flow(&fm);

        let report = generate_report(
            "test".into(),
            Duration::from_secs(1),
            Duration::from_secs(0),
            0,
            &start,
            &end,
            0,
        );

        let formatted = format_report(&report);
        assert!(formatted.contains("Torvyn Benchmark Report"));
        assert!(formatted.contains("test"));
    }

    #[test]
    fn test_format_ns() {
        assert_eq!(format_ns(500), "500 ns");
        assert_eq!(format_ns(5_000), "5.0 μs");
        assert_eq!(format_ns(5_000_000), "5.0 ms");
        assert_eq!(format_ns(5_000_000_000), "5.00 s");
    }

    #[test]
    fn test_report_serde_roundtrip() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(1)],
            &[StreamId::new(1)],
            0,
        );
        let start = snapshot_flow(&fm);
        let end = snapshot_flow(&fm);

        let report = generate_report(
            "test".into(),
            Duration::from_secs(1),
            Duration::from_secs(0),
            0,
            &start,
            &end,
            0,
        );

        let json = serde_json::to_string(&report).unwrap();
        let parsed: BenchmarkReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pipeline_description, "test");
    }

    #[test]
    fn test_report_empty_flow() {
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
}
