//! Shared record types for the Torvyn runtime.
//!
//! These are plain data structures (no resource handles, no lifecycle)
//! that travel with stream elements or are recorded by the observability system.

use crate::enums::CopyReason;
use crate::{ComponentId, SpanId, TraceId};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Metadata accompanying each stream element.
///
/// Maps to the WIT `element-meta` record in `torvyn:streaming@0.1.0`
/// (Doc 01, Section 3.1). This is a plain data record — cheap to copy.
///
/// Per consolidated review (C01-4): the runtime (reactor's flow driver)
/// assigns `sequence` and `timestamp_ns` before delivery. Component-provided
/// values are advisory and may be overwritten.
///
/// # Examples
/// ```
/// use torvyn_types::ElementMeta;
///
/// let meta = ElementMeta {
///     sequence: 42,
///     timestamp_ns: 1_700_000_000_000_000_000,
///     content_type: "application/json".into(),
/// };
/// assert_eq!(meta.sequence, 42);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ElementMeta {
    /// Monotonic sequence number within this flow.
    /// Starts at 0 and increments by 1 for each element.
    /// The runtime guarantees uniqueness and ordering within a single flow.
    pub sequence: u64,

    /// Wall-clock timestamp in nanoseconds since Unix epoch.
    /// Set by the runtime when the element enters the pipeline.
    pub timestamp_ns: u64,

    /// Content type of the payload (e.g., "application/json").
    /// Mirrors the buffer's content-type for convenience; components
    /// can use this for dispatch without reading the buffer.
    pub content_type: String,
}

impl ElementMeta {
    /// Create a new `ElementMeta` with the given values.
    ///
    /// # HOT PATH — called per stream element.
    #[inline]
    pub fn new(sequence: u64, timestamp_ns: u64, content_type: String) -> Self {
        Self {
            sequence,
            timestamp_ns,
            content_type,
        }
    }
}

impl std::fmt::Display for ElementMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "element[seq={}, ts={}ns, type={}]",
            self.sequence, self.timestamp_ns, self.content_type
        )
    }
}

/// Record of a data transfer for copy accounting and observability.
///
/// Created by the resource manager whenever data crosses a component boundary.
/// Fed to the observability system for reporting.
///
/// # Examples
/// ```
/// use torvyn_types::{TransferRecord, ComponentId, CopyReason};
///
/// let record = TransferRecord {
///     source: ComponentId::new(1),
///     destination: ComponentId::new(2),
///     byte_count: 1024,
///     copy_reason: CopyReason::CrossComponent,
///     timestamp_ns: 1_700_000_000_000_000_000,
/// };
/// assert_eq!(record.byte_count, 1024);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TransferRecord {
    /// The component that produced the data.
    pub source: ComponentId,
    /// The component that consumed the data.
    pub destination: ComponentId,
    /// Number of bytes transferred.
    pub byte_count: u64,
    /// The reason for the copy.
    pub copy_reason: CopyReason,
    /// Timestamp of the transfer in nanoseconds since Unix epoch.
    pub timestamp_ns: u64,
}

impl std::fmt::Display for TransferRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "transfer[{} \u{2192} {}, {}B, reason={}, ts={}ns]",
            self.source, self.destination, self.byte_count,
            self.copy_reason, self.timestamp_ns
        )
    }
}

/// W3C Trace Context carried through a flow for distributed tracing.
///
/// Combines a trace ID and span ID. Propagated through `flow-context`
/// resources in the WIT layer.
///
/// # Examples
/// ```
/// use torvyn_types::{TraceContext, TraceId, SpanId};
///
/// let ctx = TraceContext::new(
///     TraceId::new([1; 16]),
///     SpanId::new([2; 8]),
/// );
/// assert!(ctx.is_valid());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TraceContext {
    /// The W3C trace ID (128-bit).
    pub trace_id: TraceId,
    /// The W3C span ID (64-bit) for the current scope.
    pub span_id: SpanId,
}

impl TraceContext {
    /// Create a new `TraceContext`.
    ///
    /// # COLD PATH — created at flow start or when entering a new span.
    #[inline]
    pub const fn new(trace_id: TraceId, span_id: SpanId) -> Self {
        Self { trace_id, span_id }
    }

    /// Returns `true` if both trace ID and span ID are valid (non-zero).
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.trace_id.is_valid() && self.span_id.is_valid()
    }

    /// Returns an invalid context (tracing disabled).
    #[inline]
    pub const fn disabled() -> Self {
        Self {
            trace_id: TraceId::invalid(),
            span_id: SpanId::invalid(),
        }
    }
}

impl std::fmt::Display for TraceContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "trace[{}:{}]", self.trace_id, self.span_id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_element_meta_new() {
        let meta = ElementMeta::new(0, 100, "text/plain".into());
        assert_eq!(meta.sequence, 0);
        assert_eq!(meta.timestamp_ns, 100);
        assert_eq!(meta.content_type, "text/plain");
    }

    #[test]
    fn test_element_meta_display() {
        let meta = ElementMeta::new(42, 1000, "application/json".into());
        let display = format!("{meta}");
        assert!(display.contains("seq=42"));
        assert!(display.contains("application/json"));
    }

    #[test]
    fn test_transfer_record_display() {
        let record = TransferRecord {
            source: ComponentId::new(1),
            destination: ComponentId::new(2),
            byte_count: 512,
            copy_reason: CopyReason::CrossComponent,
            timestamp_ns: 1000,
        };
        let display = format!("{record}");
        assert!(display.contains("component-1"));
        assert!(display.contains("component-2"));
        assert!(display.contains("512B"));
    }

    #[test]
    fn test_trace_context_valid() {
        let ctx = TraceContext::new(TraceId::new([1; 16]), SpanId::new([2; 8]));
        assert!(ctx.is_valid());
    }

    #[test]
    fn test_trace_context_disabled() {
        let ctx = TraceContext::disabled();
        assert!(!ctx.is_valid());
    }

    #[test]
    fn test_trace_context_display() {
        let ctx = TraceContext::new(TraceId::new([0xab; 16]), SpanId::new([0xcd; 8]));
        let display = format!("{ctx}");
        assert!(display.starts_with("trace["));
        assert!(display.contains("abababab"));
        assert!(display.contains("cdcdcdcd"));
    }
}
