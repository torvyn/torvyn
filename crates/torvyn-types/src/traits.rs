//! Shared traits for the Torvyn runtime.
//!
//! The primary trait here is [`EventSink`], the hot-path interface for
//! recording observability events. All methods must be non-blocking and
//! allocation-free on the hot path.

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

    /// Returns the current observability level.
    ///
    /// Hot-path callers can skip expensive recording at lower levels.
    ///
    /// # HOT PATH — checked per element to skip recording.
    fn level(&self) -> ObservabilityLevel;
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
}
