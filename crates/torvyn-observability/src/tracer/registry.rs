//! Per-flow trace state: context plus the flow's span ring buffer.
//!
//! The metrics side of observability keeps aggregates, so a flow's counters
//! and histograms can live behind lock-free atomics. Spans cannot: a span is
//! an individual event, it has to be *retained*, and the ring buffer that
//! retains it is a single-producer structure that needs `&mut` to push.
//!
//! This registry is the span-side counterpart to
//! [`MetricsRegistry`](crate::metrics::registry::MetricsRegistry) and mirrors
//! its shape deliberately: an `RwLock<HashMap<..>>` for the cold
//! register/deregister path, and `Arc` handles so the hot path takes a read
//! lock and a hash lookup rather than holding the registry lock while it
//! writes.
//!
//! # Why a `Mutex` around the ring buffer
//!
//! [`SpanRingBuffer::push`] takes `&mut self` because the buffer is
//! documented as single-producer, and in practice it is: the reactor runs one
//! Tokio task per flow, so exactly one writer touches a given flow's buffer.
//! But `EventSink` methods take `&self`, so the shared collector needs
//! interior mutability to reach it. The mutex is therefore uncontended in
//! normal operation — the cost is an uncontended lock/unlock pair, which sits
//! comfortably inside the Diagnostic level's per-element budget, and the
//! mutex is never held across an await or a syscall.
//!
//! Nothing here runs at Production level: the collector checks the level
//! before it reaches this module.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use torvyn_types::{ComponentId, FlowId, SpanId};

use super::context::FlowTraceContext;
use super::ring_buffer::{CompactSpanRecord, SpanRingBuffer};

/// A flow's tracing state.
pub struct FlowTraceState {
    /// The flow's trace context, fixed for the flow's lifetime.
    context: FlowTraceContext,
    /// Retained spans, oldest evicted first once capacity is reached.
    spans: Mutex<SpanRingBuffer>,
}

impl FlowTraceState {
    /// Create trace state for a flow.
    ///
    /// # COLD PATH — once per flow.
    #[must_use]
    pub fn new(context: FlowTraceContext, ring_buffer_capacity: usize) -> Self {
        Self {
            context,
            spans: Mutex::new(SpanRingBuffer::new(ring_buffer_capacity)),
        }
    }

    /// The flow's trace context.
    #[must_use]
    pub const fn context(&self) -> &FlowTraceContext {
        &self.context
    }

    /// Whether head sampling selected this flow for tracing.
    #[must_use]
    pub fn is_sampled(&self) -> bool {
        self.context.flags.is_sampled()
    }

    /// Push one span. Returns `false` if the buffer's lock was poisoned,
    /// which the caller reports as a dropped span rather than a panic — an
    /// observability failure must never take a flow down with it.
    ///
    /// # HOT PATH at Diagnostic level.
    pub fn push_span(&self, record: CompactSpanRecord) -> bool {
        match self.spans.lock() {
            Ok(mut buffer) => {
                buffer.push(record);
                true
            }
            Err(_) => false,
        }
    }

    /// Take every retained span, oldest first, emptying the buffer.
    ///
    /// # COLD PATH — export or inspection.
    #[must_use]
    pub fn drain_spans(&self) -> Vec<CompactSpanRecord> {
        self.spans
            .lock()
            .map(|mut buffer| buffer.drain())
            .unwrap_or_default()
    }

    /// Number of spans currently retained.
    #[must_use]
    pub fn span_count(&self) -> usize {
        self.spans.lock().map(|buffer| buffer.len()).unwrap_or(0)
    }

    /// Whether the ring buffer has wrapped, i.e. whether older spans were
    /// evicted to make room. A trace built from a wrapped buffer shows the
    /// most recent window, not the whole run, and callers are expected to say
    /// so rather than present a partial trace as a complete one.
    #[must_use]
    pub fn has_wrapped(&self) -> bool {
        self.spans
            .lock()
            .map(|buffer| buffer.has_wrapped())
            .unwrap_or(false)
    }
}

/// Registry of per-flow trace state.
#[derive(Default)]
pub struct TraceRegistry {
    flows: RwLock<HashMap<FlowId, Arc<FlowTraceState>>>,
}

impl TraceRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flows: RwLock::new(HashMap::new()),
        }
    }

    /// Register a flow's trace state, replacing any previous entry for the
    /// same id.
    ///
    /// # COLD PATH — once per flow.
    pub fn register(&self, flow_id: FlowId, state: Arc<FlowTraceState>) {
        if let Ok(mut flows) = self.flows.write() {
            flows.insert(flow_id, state);
        }
    }

    /// Look up a flow's trace state.
    ///
    /// # HOT PATH at Diagnostic level — a read lock and a hash lookup, the
    /// same shape as the metrics registry's per-flow lookup.
    #[must_use]
    pub fn get(&self, flow_id: FlowId) -> Option<Arc<FlowTraceState>> {
        self.flows.read().ok()?.get(&flow_id).map(Arc::clone)
    }

    /// Remove a flow's trace state, returning it.
    ///
    /// # COLD PATH
    pub fn deregister(&self, flow_id: FlowId) -> Option<Arc<FlowTraceState>> {
        self.flows.write().ok()?.remove(&flow_id)
    }

    /// Number of flows currently registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.flows.read().map(|flows| flows.len()).unwrap_or(0)
    }

    /// Whether any flow is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Derive the span id for one component invocation.
///
/// Deterministic in `(flow_id, component_id, element_sequence)`, which buys
/// two things over a randomly generated id. It is unique for distinct
/// invocations within a flow by construction rather than by luck — and
/// [`generate_span_id`](super::context::generate_span_id) seeds itself from
/// the current nanosecond, so two spans created inside the same nanosecond
/// receive the *same* id, which on the hot path is not hypothetical. It also
/// costs a few arithmetic operations instead of a clock read.
///
/// The mix is the SplitMix64 finalizer, which has good avalanche behaviour
/// over the low-entropy inputs used here.
///
/// # HOT PATH at Diagnostic level.
#[inline]
#[must_use]
pub fn derive_span_id(flow_id: FlowId, component_id: ComponentId, element_sequence: u64) -> SpanId {
    // Combine the three identifiers into one key. The rotations keep the
    // component and flow bits away from the sequence's low bits, which are
    // the ones that vary fastest.
    let key =
        element_sequence ^ component_id.as_u64().rotate_left(21) ^ flow_id.as_u64().rotate_left(43);
    SpanId::new(splitmix64(key).to_le_bytes())
}

/// SplitMix64 finalizer.
#[inline]
fn splitmix64(mut x: u64) -> u64 {
    // A zero key would otherwise map to zero, and an all-zero span id is
    // reserved by the W3C spec to mean "no span".
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracer::context::{generate_span_id, generate_trace_id};
    use std::collections::HashSet;
    use torvyn_types::TraceContext;

    fn state(capacity: usize) -> Arc<FlowTraceState> {
        let mut ctx =
            FlowTraceContext::new(generate_trace_id(), generate_span_id(), FlowId::new(1));
        ctx.set_sampled();
        Arc::new(FlowTraceState::new(ctx, capacity))
    }

    fn record(seq: u64) -> CompactSpanRecord {
        CompactSpanRecord {
            span_id: SpanId::new([1; 8]),
            parent_span_id: SpanId::invalid(),
            component_id: ComponentId::new(1),
            start_ns: seq * 100,
            end_ns: seq * 100 + 50,
            status_code: 0,
            element_sequence: seq,
        }
    }

    #[test]
    fn push_then_drain_returns_records_in_order() {
        let s = state(8);
        for i in 0..5 {
            assert!(s.push_span(record(i)));
        }
        assert_eq!(s.span_count(), 5);
        assert!(!s.has_wrapped());

        let drained = s.drain_spans();
        assert_eq!(drained.len(), 5);
        assert_eq!(drained[0].element_sequence, 0);
        assert_eq!(drained[4].element_sequence, 4);
        assert_eq!(s.span_count(), 0);
    }

    #[test]
    fn wrapping_is_observable() {
        let s = state(8);
        for i in 0..12 {
            assert!(s.push_span(record(i)));
        }
        assert!(s.has_wrapped());
        let drained = s.drain_spans();
        assert_eq!(drained.len(), 8);
        assert_eq!(drained[0].element_sequence, 4);
    }

    #[test]
    fn registry_register_get_deregister() {
        let reg = TraceRegistry::new();
        assert!(reg.is_empty());

        reg.register(FlowId::new(7), state(8));
        assert_eq!(reg.len(), 1);
        assert!(reg.get(FlowId::new(7)).is_some());
        assert!(reg.get(FlowId::new(8)).is_none());

        assert!(reg.deregister(FlowId::new(7)).is_some());
        assert!(reg.is_empty());
        assert!(reg.deregister(FlowId::new(7)).is_none());
    }

    #[test]
    fn derived_span_ids_are_unique_per_invocation() {
        let flow = FlowId::new(3);
        let mut seen = HashSet::new();
        for component in 1..=4u64 {
            for sequence in 0..1_000u64 {
                let id = derive_span_id(flow, ComponentId::new(component), sequence);
                assert!(id.is_valid(), "span id must be non-zero per W3C");
                assert!(
                    seen.insert(id),
                    "collision at component {component}, sequence {sequence}"
                );
            }
        }
        assert_eq!(seen.len(), 4_000);
    }

    #[test]
    fn derived_span_ids_differ_across_flows() {
        let a = derive_span_id(FlowId::new(1), ComponentId::new(1), 0);
        let b = derive_span_id(FlowId::new(2), ComponentId::new(1), 0);
        assert_ne!(a, b);
    }

    #[test]
    fn derived_span_id_is_deterministic() {
        let a = derive_span_id(FlowId::new(9), ComponentId::new(2), 42);
        let b = derive_span_id(FlowId::new(9), ComponentId::new(2), 42);
        assert_eq!(a, b);
    }

    #[test]
    fn trace_context_is_exposed_for_propagation() {
        let s = state(8);
        let ctx: TraceContext = s.context().trace_ctx;
        assert!(ctx.is_valid());
        assert!(s.is_sampled());
    }
}
