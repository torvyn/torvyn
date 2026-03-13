//! Extended trace context for observability-internal use.
//!
//! `torvyn-types::TraceContext` is the compact cross-crate type.
//! This module defines `FlowTraceContext`, which adds flow-level metadata
//! (parent span, flags, sampling state) used within the observability crate.

use torvyn_types::{FlowId, SpanId, TraceContext, TraceId};

/// Trace flags for Torvyn-specific control.
///
/// Compatible with W3C trace flags (bit 0 = sampled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceFlags(u8);

impl TraceFlags {
    /// No flags set.
    pub const NONE: Self = Self(0x00);
    /// W3C sampled bit.
    pub const SAMPLED: Self = Self(0x01);
    /// Torvyn-specific: full diagnostic mode.
    pub const DIAGNOSTIC: Self = Self(0x02);

    /// Create from raw bits.
    #[inline]
    pub const fn new(bits: u8) -> Self {
        Self(bits)
    }

    /// Whether the sampled flag is set.
    #[inline]
    pub const fn is_sampled(self) -> bool {
        self.0 & Self::SAMPLED.0 != 0
    }

    /// Whether the diagnostic flag is set.
    #[inline]
    pub const fn is_diagnostic(self) -> bool {
        self.0 & Self::DIAGNOSTIC.0 != 0
    }

    /// Return a copy with the sampled flag set.
    #[inline]
    pub const fn with_sampled(self) -> Self {
        Self(self.0 | Self::SAMPLED.0)
    }

    /// Return a copy with the diagnostic flag set.
    #[inline]
    pub const fn with_diagnostic(self) -> Self {
        Self(self.0 | Self::DIAGNOSTIC.0)
    }

    /// Raw bit value.
    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// Flow-level trace context with full metadata.
///
/// This is the observability-internal extended context that wraps
/// `TraceContext` with flow-level information.
///
/// # Invariants
/// - `trace_ctx.trace_id` matches the flow's root trace ID.
/// - `parent_span_id` is the span of the upstream component (or root span).
#[derive(Clone, Copy, Debug)]
pub struct FlowTraceContext {
    /// Core trace context (trace_id + current span_id).
    pub trace_ctx: TraceContext,
    /// The span ID of the parent span.
    pub parent_span_id: SpanId,
    /// Flow ID for Torvyn-internal correlation.
    pub flow_id: FlowId,
    /// Trace flags.
    pub flags: TraceFlags,
}

impl FlowTraceContext {
    /// Create a new flow trace context.
    ///
    /// # COLD PATH — created at flow start.
    pub fn new(trace_id: TraceId, root_span_id: SpanId, flow_id: FlowId) -> Self {
        Self {
            trace_ctx: TraceContext::new(trace_id, root_span_id),
            parent_span_id: SpanId::invalid(),
            flow_id,
            flags: TraceFlags::NONE,
        }
    }

    /// Create a child context for a component invocation.
    ///
    /// # HOT PATH — called per invocation.
    #[inline]
    pub fn child(&self, new_span_id: SpanId) -> Self {
        Self {
            trace_ctx: TraceContext::new(self.trace_ctx.trace_id, new_span_id),
            parent_span_id: self.trace_ctx.span_id,
            flow_id: self.flow_id,
            flags: self.flags,
        }
    }

    /// Mark this context as sampled.
    #[inline]
    pub fn set_sampled(&mut self) {
        self.flags = self.flags.with_sampled();
    }

    /// Mark this context as diagnostic.
    #[inline]
    pub fn set_diagnostic(&mut self) {
        self.flags = self.flags.with_diagnostic();
    }
}

/// Generate a random trace ID.
///
/// # COLD PATH — called once per flow.
pub fn generate_trace_id() -> TraceId {
    // Use a simple XorShift for non-cryptographic randomness.
    // In production, this could use thread_rng or a faster PRNG.
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let mut state = seed;
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let b = state.to_le_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
    }
    TraceId::new(bytes)
}

/// Generate a random span ID.
///
/// # HOT PATH when sampled — called per component invocation.
pub fn generate_span_id() -> SpanId {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let mut state = seed;
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    SpanId::new(state.to_le_bytes())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_flags_sampled() {
        let flags = TraceFlags::NONE.with_sampled();
        assert!(flags.is_sampled());
        assert!(!flags.is_diagnostic());
    }

    #[test]
    fn test_trace_flags_diagnostic() {
        let flags = TraceFlags::NONE.with_diagnostic();
        assert!(!flags.is_sampled());
        assert!(flags.is_diagnostic());
    }

    #[test]
    fn test_trace_flags_both() {
        let flags = TraceFlags::NONE.with_sampled().with_diagnostic();
        assert!(flags.is_sampled());
        assert!(flags.is_diagnostic());
    }

    #[test]
    fn test_flow_trace_context_child() {
        let parent_span = SpanId::new([1; 8]);
        let ctx = FlowTraceContext::new(TraceId::new([1; 16]), parent_span, FlowId::new(1));

        let child_span = SpanId::new([2; 8]);
        let child = ctx.child(child_span);

        assert_eq!(child.trace_ctx.trace_id, ctx.trace_ctx.trace_id);
        assert_eq!(child.trace_ctx.span_id, child_span);
        assert_eq!(child.parent_span_id, parent_span);
        assert_eq!(child.flow_id, ctx.flow_id);
    }

    #[test]
    fn test_generate_trace_id_is_valid() {
        let id = generate_trace_id();
        assert!(id.is_valid());
    }

    #[test]
    fn test_generate_span_id_is_valid() {
        let id = generate_span_id();
        assert!(id.is_valid());
    }

    #[test]
    fn test_generate_trace_id_unique() {
        let a = generate_trace_id();
        // Add a tiny sleep to change the seed.
        std::thread::sleep(std::time::Duration::from_nanos(1));
        let b = generate_trace_id();
        // Not guaranteed unique with time-based PRNG, but overwhelmingly likely.
        // This test is probabilistic.
        assert_ne!(a, b);
    }
}
