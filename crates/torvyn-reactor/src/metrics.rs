//! Metrics types for streams, flows, and components.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use torvyn_types::{ComponentId, StreamId};

// ---------------------------------------------------------------------------
// StreamMetrics
// ---------------------------------------------------------------------------

/// Cumulative metrics for a single stream.
///
/// Per Doc 04 §2.2. Updated by the flow driver on every element.
///
/// # HOT PATH — updated per element.
#[derive(Clone, Debug, Default)]
pub struct StreamMetrics {
    /// Total elements that have passed through this stream.
    pub elements_total: u64,
    /// Total backpressure events on this stream.
    pub backpressure_events: u64,
    /// Cumulative time spent in backpressure state.
    pub backpressure_duration: Duration,
    /// Maximum observed queue depth.
    pub peak_queue_depth: u32,
    /// Timestamp of last element processed.
    pub last_activity: Option<Instant>,
}

impl StreamMetrics {
    /// Create a new zeroed `StreamMetrics`.
    ///
    /// # COLD PATH
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that an element was enqueued.
    ///
    /// # HOT PATH — zero-alloc.
    #[inline(always)]
    pub fn record_enqueue(&mut self, queue_depth: u32) {
        self.elements_total += 1;
        if queue_depth > self.peak_queue_depth {
            self.peak_queue_depth = queue_depth;
        }
        self.last_activity = Some(Instant::now());
    }

    /// Record a backpressure event.
    ///
    /// # WARM PATH
    #[inline]
    pub fn record_backpressure_event(&mut self) {
        self.backpressure_events += 1;
    }

    /// Add backpressure duration.
    ///
    /// # WARM PATH
    #[inline]
    pub fn add_backpressure_duration(&mut self, duration: Duration) {
        self.backpressure_duration += duration;
    }
}

// ---------------------------------------------------------------------------
// ComponentMetrics
// ---------------------------------------------------------------------------

/// Per-component metrics within a flow.
#[derive(Clone, Debug, Default)]
pub struct ComponentMetrics {
    /// Total invocations of this component.
    pub invocations: u64,
    /// Total errors from this component.
    pub errors: u64,
    /// Cumulative invocation time.
    pub total_invocation_time: Duration,
}

impl ComponentMetrics {
    /// Create a new zeroed `ComponentMetrics`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a completed invocation.
    ///
    /// # HOT PATH — zero-alloc.
    #[inline(always)]
    pub fn record_invocation(&mut self, duration: Duration, is_error: bool) {
        self.invocations += 1;
        self.total_invocation_time += duration;
        if is_error {
            self.errors += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// FlowCompletionStats
// ---------------------------------------------------------------------------

/// Summary statistics emitted when a flow completes, cancels, or fails.
///
/// Per Doc 04 §10.4.
#[derive(Clone, Debug)]
pub struct FlowCompletionStats {
    /// Wall-clock duration of the flow's Running + Draining phases.
    pub total_duration: Duration,
    /// Total elements processed across all streams.
    pub total_elements: u64,
    /// Total backpressure events across all streams.
    pub total_backpressure_events: u64,
    /// Cumulative backpressure duration across all streams.
    pub total_backpressure_duration: Duration,
    /// Per-stream final metrics.
    pub stream_stats: HashMap<StreamId, StreamMetrics>,
    /// Per-component final metrics.
    pub component_stats: HashMap<ComponentId, ComponentMetrics>,
}

impl FlowCompletionStats {
    /// Create a new `FlowCompletionStats` with the given duration.
    ///
    /// # COLD PATH
    pub fn new(total_duration: Duration) -> Self {
        Self {
            total_duration,
            total_elements: 0,
            total_backpressure_events: 0,
            total_backpressure_duration: Duration::ZERO,
            stream_stats: HashMap::new(),
            component_stats: HashMap::new(),
        }
    }

    /// Aggregate stream metrics into the totals.
    ///
    /// # COLD PATH — called once during flow cleanup.
    pub fn aggregate_from_streams(&mut self, streams: &[(StreamId, StreamMetrics)]) {
        for (id, metrics) in streams {
            self.total_elements += metrics.elements_total;
            self.total_backpressure_events += metrics.backpressure_events;
            self.total_backpressure_duration += metrics.backpressure_duration;
            self.stream_stats.insert(*id, metrics.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// ReactorMetrics
// ---------------------------------------------------------------------------

/// Aggregate metrics for the entire reactor.
///
/// Provides a snapshot of reactor-wide statistics including active flow
/// counts, total elements processed, scheduling overhead, and fairness.
#[derive(Clone, Debug, Default)]
pub struct ReactorMetrics {
    /// Number of currently active (non-terminal) flows.
    pub active_flows: u64,
    /// Total number of flows created since reactor start.
    pub total_flows_created: u64,
    /// Total number of flows that reached a terminal state.
    pub total_flows_completed: u64,
    /// Total elements processed across all flows and streams.
    pub total_elements_processed: u64,
    /// Cumulative scheduling overhead (time spent in the scheduler, not in components).
    pub scheduling_overhead: Duration,
    /// Total yields across all flows.
    pub total_yields: u64,
    /// Total backpressure events across all flows.
    pub total_backpressure_events: u64,
    /// Cumulative backpressure duration across all flows.
    pub total_backpressure_duration: Duration,
}

impl ReactorMetrics {
    /// Create a new zeroed `ReactorMetrics`.
    ///
    /// # COLD PATH
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a new flow was created.
    #[inline]
    pub fn record_flow_created(&mut self) {
        self.total_flows_created += 1;
        self.active_flows += 1;
    }

    /// Record that a flow reached a terminal state.
    #[inline]
    pub fn record_flow_completed(&mut self, stats: &FlowCompletionStats) {
        self.active_flows = self.active_flows.saturating_sub(1);
        self.total_flows_completed += 1;
        self.total_elements_processed += stats.total_elements;
        self.total_backpressure_events += stats.total_backpressure_events;
        self.total_backpressure_duration += stats.total_backpressure_duration;
    }

    /// Record scheduling overhead for a single scheduling decision.
    #[inline]
    pub fn record_scheduling_overhead(&mut self, overhead: Duration) {
        self.scheduling_overhead += overhead;
    }

    /// Record a yield event.
    #[inline]
    pub fn record_yield(&mut self) {
        self.total_yields += 1;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_metrics_record_enqueue() {
        let mut m = StreamMetrics::new();
        m.record_enqueue(5);
        assert_eq!(m.elements_total, 1);
        assert_eq!(m.peak_queue_depth, 5);
        m.record_enqueue(3);
        assert_eq!(m.elements_total, 2);
        assert_eq!(m.peak_queue_depth, 5); // still 5
        m.record_enqueue(10);
        assert_eq!(m.peak_queue_depth, 10);
    }

    #[test]
    fn test_stream_metrics_backpressure() {
        let mut m = StreamMetrics::new();
        m.record_backpressure_event();
        m.add_backpressure_duration(Duration::from_millis(50));
        assert_eq!(m.backpressure_events, 1);
        assert_eq!(m.backpressure_duration, Duration::from_millis(50));
    }

    #[test]
    fn test_component_metrics_record() {
        let mut m = ComponentMetrics::new();
        m.record_invocation(Duration::from_micros(100), false);
        m.record_invocation(Duration::from_micros(200), true);
        assert_eq!(m.invocations, 2);
        assert_eq!(m.errors, 1);
        assert_eq!(m.total_invocation_time, Duration::from_micros(300));
    }

    #[test]
    fn test_flow_completion_stats_aggregate() {
        let mut stats = FlowCompletionStats::new(Duration::from_secs(5));
        let stream_data = vec![
            (StreamId::new(0), {
                let mut m = StreamMetrics::new();
                m.elements_total = 100;
                m.backpressure_events = 2;
                m
            }),
            (StreamId::new(1), {
                let mut m = StreamMetrics::new();
                m.elements_total = 100;
                m.backpressure_events = 1;
                m
            }),
        ];
        stats.aggregate_from_streams(&stream_data);
        assert_eq!(stats.total_elements, 200);
        assert_eq!(stats.total_backpressure_events, 3);
        assert_eq!(stats.stream_stats.len(), 2);
    }

    #[test]
    fn test_reactor_metrics_initial() {
        let m = ReactorMetrics::new();
        assert_eq!(m.active_flows, 0);
        assert_eq!(m.total_flows_created, 0);
        assert_eq!(m.total_elements_processed, 0);
    }

    #[test]
    fn test_reactor_metrics_flow_lifecycle() {
        let mut m = ReactorMetrics::new();
        m.record_flow_created();
        m.record_flow_created();
        assert_eq!(m.active_flows, 2);
        assert_eq!(m.total_flows_created, 2);

        let mut stats = FlowCompletionStats::new(Duration::from_secs(1));
        stats.total_elements = 50;
        stats.total_backpressure_events = 3;
        m.record_flow_completed(&stats);

        assert_eq!(m.active_flows, 1);
        assert_eq!(m.total_flows_completed, 1);
        assert_eq!(m.total_elements_processed, 50);
        assert_eq!(m.total_backpressure_events, 3);
    }

    #[test]
    fn test_reactor_metrics_scheduling_overhead() {
        let mut m = ReactorMetrics::new();
        m.record_scheduling_overhead(Duration::from_micros(10));
        m.record_scheduling_overhead(Duration::from_micros(20));
        assert_eq!(m.scheduling_overhead, Duration::from_micros(30));
    }

    #[test]
    fn test_reactor_metrics_yield_tracking() {
        let mut m = ReactorMetrics::new();
        m.record_yield();
        m.record_yield();
        assert_eq!(m.total_yields, 2);
    }
}
