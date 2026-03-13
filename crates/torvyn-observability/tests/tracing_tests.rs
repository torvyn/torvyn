//! Integration tests for the tracing subsystem.

use torvyn_observability::tracer::*;
use torvyn_types::{ComponentId, FlowId, SpanId, TraceId};

#[test]
fn test_trace_context_propagation() {
    let trace_id = generate_trace_id();
    let root_span = generate_span_id();
    let ctx = FlowTraceContext::new(trace_id, root_span, FlowId::new(1));

    // Create child contexts for component invocations.
    let child1_span = generate_span_id();
    let child1 = ctx.child(child1_span);
    assert_eq!(child1.parent_span_id, root_span);
    assert_eq!(child1.trace_ctx.trace_id, trace_id);

    let child2_span = generate_span_id();
    let child2 = child1.child(child2_span);
    assert_eq!(child2.parent_span_id, child1_span);
    assert_eq!(child2.trace_ctx.trace_id, trace_id);
}

#[test]
fn test_span_ring_buffer_fill_and_drain() {
    let mut rb = SpanRingBuffer::new(64);

    for i in 0..100u64 {
        rb.push(CompactSpanRecord {
            span_id: SpanId::new(i.to_le_bytes()),
            parent_span_id: SpanId::invalid(),
            component_id: ComponentId::new(1),
            start_ns: i * 1000,
            end_ns: (i + 1) * 1000,
            status_code: 0,
            element_sequence: i,
        });
    }

    assert!(rb.has_wrapped());
    let records = rb.drain();
    assert_eq!(records.len(), 64);
    // Oldest should be 100 - 64 = 36.
    assert_eq!(records[0].element_sequence, 36);
    assert_eq!(records[63].element_sequence, 99);
}

#[test]
fn test_sampling_decision_deterministic() {
    use torvyn_observability::config::TracingConfig;

    let mut config = TracingConfig::default();
    config.sample_rate = 0.5;
    let sampler = Sampler::new(&config);

    let trace_id = [0xAB; 16];
    let decision1 = sampler.should_sample_head(&trace_id);
    let decision2 = sampler.should_sample_head(&trace_id);
    assert_eq!(
        decision1, decision2,
        "same trace ID should give same decision"
    );
}

#[test]
fn test_trace_context_sampling_flags() {
    let mut ctx = FlowTraceContext::new(
        TraceId::new([1; 16]),
        SpanId::new([2; 8]),
        FlowId::new(1),
    );

    assert!(!ctx.flags.is_sampled());
    ctx.set_sampled();
    assert!(ctx.flags.is_sampled());

    ctx.set_diagnostic();
    assert!(ctx.flags.is_diagnostic());
    assert!(ctx.flags.is_sampled()); // still sampled
}

#[test]
fn test_w3c_trace_id_format() {
    let trace_id = generate_trace_id();
    let formatted = format!("{trace_id}");
    // W3C trace ID is 32 lowercase hex characters.
    assert_eq!(formatted.len(), 32);
    assert!(formatted.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_span_id_format() {
    let span_id = generate_span_id();
    let formatted = format!("{span_id}");
    // Span ID is 16 lowercase hex characters.
    assert_eq!(formatted.len(), 16);
    assert!(formatted.chars().all(|c| c.is_ascii_hexdigit()));
}
