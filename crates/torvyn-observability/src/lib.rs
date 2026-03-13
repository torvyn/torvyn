//! # torvyn-observability
//!
//! Tracing, metrics, diagnostics, and runtime inspection for the Torvyn
//! reactive streaming runtime.
//!
//! This crate provides the observability subsystem defined in HLI Doc 05.
//! It implements the [`EventSink`](torvyn_types::EventSink) trait from
//! `torvyn-types` for hot-path recording and manages three internal
//! subsystems:
//!
//! - **Metrics**: Pre-allocated counters, histograms, and gauges scoped per
//!   flow, component, and stream.
//! - **Tracing**: Trace context propagation, sampling, and span ring buffers
//!   for retroactive export.
//! - **Events**: Structured diagnostic events for lifecycle, performance,
//!   error, security, and resource state transitions.
//!
//! ## Architecture
//!
//! The [`ObservabilityCollector`] is the central orchestrator. It:
//! - Implements `EventSink` for the reactor, resource manager, and host
//!   lifecycle manager.
//! - Owns the [`MetricsRegistry`] that holds pre-allocated per-flow metrics.
//! - Manages the event channel and diagnostic event buffer.
//! - Provides [`FlowObserver`] handles to the reactor for per-flow recording.
//!
//! ## Observability Levels
//!
//! | Level | Overhead | What's collected |
//! |-------|----------|------------------|
//! | Off | 0 | Nothing |
//! | Production | < 500ns/element | Counters, histograms, sampled traces |
//! | Diagnostic | < 2μs/element | All of Production + per-element spans, events |
//!
//! Level switching is atomic and does not require restarting flows.

#![deny(missing_docs)]

pub mod bench;
pub mod collector;
pub mod config;
pub mod events;
pub mod export;
pub mod metrics;
pub mod tracer;

// Re-exports for public API.
pub use bench::BenchmarkReport;
pub use collector::{FlowObserver, ObservabilityCollector};
pub use config::ObservabilityConfig;
pub use events::{DiagnosticEvent, EventCategory, EventPayload};
pub use metrics::{
    Counter, FlowMetricsSnapshot, Gauge, Histogram, HistogramSnapshot, MetricsRegistry,
    LATENCY_BUCKETS_NS, SIZE_BUCKETS_BYTES,
};
pub use tracer::{
    CompactSpanRecord, FlowTraceContext, Sampler, SamplingDecision, SpanRingBuffer, TraceFlags,
};
