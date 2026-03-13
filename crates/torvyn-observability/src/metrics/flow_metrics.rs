//! Pre-allocated per-flow, per-component, and per-stream metric structures.
//!
//! All metrics use atomic counters and histograms, enabling lock-free
//! hot-path recording without allocation.
//!
//! Per HLI Doc 05 §3.2: metrics are organized hierarchically and allocated
//! at flow creation time.

use super::{Counter, Gauge, Histogram, LATENCY_BUCKETS_NS};
use torvyn_types::{ComponentId, FlowId, StreamId};

/// Pre-allocated metric storage for a single flow.
///
/// # Invariants
/// - `components` and `streams` are immutable after creation.
/// - All atomic fields are safe for concurrent read/write without locks.
///
/// # Memory
/// - Base: ~2 KB.
/// - Per component: ~512 B.
/// - Per stream: ~512 B.
pub struct FlowMetrics {
    /// Flow identifier.
    pub flow_id: FlowId,
    /// Total elements processed through this flow.
    pub elements_total: Counter,
    /// Total elements that produced errors.
    pub errors_total: Counter,
    /// Total copy operations in the flow.
    pub copies_total: Counter,
    /// Total bytes copied across all operations.
    pub copy_bytes_total: Counter,
    /// End-to-end latency histogram (ns).
    pub end_to_end_latency: Histogram,
    /// Per-component metrics.
    pub components: Vec<ComponentMetrics>,
    /// Per-stream metrics.
    pub streams: Vec<StreamMetrics>,
    /// Wall-clock start time (ns since epoch).
    pub start_time_ns: u64,
}

impl FlowMetrics {
    /// Create new pre-allocated metrics for a flow.
    ///
    /// # COLD PATH — called once per flow registration.
    ///
    /// # Preconditions
    /// - `component_ids` and `stream_ids` must not be empty.
    pub fn new(
        flow_id: FlowId,
        component_ids: &[ComponentId],
        stream_ids: &[StreamId],
        start_time_ns: u64,
    ) -> Self {
        let components = component_ids
            .iter()
            .map(|&id| ComponentMetrics::new(id))
            .collect();
        let streams = stream_ids
            .iter()
            .map(|&id| StreamMetrics::new(id))
            .collect();

        Self {
            flow_id,
            elements_total: Counter::new(),
            errors_total: Counter::new(),
            copies_total: Counter::new(),
            copy_bytes_total: Counter::new(),
            end_to_end_latency: Histogram::new(LATENCY_BUCKETS_NS),
            components,
            streams,
            start_time_ns,
        }
    }

    /// Find component metrics by component ID.
    ///
    /// # HOT PATH — linear scan over a small array (typically < 10 components).
    #[inline]
    pub fn component(&self, id: ComponentId) -> Option<&ComponentMetrics> {
        self.components.iter().find(|c| c.component_id == id)
    }

    /// Find stream metrics by stream ID.
    ///
    /// # HOT PATH — linear scan over a small array.
    #[inline]
    pub fn stream(&self, id: StreamId) -> Option<&StreamMetrics> {
        self.streams.iter().find(|s| s.stream_id == id)
    }
}

impl std::fmt::Debug for FlowMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowMetrics")
            .field("flow_id", &self.flow_id)
            .field("elements_total", &self.elements_total.read())
            .field("errors_total", &self.errors_total.read())
            .field("components", &self.components.len())
            .field("streams", &self.streams.len())
            .finish()
    }
}

/// Pre-allocated metric storage for a single component instance.
///
/// Per HLI Doc 05 §3.3.2.
pub struct ComponentMetrics {
    /// Component instance identifier.
    pub component_id: ComponentId,
    /// Total invocations.
    pub invocations: Counter,
    /// Total errors.
    pub errors: Counter,
    /// Processing time per invocation (ns).
    pub processing_time: Histogram,
    /// Wasm fuel consumed (if metering enabled).
    pub fuel_consumed: Counter,
    /// Current linear memory size (bytes).
    pub memory_current: Gauge,
    /// Peak linear memory high-water mark (bytes).
    pub memory_peak: Gauge,
}

impl ComponentMetrics {
    /// # COLD PATH
    pub fn new(component_id: ComponentId) -> Self {
        Self {
            component_id,
            invocations: Counter::new(),
            errors: Counter::new(),
            processing_time: Histogram::new(LATENCY_BUCKETS_NS),
            fuel_consumed: Counter::new(),
            memory_current: Gauge::new(),
            memory_peak: Gauge::new(),
        }
    }
}

impl std::fmt::Debug for ComponentMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentMetrics")
            .field("component_id", &self.component_id)
            .field("invocations", &self.invocations.read())
            .field("errors", &self.errors.read())
            .finish()
    }
}

/// Pre-allocated metric storage for a single stream connection.
///
/// Per HLI Doc 05 §3.3.3.
pub struct StreamMetrics {
    /// Stream connection identifier.
    pub stream_id: StreamId,
    /// Total elements passed through this stream.
    pub elements: Counter,
    /// Current queue depth.
    pub queue_depth: Gauge,
    /// Peak queue depth high-water mark.
    pub queue_depth_peak: Gauge,
    /// Total backpressure activation events.
    pub backpressure_events: Counter,
    /// Cumulative backpressure duration (ns).
    pub backpressure_duration_ns: Counter,
    /// Queue wait time histogram (ns).
    pub queue_wait_time: Histogram,
}

impl StreamMetrics {
    /// # COLD PATH
    pub fn new(stream_id: StreamId) -> Self {
        Self {
            stream_id,
            elements: Counter::new(),
            queue_depth: Gauge::new(),
            queue_depth_peak: Gauge::new(),
            backpressure_events: Counter::new(),
            backpressure_duration_ns: Counter::new(),
            queue_wait_time: Histogram::new(LATENCY_BUCKETS_NS),
        }
    }
}

impl std::fmt::Debug for StreamMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamMetrics")
            .field("stream_id", &self.stream_id)
            .field("elements", &self.elements.read())
            .field("queue_depth", &self.queue_depth.read())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_metrics_new() {
        let flow_id = FlowId::new(1);
        let components = vec![ComponentId::new(1), ComponentId::new(2)];
        let streams = vec![StreamId::new(1)];
        let fm = FlowMetrics::new(flow_id, &components, &streams, 1000);

        assert_eq!(fm.flow_id, flow_id);
        assert_eq!(fm.elements_total.read(), 0);
        assert_eq!(fm.components.len(), 2);
        assert_eq!(fm.streams.len(), 1);
    }

    #[test]
    fn test_flow_metrics_component_lookup() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(10), ComponentId::new(20)],
            &[StreamId::new(1)],
            0,
        );

        assert!(fm.component(ComponentId::new(10)).is_some());
        assert!(fm.component(ComponentId::new(20)).is_some());
        assert!(fm.component(ComponentId::new(99)).is_none());
    }

    #[test]
    fn test_flow_metrics_stream_lookup() {
        let fm = FlowMetrics::new(
            FlowId::new(1),
            &[ComponentId::new(1)],
            &[StreamId::new(5), StreamId::new(6)],
            0,
        );

        assert!(fm.stream(StreamId::new(5)).is_some());
        assert!(fm.stream(StreamId::new(99)).is_none());
    }

    #[test]
    fn test_component_metrics_record() {
        let cm = ComponentMetrics::new(ComponentId::new(1));
        cm.invocations.increment(1);
        cm.processing_time.record(5000);
        assert_eq!(cm.invocations.read(), 1);
        assert_eq!(cm.processing_time.count(), 1);
    }

    #[test]
    fn test_stream_metrics_queue_depth_peak() {
        let sm = StreamMetrics::new(StreamId::new(1));
        sm.queue_depth.set(10);
        sm.queue_depth_peak.update_max(10);
        sm.queue_depth.set(5);
        sm.queue_depth_peak.update_max(5);
        assert_eq!(sm.queue_depth.read(), 5);
        assert_eq!(sm.queue_depth_peak.read(), 10);
    }
}
