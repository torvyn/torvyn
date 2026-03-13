//! Stream state and element reference types.
//!
//! A stream is the runtime connection between two components in a pipeline.
//! Each stream owns a [`BoundedQueue`] and tracks backpressure, demand,
//! and metrics.

use std::time::Instant;

use torvyn_types::{
    BackpressurePolicy, BufferHandle, ComponentId, ElementMeta, FlowId, StreamId,
};

use crate::backpressure::BackpressureState;
use crate::metrics::StreamMetrics;
use crate::queue::BoundedQueue;

// ---------------------------------------------------------------------------
// StreamElementRef
// ---------------------------------------------------------------------------

/// Reference to a stream element in the queue.
///
/// Does NOT own the underlying buffer — just references it via `BufferHandle`.
/// Per C04-1: uses `BufferHandle` (not `ResourceHandle`).
///
/// # HOT PATH — created per element, stored in the queue.
///
/// # Invariants
/// - `buffer_handle` refers to a valid buffer in the resource manager.
/// - `meta.sequence` is monotonically increasing within a stream.
#[derive(Clone, Debug)]
pub struct StreamElementRef {
    /// Sequence number within this stream (monotonically increasing).
    pub sequence: u64,
    /// Handle to the payload buffer (managed by Resource Manager).
    /// Per C04-1: `BufferHandle` from `torvyn-types`.
    pub buffer_handle: BufferHandle,
    /// Element metadata (sequence, timestamp, content type).
    pub meta: ElementMeta,
    /// Timestamp when this element entered the queue.
    /// Per DI-12: `Instant::now()` cost is ~20-30ns, within budget.
    pub enqueued_at: Instant,
}

// ---------------------------------------------------------------------------
// StreamState
// ---------------------------------------------------------------------------

/// Runtime state of a stream connecting two components.
///
/// Owned exclusively by the flow driver task — no shared-memory
/// synchronization required.
///
/// # Invariants
/// - `queue.capacity()` equals the configured capacity.
/// - `demand >= 0` at all times.
/// - `producer_complete` is set at most once (monotonic).
pub struct StreamState {
    /// Unique stream identifier.
    pub id: StreamId,
    /// The flow this stream belongs to.
    pub flow_id: FlowId,
    /// Upstream component (producer).
    pub producer: ComponentId,
    /// Downstream component (consumer).
    pub consumer: ComponentId,
    /// Bounded ring buffer holding pending stream elements.
    pub queue: BoundedQueue<StreamElementRef>,
    /// Backpressure policy for this stream.
    pub backpressure_policy: BackpressurePolicy,
    /// Current backpressure state.
    pub backpressure: BackpressureState,
    /// Low watermark ratio (fraction of capacity for backpressure deactivation).
    pub low_watermark_ratio: f64,
    /// Demand credits: how many more elements the consumer is willing to accept.
    pub demand: u64,
    /// Whether the producer has signaled completion (no more elements).
    pub producer_complete: bool,
    /// Whether the consumer has been notified of completion.
    pub consumer_notified_complete: bool,
    /// Next sequence number to assign.
    pub next_sequence: u64,
    /// Cumulative metrics for this stream.
    pub metrics: StreamMetrics,
}

impl StreamState {
    /// Create a new `StreamState` from configuration.
    ///
    /// The initial demand is set to the queue capacity (per Doc 04 §4.2).
    ///
    /// # COLD PATH — called once per stream during flow setup.
    pub fn new(
        id: StreamId,
        flow_id: FlowId,
        producer: ComponentId,
        consumer: ComponentId,
        capacity: usize,
        backpressure_policy: BackpressurePolicy,
        low_watermark_ratio: f64,
    ) -> Self {
        Self {
            id,
            flow_id,
            producer,
            consumer,
            queue: BoundedQueue::new(capacity),
            backpressure_policy,
            backpressure: BackpressureState::Normal,
            low_watermark_ratio,
            demand: capacity as u64, // initial demand grant per Doc 04 §4.2
            producer_complete: false,
            consumer_notified_complete: false,
            next_sequence: 0,
            metrics: StreamMetrics::new(),
        }
    }

    /// Returns `true` if the producer can produce (has demand and queue space).
    ///
    /// # HOT PATH
    #[inline]
    pub fn producer_can_produce(&self) -> bool {
        !self.producer_complete
            && self.demand > 0
            && !self.queue.is_full()
            && !self.backpressure.is_active()
    }

    /// Returns `true` if the consumer has elements to consume.
    ///
    /// # HOT PATH
    #[inline]
    pub fn consumer_has_input(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Returns the low watermark depth (the queue depth at which
    /// backpressure is deactivated).
    ///
    /// # HOT PATH
    #[inline]
    pub fn low_watermark_depth(&self) -> usize {
        (self.queue.capacity() as f64 * self.low_watermark_ratio) as usize
    }

    /// Returns `true` if the stream is complete (producer done + queue drained).
    ///
    /// # WARM PATH
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.producer_complete && self.queue.is_empty()
    }
}

impl std::fmt::Debug for StreamState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamState")
            .field("id", &self.id)
            .field("producer", &self.producer)
            .field("consumer", &self.consumer)
            .field("queue_len", &self.queue.len())
            .field("queue_cap", &self.queue.capacity())
            .field("demand", &self.demand)
            .field("backpressure", &self.backpressure)
            .field("producer_complete", &self.producer_complete)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stream(capacity: usize) -> StreamState {
        StreamState::new(
            StreamId::new(1),
            FlowId::new(1),
            ComponentId::new(10),
            ComponentId::new(20),
            capacity,
            BackpressurePolicy::BlockProducer,
            0.5,
        )
    }

    #[test]
    fn test_stream_state_initial_demand() {
        let s = make_stream(64);
        assert_eq!(s.demand, 64);
    }

    #[test]
    fn test_stream_state_producer_can_produce_initial() {
        let s = make_stream(64);
        assert!(s.producer_can_produce());
    }

    #[test]
    fn test_stream_state_producer_cannot_produce_when_complete() {
        let mut s = make_stream(64);
        s.producer_complete = true;
        assert!(!s.producer_can_produce());
    }

    #[test]
    fn test_stream_state_producer_cannot_produce_when_no_demand() {
        let mut s = make_stream(64);
        s.demand = 0;
        assert!(!s.producer_can_produce());
    }

    #[test]
    fn test_stream_state_consumer_has_input_empty() {
        let s = make_stream(64);
        assert!(!s.consumer_has_input());
    }

    #[test]
    fn test_stream_state_low_watermark_depth() {
        let s = make_stream(64);
        assert_eq!(s.low_watermark_depth(), 32); // 64 * 0.5
    }

    #[test]
    fn test_stream_state_is_complete() {
        let mut s = make_stream(64);
        assert!(!s.is_complete());
        s.producer_complete = true;
        assert!(s.is_complete()); // queue is empty
    }
}
