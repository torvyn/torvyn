//! Shared traits for the Torvyn runtime.
//!
//! The primary trait here is [`EventSink`], the hot-path interface for
//! recording observability events. All methods must be non-blocking and
//! allocation-free on the hot path.

use std::sync::Arc;

use crate::{
    enums::{CopyReason, ObservabilityLevel},
    error::ProcessErrorKind,
    ComponentId, FlowId, ResourceId, StreamId,
};

/// The hot-path trait for recording observability events.
///
/// Implemented by the observability collector (`torvyn-observability`) and
/// provided to the reactor, resource manager, and host lifecycle manager.
///
/// Per Doc 05, Section 9.1: all methods must be non-blocking and
/// allocation-free on the hot path.
///
/// Use [`NoopEventSink`] for testing or when observability is disabled.
///
/// # Examples
/// ```
/// use torvyn_types::{NoopEventSink, EventSink, ObservabilityLevel};
///
/// let sink = NoopEventSink;
/// assert_eq!(sink.level(), ObservabilityLevel::Off);
/// ```
pub trait EventSink: Send + Sync + 'static {
    /// Record a component invocation completion.
    ///
    /// Called by the reactor after every component invocation.
    ///
    /// # HOT PATH — must be non-blocking, allocation-free.
    fn record_invocation(
        &self,
        flow_id: FlowId,
        component_id: ComponentId,
        start_ns: u64,
        end_ns: u64,
        status: InvocationStatus,
    );

    /// Record a stream element transfer between components.
    ///
    /// Called by the reactor when an element moves through a stream queue.
    ///
    /// # HOT PATH — must be non-blocking, allocation-free.
    fn record_element_transfer(
        &self,
        flow_id: FlowId,
        stream_id: StreamId,
        element_sequence: u64,
        queue_depth_after: u32,
    );

    /// Record a backpressure state change.
    ///
    /// Called by the reactor when backpressure activates or deactivates.
    ///
    /// # WARM PATH — called per backpressure event.
    fn record_backpressure(
        &self,
        flow_id: FlowId,
        stream_id: StreamId,
        activated: bool,
        queue_depth: u32,
        timestamp_ns: u64,
    );

    /// Record a resource copy operation.
    ///
    /// Called by the resource manager when data is copied across a boundary.
    ///
    /// # HOT PATH — must be non-blocking, allocation-free.
    fn record_copy(
        &self,
        flow_id: FlowId,
        resource_id: ResourceId,
        from_component: ComponentId,
        to_component: ComponentId,
        copy_bytes: u64,
        reason: CopyReason,
    );

    /// Record an element's end-to-end latency through the flow.
    ///
    /// Called by the reactor when a sink consumes an element, measured from the
    /// element's pipeline-entry timestamp (set at the source) to its
    /// consumption at the sink. Unlike per-component processing time, this is
    /// the full journey — queueing plus every stage. The default is a no-op.
    ///
    /// # HOT PATH — called once per element delivered to a sink.
    fn record_flow_latency(&self, flow_id: FlowId, latency_ns: u64) {
        let _ = (flow_id, latency_ns);
    }

    /// Returns the current observability level.
    ///
    /// Hot-path callers can skip expensive recording at lower levels.
    ///
    /// # HOT PATH — checked per element to skip recording.
    fn level(&self) -> ObservabilityLevel;

    /// Pre-register a flow's per-flow metric state before its driver starts.
    ///
    /// Called by the reactor exactly once per flow — after the flow id is
    /// assigned and before the flow driver is spawned — so that no later
    /// `record_*` call can observe a flow whose metrics have not yet been
    /// allocated. `component_ids` and `stream_ids` enumerate the flow's
    /// stages and stream connections in the same order the reactor records
    /// them, letting the sink allocate matching metric slots up front.
    ///
    /// The default is a no-op: sinks that hold no per-flow state (such as
    /// [`NoopEventSink`]) need not implement it.
    ///
    /// # COLD PATH — called once per flow, off the element hot path.
    fn on_flow_start(
        &self,
        flow_id: FlowId,
        component_ids: &[ComponentId],
        stream_ids: &[StreamId],
    ) {
        let _ = (flow_id, component_ids, stream_ids);
    }
}

/// Forwarding implementation so any `Arc`-shared sink satisfies the reactor's
/// `E: EventSink + Clone + 'static` bound without the sink itself being
/// `Clone`: the `Arc` provides the cheap clone (one shared sink, reference
/// counted), and every call forwards to the single instance behind it.
///
/// This lets the host install a shared [`ObservabilityCollector`] (which owns
/// `Arc`-backed registries and a background event recorder, and so is not
/// `Clone`) by wrapping it in an `Arc`.
///
/// [`ObservabilityCollector`]: https://docs.rs/torvyn-observability
impl<E: EventSink> EventSink for Arc<E> {
    #[inline]
    fn record_invocation(
        &self,
        flow_id: FlowId,
        component_id: ComponentId,
        start_ns: u64,
        end_ns: u64,
        status: InvocationStatus,
    ) {
        (**self).record_invocation(flow_id, component_id, start_ns, end_ns, status);
    }

    #[inline]
    fn record_element_transfer(
        &self,
        flow_id: FlowId,
        stream_id: StreamId,
        element_sequence: u64,
        queue_depth_after: u32,
    ) {
        (**self).record_element_transfer(flow_id, stream_id, element_sequence, queue_depth_after);
    }

    #[inline]
    fn record_backpressure(
        &self,
        flow_id: FlowId,
        stream_id: StreamId,
        activated: bool,
        queue_depth: u32,
        timestamp_ns: u64,
    ) {
        (**self).record_backpressure(flow_id, stream_id, activated, queue_depth, timestamp_ns);
    }

    #[inline]
    fn record_copy(
        &self,
        flow_id: FlowId,
        resource_id: ResourceId,
        from_component: ComponentId,
        to_component: ComponentId,
        copy_bytes: u64,
        reason: CopyReason,
    ) {
        (**self).record_copy(
            flow_id,
            resource_id,
            from_component,
            to_component,
            copy_bytes,
            reason,
        );
    }

    #[inline]
    fn record_flow_latency(&self, flow_id: FlowId, latency_ns: u64) {
        (**self).record_flow_latency(flow_id, latency_ns);
    }

    #[inline]
    fn level(&self) -> ObservabilityLevel {
        (**self).level()
    }

    #[inline]
    fn on_flow_start(
        &self,
        flow_id: FlowId,
        component_ids: &[ComponentId],
        stream_ids: &[StreamId],
    ) {
        (**self).on_flow_start(flow_id, component_ids, stream_ids);
    }
}

/// Status of a component invocation, for observability recording.
///
/// # HOT PATH — created per invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvocationStatus {
    /// Invocation completed successfully.
    Ok,
    /// Invocation completed with an error.
    Error(ProcessErrorKind),
    /// Invocation timed out.
    Timeout,
    /// Invocation was cancelled.
    Cancelled,
}

impl std::fmt::Display for InvocationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvocationStatus::Ok => write!(f, "ok"),
            InvocationStatus::Error(kind) => write!(f, "error({:?})", kind),
            InvocationStatus::Timeout => write!(f, "timeout"),
            InvocationStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// A no-op implementation of [`EventSink`] for testing and benchmarking.
///
/// All methods are empty. Returns [`ObservabilityLevel::Off`].
///
/// # Examples
/// ```
/// use torvyn_types::{NoopEventSink, EventSink, FlowId, ComponentId, ObservabilityLevel};
/// use torvyn_types::InvocationStatus;
///
/// let sink = NoopEventSink;
/// sink.record_invocation(FlowId::new(1), ComponentId::new(1), 0, 100, InvocationStatus::Ok);
/// assert_eq!(sink.level(), ObservabilityLevel::Off);
/// ```
#[derive(Clone, Copy)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    #[inline]
    fn record_invocation(
        &self,
        _flow_id: FlowId,
        _component_id: ComponentId,
        _start_ns: u64,
        _end_ns: u64,
        _status: InvocationStatus,
    ) {
        // No-op: zero cost when observability is off.
    }

    #[inline]
    fn record_element_transfer(
        &self,
        _flow_id: FlowId,
        _stream_id: StreamId,
        _element_sequence: u64,
        _queue_depth_after: u32,
    ) {
        // No-op.
    }

    #[inline]
    fn record_backpressure(
        &self,
        _flow_id: FlowId,
        _stream_id: StreamId,
        _activated: bool,
        _queue_depth: u32,
        _timestamp_ns: u64,
    ) {
        // No-op.
    }

    #[inline]
    fn record_copy(
        &self,
        _flow_id: FlowId,
        _resource_id: ResourceId,
        _from_component: ComponentId,
        _to_component: ComponentId,
        _copy_bytes: u64,
        _reason: CopyReason,
    ) {
        // No-op.
    }

    #[inline]
    fn level(&self) -> ObservabilityLevel {
        ObservabilityLevel::Off
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_event_sink_level() {
        let sink = NoopEventSink;
        assert_eq!(sink.level(), ObservabilityLevel::Off);
    }

    #[test]
    fn test_noop_event_sink_record_invocation_does_not_panic() {
        let sink = NoopEventSink;
        sink.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );
    }

    #[test]
    fn test_noop_event_sink_record_copy_does_not_panic() {
        let sink = NoopEventSink;
        sink.record_copy(
            FlowId::new(1),
            ResourceId::new(0, 0),
            ComponentId::new(1),
            ComponentId::new(2),
            1024,
            CopyReason::CrossComponent,
        );
    }

    #[test]
    fn test_invocation_status_display() {
        assert_eq!(format!("{}", InvocationStatus::Ok), "ok");
        assert_eq!(format!("{}", InvocationStatus::Timeout), "timeout");
        assert_eq!(format!("{}", InvocationStatus::Cancelled), "cancelled");
    }

    #[test]
    fn test_noop_event_sink_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<NoopEventSink>();
    }

    #[test]
    fn test_noop_event_sink_on_flow_start_default_does_not_panic() {
        let sink = NoopEventSink;
        sink.on_flow_start(
            FlowId::new(1),
            &[ComponentId::new(1), ComponentId::new(2)],
            &[StreamId::new(0)],
        );
    }

    /// A recording sink that counts invocations and the components/streams it
    /// was asked to register, used to prove the `Arc<E>` forwarding impl reaches
    /// the inner sink.
    #[derive(Default)]
    struct CountingSink {
        invocations: std::sync::atomic::AtomicU64,
        registered_components: std::sync::atomic::AtomicU64,
        registered_streams: std::sync::atomic::AtomicU64,
    }

    impl EventSink for CountingSink {
        fn record_invocation(
            &self,
            _flow_id: FlowId,
            _component_id: ComponentId,
            _start_ns: u64,
            _end_ns: u64,
            _status: InvocationStatus,
        ) {
            self.invocations
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        fn record_element_transfer(
            &self,
            _flow_id: FlowId,
            _stream_id: StreamId,
            _element_sequence: u64,
            _queue_depth_after: u32,
        ) {
        }

        fn record_backpressure(
            &self,
            _flow_id: FlowId,
            _stream_id: StreamId,
            _activated: bool,
            _queue_depth: u32,
            _timestamp_ns: u64,
        ) {
        }

        fn record_copy(
            &self,
            _flow_id: FlowId,
            _resource_id: ResourceId,
            _from_component: ComponentId,
            _to_component: ComponentId,
            _copy_bytes: u64,
            _reason: CopyReason,
        ) {
        }

        fn level(&self) -> ObservabilityLevel {
            ObservabilityLevel::Production
        }

        fn on_flow_start(
            &self,
            _flow_id: FlowId,
            component_ids: &[ComponentId],
            stream_ids: &[StreamId],
        ) {
            self.registered_components.fetch_add(
                component_ids.len() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            self.registered_streams.fetch_add(
                stream_ids.len() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    #[test]
    fn test_arc_event_sink_forwards_to_inner() {
        use std::sync::atomic::Ordering;

        let sink: Arc<CountingSink> = Arc::new(CountingSink::default());

        // Every method must forward through the `Arc` to the inner sink.
        EventSink::on_flow_start(
            &sink,
            FlowId::new(7),
            &[
                ComponentId::new(1),
                ComponentId::new(2),
                ComponentId::new(3),
            ],
            &[StreamId::new(0), StreamId::new(1)],
        );
        sink.record_invocation(
            FlowId::new(7),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );
        sink.record_invocation(
            FlowId::new(7),
            ComponentId::new(2),
            100,
            250,
            InvocationStatus::Ok,
        );

        assert_eq!(sink.invocations.load(Ordering::Relaxed), 2);
        assert_eq!(sink.registered_components.load(Ordering::Relaxed), 3);
        assert_eq!(sink.registered_streams.load(Ordering::Relaxed), 2);
        assert_eq!(EventSink::level(&sink), ObservabilityLevel::Production);
    }

    #[test]
    fn test_arc_event_sink_clone_shares_state() {
        use std::sync::atomic::Ordering;

        // The reactor clones the sink once per flow driver; an `Arc` clone must
        // share the single underlying sink so all drivers record into one place.
        let sink: Arc<CountingSink> = Arc::new(CountingSink::default());
        let clone = Arc::clone(&sink);

        clone.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            1,
            InvocationStatus::Ok,
        );
        sink.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            1,
            InvocationStatus::Ok,
        );

        assert_eq!(sink.invocations.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_arc_event_sink_satisfies_clone_bound() {
        // The coordinator requires `E: EventSink + Clone + 'static`; assert the
        // `Arc<E>` wrapper meets exactly that bound.
        fn assert_event_sink_clone<T: EventSink + Clone + 'static>() {}
        assert_event_sink_clone::<Arc<CountingSink>>();
    }
}
