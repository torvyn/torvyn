//! Point-in-time metric snapshots for export and comparison.

use serde::{Deserialize, Serialize};
use torvyn_types::{ComponentId, FlowId, StreamId};

use super::flow_metrics::FlowMetrics;

/// Snapshot of all flow-level metrics at a point in time.
///
/// All values are plain integers read from atomics. This struct is `Clone`,
/// `Serialize`, and safe to pass across async boundaries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowMetricsSnapshot {
    /// Flow identifier.
    pub flow_id: FlowId,
    /// Total elements processed.
    pub elements_total: u64,
    /// Total errors.
    pub errors_total: u64,
    /// Total copy operations.
    pub copies_total: u64,
    /// Total bytes copied.
    pub copy_bytes_total: u64,
    /// Latency p50 in nanoseconds.
    pub latency_p50_ns: u64,
    /// Latency p95 in nanoseconds.
    pub latency_p95_ns: u64,
    /// Latency p99 in nanoseconds.
    pub latency_p99_ns: u64,
    /// Latency p99.9 in nanoseconds.
    pub latency_p999_ns: u64,
    /// Minimum latency in nanoseconds.
    pub latency_min_ns: u64,
    /// Maximum latency in nanoseconds.
    pub latency_max_ns: u64,
    /// Mean latency in nanoseconds.
    pub latency_mean_ns: f64,
    /// Per-component snapshots.
    pub components: Vec<ComponentMetricsSnapshot>,
    /// Per-stream snapshots.
    pub streams: Vec<StreamMetricsSnapshot>,
}

/// Snapshot of per-component metrics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentMetricsSnapshot {
    /// Component instance identifier.
    pub component_id: ComponentId,
    /// Total invocations.
    pub invocations: u64,
    /// Total errors.
    pub errors: u64,
    /// Processing time p50 in nanoseconds.
    pub processing_time_p50_ns: u64,
    /// Processing time p95 in nanoseconds.
    pub processing_time_p95_ns: u64,
    /// Processing time p99 in nanoseconds.
    pub processing_time_p99_ns: u64,
    /// Mean processing time in nanoseconds.
    pub processing_time_mean_ns: f64,
    /// Total fuel consumed.
    pub fuel_consumed: u64,
    /// Current memory usage in bytes.
    pub memory_current: u64,
    /// Peak memory usage in bytes.
    pub memory_peak: u64,
}

/// Snapshot of per-stream metrics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamMetricsSnapshot {
    /// Stream connection identifier.
    pub stream_id: StreamId,
    /// Total elements transferred.
    pub elements: u64,
    /// Current queue depth.
    pub queue_depth: u64,
    /// Peak queue depth.
    pub queue_depth_peak: u64,
    /// Total backpressure activation events.
    pub backpressure_events: u64,
    /// Cumulative backpressure duration in nanoseconds.
    pub backpressure_duration_ns: u64,
}

/// Take a point-in-time snapshot from live flow metrics.
///
/// # COLD PATH — reads all atomic values and computes percentiles.
pub fn snapshot_flow(metrics: &FlowMetrics) -> FlowMetricsSnapshot {
    let latency_snap = metrics.end_to_end_latency.snapshot();

    let components = metrics
        .components
        .iter()
        .map(|c| {
            let pt_snap = c.processing_time.snapshot();
            ComponentMetricsSnapshot {
                component_id: c.component_id,
                invocations: c.invocations.read(),
                errors: c.errors.read(),
                processing_time_p50_ns: pt_snap.percentile(50.0),
                processing_time_p95_ns: pt_snap.percentile(95.0),
                processing_time_p99_ns: pt_snap.percentile(99.0),
                processing_time_mean_ns: pt_snap.mean(),
                fuel_consumed: c.fuel_consumed.read(),
                memory_current: c.memory_current.read(),
                memory_peak: c.memory_peak.read(),
            }
        })
        .collect();

    let streams = metrics
        .streams
        .iter()
        .map(|s| StreamMetricsSnapshot {
            stream_id: s.stream_id,
            elements: s.elements.read(),
            queue_depth: s.queue_depth.read(),
            queue_depth_peak: s.queue_depth_peak.read(),
            backpressure_events: s.backpressure_events.read(),
            backpressure_duration_ns: s.backpressure_duration_ns.read(),
        })
        .collect();

    FlowMetricsSnapshot {
        flow_id: metrics.flow_id,
        elements_total: metrics.elements_total.read(),
        errors_total: metrics.errors_total.read(),
        copies_total: metrics.copies_total.read(),
        copy_bytes_total: metrics.copy_bytes_total.read(),
        latency_p50_ns: latency_snap.percentile(50.0),
        latency_p95_ns: latency_snap.percentile(95.0),
        latency_p99_ns: latency_snap.percentile(99.0),
        latency_p999_ns: latency_snap.percentile(99.9),
        latency_min_ns: latency_snap.min,
        latency_max_ns: latency_snap.max,
        latency_mean_ns: latency_snap.mean(),
        components,
        streams,
    }
}

/// Compute the delta between two snapshots (end - start).
///
/// # COLD PATH — used by benchmark harness.
pub fn delta(start: &FlowMetricsSnapshot, end: &FlowMetricsSnapshot) -> FlowMetricsSnapshot {
    FlowMetricsSnapshot {
        flow_id: end.flow_id,
        elements_total: end.elements_total.saturating_sub(start.elements_total),
        errors_total: end.errors_total.saturating_sub(start.errors_total),
        copies_total: end.copies_total.saturating_sub(start.copies_total),
        copy_bytes_total: end.copy_bytes_total.saturating_sub(start.copy_bytes_total),
        // Percentiles are from the end snapshot (not delta-able).
        latency_p50_ns: end.latency_p50_ns,
        latency_p95_ns: end.latency_p95_ns,
        latency_p99_ns: end.latency_p99_ns,
        latency_p999_ns: end.latency_p999_ns,
        latency_min_ns: end.latency_min_ns,
        latency_max_ns: end.latency_max_ns,
        latency_mean_ns: end.latency_mean_ns,
        components: end.components.clone(),
        streams: end.streams.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::flow_metrics::FlowMetrics;

    #[test]
    fn test_snapshot_empty_flow() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(1)],
            &[StreamId::new(1)],
            0,
        );
        let snap = snapshot_flow(&fm);
        assert_eq!(snap.elements_total, 0);
        assert_eq!(snap.errors_total, 0);
        assert_eq!(snap.components.len(), 1);
        assert_eq!(snap.streams.len(), 1);
    }

    #[test]
    fn test_snapshot_with_data() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(1)],
            &[StreamId::new(1)],
            0,
        );
        fm.elements_total.increment(100);
        fm.errors_total.increment(3);
        fm.end_to_end_latency.record(5000);

        let snap = snapshot_flow(&fm);
        assert_eq!(snap.elements_total, 100);
        assert_eq!(snap.errors_total, 3);
    }

    #[test]
    fn test_delta_snapshot() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(1)],
            &[StreamId::new(1)],
            0,
        );
        fm.elements_total.increment(10);
        let start = snapshot_flow(&fm);

        fm.elements_total.increment(90);
        let end = snapshot_flow(&fm);

        let d = delta(&start, &end);
        assert_eq!(d.elements_total, 90);
    }
}
