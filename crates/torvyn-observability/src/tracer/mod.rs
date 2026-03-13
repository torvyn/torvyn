//! Tracing subsystem: context, sampling, and span ring buffers.

pub mod context;
pub mod ring_buffer;
pub mod sampling;

pub use context::{FlowTraceContext, TraceFlags, generate_span_id, generate_trace_id};
pub use ring_buffer::{CompactSpanRecord, SpanRingBuffer};
pub use sampling::{Sampler, SamplingDecision};
