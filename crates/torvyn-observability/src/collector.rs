//! Central observability collector.
//!
//! Implements `EventSink` (from `torvyn-types`) for hot-path recording.
//! Manages the metrics registry, tracer, event recorder, and export pipeline.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use torvyn_types::{
    ComponentId, CopyReason, EventSink, FlowId, InvocationStatus, ObservabilityLevel, ResourceId,
    StreamId,
};

use tokio::sync::mpsc;

use crate::config::{ExportTarget, ObservabilityConfig};
use crate::events::{event_channel, DiagnosticEvent, EventBuffer, EventSender};
use crate::export::json::{metrics_export_task, JsonTarget};
use crate::metrics::flow_metrics::FlowMetrics;
use crate::metrics::registry::MetricsRegistry;
use crate::metrics::snapshot::{snapshot_flow, FlowMetricsSnapshot};
use crate::tracer::{
    generate_span_id, generate_trace_id, FlowTraceContext, Sampler, SpanRingBuffer,
};

/// The central observability collector.
///
/// Implements `EventSink` for hot-path recording and manages all internal
/// observability modules.
///
/// # Thread Safety
/// All methods are `Send + Sync`. Hot-path methods use only atomic operations
/// and non-blocking channel sends.
///
/// # Invariants
/// - `level` is always a valid `ObservabilityLevel` discriminant (0, 1, or 2).
/// - `metrics` registry is consistent: every registered flow has metrics.
pub struct ObservabilityCollector {
    /// Current observability level (atomic for hot-path checking).
    level: Arc<AtomicU8>,
    /// Metrics registry.
    metrics: Arc<MetricsRegistry>,
    /// Event channel sender for diagnostic events.
    event_tx: EventSender,
    /// Event buffer for inspection API.
    event_buffer: Arc<EventBuffer>,
    /// Trace sampler.
    sampler: Sampler,
    /// Configuration.
    config: ObservabilityConfig,
    /// Held to keep the periodic metrics export task running; dropping it (on
    /// collector teardown) closes the task's stop channel and the task exits.
    /// `None` when export is disabled or targets a not-yet-wired transport.
    _export_stop: Option<mpsc::Sender<()>>,
}

impl ObservabilityCollector {
    /// Create a new collector with the given configuration.
    ///
    /// Spawns the background event recorder and, when the configured export
    /// target is `Stdout` or `File`, the periodic metrics exporter. Must be
    /// called from within a Tokio runtime.
    ///
    /// # COLD PATH — called once at runtime startup.
    ///
    /// # Errors
    /// Returns error if configuration validation fails.
    pub fn new(
        config: ObservabilityConfig,
    ) -> Result<Self, Vec<crate::config::ConfigValidationError>> {
        config.validate()?;

        let level = Arc::new(AtomicU8::new(config.level as u8));
        let metrics = Arc::new(MetricsRegistry::new());
        let (event_tx, event_rx) = event_channel(config.event_channel_capacity);
        let event_buffer = Arc::new(EventBuffer::new(config.event_channel_capacity));
        let sampler = Sampler::new(&config.tracing);

        // Spawn event recorder task.
        let buffer_clone = Arc::clone(&event_buffer);
        tokio::spawn(crate::events::recorder::event_recorder_task(
            event_rx,
            buffer_clone,
            None,
        ));

        // Spawn the periodic metrics exporter for the self-contained JSON
        // targets (stdout / file). `interval` is validated to be non-zero
        // above. OTLP-over-HTTP and channel targets are not yet wired.
        let export_stop = match &config.export.target {
            ExportTarget::Stdout | ExportTarget::File(_) => {
                let json_target = match &config.export.target {
                    ExportTarget::File(path) => JsonTarget::File(path.clone()),
                    _ => JsonTarget::Stderr,
                };
                let (stop_tx, stop_rx) = mpsc::channel(1);
                tokio::spawn(metrics_export_task(
                    Arc::clone(&metrics),
                    json_target,
                    config.export.interval,
                    stop_rx,
                ));
                Some(stop_tx)
            }
            ExportTarget::OtlpHttp => {
                tracing::warn!(
                    "observability: OTLP export is configured but not yet implemented; \
                     no metrics will be exported"
                );
                None
            }
            ExportTarget::None | ExportTarget::Channel => None,
        };

        Ok(Self {
            level,
            metrics,
            event_tx,
            event_buffer,
            sampler,
            config,
            _export_stop: export_stop,
        })
    }

    /// Create a collector for testing without spawning background tasks.
    ///
    /// # COLD PATH
    pub fn new_for_testing(config: ObservabilityConfig) -> Self {
        let level = Arc::new(AtomicU8::new(config.level as u8));
        let metrics = Arc::new(MetricsRegistry::new());
        let (event_tx, _event_rx) = event_channel(config.event_channel_capacity);
        let event_buffer = Arc::new(EventBuffer::new(config.event_channel_capacity));
        let sampler = Sampler::new(&config.tracing);

        Self {
            level,
            metrics,
            event_tx,
            event_buffer,
            sampler,
            config,
            _export_stop: None,
        }
    }

    /// Register a new flow and pre-allocate its metric structures.
    ///
    /// Returns a `FlowObserver` handle for the reactor to use.
    ///
    /// # COLD PATH
    ///
    /// # Errors
    /// Returns error if the flow is already registered.
    pub fn register_flow(
        &self,
        flow_id: FlowId,
        component_ids: &[ComponentId],
        stream_ids: &[StreamId],
    ) -> Result<FlowObserver, crate::metrics::registry::RegistryError> {
        let start_time_ns = crate::events::current_time_ns();
        let flow_metrics =
            self.metrics
                .register_flow(flow_id, component_ids, stream_ids, start_time_ns)?;

        let trace_id = generate_trace_id();
        let root_span_id = generate_span_id();
        let mut trace_ctx = FlowTraceContext::new(trace_id, root_span_id, flow_id);

        // Head-based sampling decision.
        let sampled = self.sampler.should_sample_head(trace_id.as_bytes());
        if sampled == crate::tracer::SamplingDecision::Sample {
            trace_ctx.set_sampled();
        }

        let ring_buffer = SpanRingBuffer::new(self.config.tracing.ring_buffer_capacity);

        Ok(FlowObserver {
            flow_id,
            metrics: flow_metrics,
            trace_ctx,
            ring_buffer,
            level: Arc::clone(&self.level),
            event_tx: self.event_tx.clone(),
            sampler_error_promote: self.sampler.should_promote_error(),
        })
    }

    /// Deregister a flow and return its final metrics snapshot.
    ///
    /// # COLD PATH
    ///
    /// # Errors
    /// Returns error if the flow is not registered.
    pub fn deregister_flow(
        &self,
        flow_id: FlowId,
    ) -> Result<FlowMetricsSnapshot, crate::metrics::registry::RegistryError> {
        let metrics = self.metrics.deregister_flow(flow_id)?;
        Ok(snapshot_flow(&metrics))
    }

    /// Take a metrics snapshot for a flow without deregistering it.
    ///
    /// # COLD PATH
    pub fn snapshot(&self, flow_id: FlowId) -> Option<FlowMetricsSnapshot> {
        let metrics = self.metrics.get_flow(flow_id)?;
        Some(snapshot_flow(&metrics))
    }

    /// Set the observability level at runtime.
    ///
    /// # COLD PATH
    pub fn set_level(&self, level: ObservabilityLevel) {
        self.level.store(level as u8, Ordering::Release);
    }

    /// Get the current observability level.
    #[inline]
    pub fn current_level(&self) -> ObservabilityLevel {
        ObservabilityLevel::from_u8(self.level.load(Ordering::Acquire))
            .unwrap_or(ObservabilityLevel::Off)
    }

    /// Get a reference to the metrics registry.
    pub fn registry(&self) -> &Arc<MetricsRegistry> {
        &self.metrics
    }

    /// Get a reference to the event buffer.
    pub fn event_buffer(&self) -> &Arc<EventBuffer> {
        &self.event_buffer
    }

    /// Emit a diagnostic event.
    ///
    /// # WARM PATH — used for lifecycle, security, and error events.
    pub fn emit_event(&self, event: DiagnosticEvent) {
        // Best-effort: don't block if channel is full.
        if self.event_tx.try_send(event).is_err() {
            self.metrics.system.events_dropped.increment(1);
        }
    }

    /// Graceful shutdown: signal all background tasks to stop.
    ///
    /// # COLD PATH
    pub async fn shutdown(&self) {
        // Dropping the sender will cause the recorder task to exit.
        // For a real implementation, we'd keep a JoinHandle and await it.
    }
}

/// Implement the `EventSink` trait from `torvyn-types`.
///
/// All methods are non-blocking and allocation-free on the hot path.
impl EventSink for ObservabilityCollector {
    /// # HOT PATH
    #[inline]
    fn record_invocation(
        &self,
        flow_id: FlowId,
        component_id: ComponentId,
        start_ns: u64,
        end_ns: u64,
        status: InvocationStatus,
    ) {
        let level = self.current_level();
        if !level.is_enabled() {
            return;
        }

        // Look up pre-allocated metrics.
        if let Some(flow_metrics) = self.metrics.get_flow(flow_id) {
            let duration_ns = end_ns.saturating_sub(start_ns);

            // Always at Production level: update counters and histogram.
            flow_metrics.elements_total.increment(1);
            if let Some(comp) = flow_metrics.component(component_id) {
                comp.invocations.increment(1);
                comp.processing_time.record(duration_ns);
                if !matches!(status, InvocationStatus::Ok) {
                    comp.errors.increment(1);
                    flow_metrics.errors_total.increment(1);
                }
            }
        }
    }

    /// # HOT PATH
    #[inline]
    fn record_element_transfer(
        &self,
        flow_id: FlowId,
        stream_id: StreamId,
        _element_sequence: u64,
        queue_depth_after: u32,
    ) {
        let level = self.current_level();
        if !level.is_enabled() {
            return;
        }

        if let Some(flow_metrics) = self.metrics.get_flow(flow_id) {
            if let Some(stream) = flow_metrics.stream(stream_id) {
                stream.elements.increment(1);
                stream.queue_depth.set(queue_depth_after as u64);
                stream.queue_depth_peak.update_max(queue_depth_after as u64);
            }
        }
    }

    /// # WARM PATH
    #[inline]
    fn record_backpressure(
        &self,
        flow_id: FlowId,
        stream_id: StreamId,
        activated: bool,
        _queue_depth: u32,
        _timestamp_ns: u64,
    ) {
        let level = self.current_level();
        if !level.is_enabled() {
            return;
        }

        if let Some(flow_metrics) = self.metrics.get_flow(flow_id) {
            if let Some(stream) = flow_metrics.stream(stream_id) {
                if activated {
                    stream.backpressure_events.increment(1);
                }
            }
        }
    }

    /// # HOT PATH
    #[inline]
    fn record_copy(
        &self,
        flow_id: FlowId,
        _resource_id: ResourceId,
        _from_component: ComponentId,
        _to_component: ComponentId,
        copy_bytes: u64,
        _reason: CopyReason,
    ) {
        let level = self.current_level();
        if !level.is_enabled() {
            return;
        }

        if let Some(flow_metrics) = self.metrics.get_flow(flow_id) {
            flow_metrics.copies_total.increment(1);
            flow_metrics.copy_bytes_total.increment(copy_bytes);
        }
    }

    /// # HOT PATH
    #[inline]
    fn record_flow_latency(&self, flow_id: FlowId, latency_ns: u64) {
        let level = self.current_level();
        if !level.is_enabled() {
            return;
        }

        if let Some(flow_metrics) = self.metrics.get_flow(flow_id) {
            flow_metrics.end_to_end_latency.record(latency_ns);
        }
    }

    /// # HOT PATH
    #[inline]
    fn level(&self) -> ObservabilityLevel {
        self.current_level()
    }

    /// Pre-allocate this flow's metric slots so that subsequent
    /// `record_invocation` / `record_element_transfer` calls — which look the
    /// flow up by id and silently skip unregistered flows — actually record.
    ///
    /// The returned [`FlowObserver`] is intentionally dropped: registration
    /// also stores the flow's `Arc<FlowMetrics>` in the registry, which is the
    /// handle the hot-path recorders consult. Trace-context bookkeeping carried
    /// by the observer is not needed for metric recording.
    ///
    /// # COLD PATH — once per flow, before the flow driver starts.
    fn on_flow_start(
        &self,
        flow_id: FlowId,
        component_ids: &[ComponentId],
        stream_ids: &[StreamId],
    ) {
        if let Err(error) = self.register_flow(flow_id, component_ids, stream_ids) {
            // Registration only fails on a duplicate flow id or a registry
            // capacity limit; neither should stall the flow. Log and continue —
            // the flow runs with metrics recording degraded to a no-op rather
            // than failing to start.
            tracing::warn!(
                flow_id = %flow_id,
                %error,
                "observability: flow metrics registration failed; metrics disabled for this flow"
            );
        }
    }
}

/// Per-flow handle with pre-allocated metric structures.
///
/// Passed to the reactor for hot-path recording. Holds the flow's trace
/// context, span ring buffer, and a reference to the flow's metrics.
///
/// # Thread Safety
/// The `FlowObserver` is designed for single-owner usage within a flow
/// task. The `metrics` field is `Arc<FlowMetrics>` and can be shared.
pub struct FlowObserver {
    /// Flow identifier.
    pub flow_id: FlowId,
    /// Pre-allocated flow metrics (shared with registry).
    pub metrics: Arc<FlowMetrics>,
    /// Flow trace context.
    pub trace_ctx: FlowTraceContext,
    /// Span ring buffer for retroactive sampling.
    pub ring_buffer: SpanRingBuffer,
    /// Level reference (shared with collector).
    level: Arc<AtomicU8>,
    /// Event channel for diagnostic events.
    event_tx: EventSender,
    /// Whether error-triggered promotion is enabled.
    sampler_error_promote: bool,
}

impl FlowObserver {
    /// Check the current observability level.
    #[inline]
    pub fn level(&self) -> ObservabilityLevel {
        ObservabilityLevel::from_u8(self.level.load(Ordering::Acquire))
            .unwrap_or(ObservabilityLevel::Off)
    }

    /// Record a component invocation into the span ring buffer.
    ///
    /// # HOT PATH when sampled.
    pub fn record_span(
        &mut self,
        component_id: ComponentId,
        start_ns: u64,
        end_ns: u64,
        status_code: u8,
        element_sequence: u64,
    ) {
        let span_id = generate_span_id();
        let record = crate::tracer::CompactSpanRecord {
            span_id,
            parent_span_id: self.trace_ctx.trace_ctx.span_id,
            component_id,
            start_ns,
            end_ns,
            status_code,
            element_sequence,
        };
        self.ring_buffer.push(record);
    }

    /// Promote the flow to sampled (e.g., due to error or latency).
    pub fn promote(&mut self) {
        self.trace_ctx.set_sampled();
    }

    /// Check if this flow is currently sampled.
    #[inline]
    pub fn is_sampled(&self) -> bool {
        self.trace_ctx.flags.is_sampled()
    }

    /// Whether error-triggered promotion is enabled.
    #[inline]
    pub fn error_promote_enabled(&self) -> bool {
        self.sampler_error_promote
    }

    /// Emit a diagnostic event through the event channel.
    ///
    /// # WARM PATH
    pub fn emit_event(&self, event: DiagnosticEvent) {
        let _ = self.event_tx.try_send(event);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ObservabilityConfig {
        ObservabilityConfig::default()
    }

    #[test]
    fn test_collector_creation() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        assert_eq!(collector.current_level(), ObservabilityLevel::Production);
    }

    #[tokio::test]
    async fn test_new_exports_metrics_to_file_target() {
        let path = std::env::temp_dir().join(format!(
            "torvyn_collector_export_{}.ndjson",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        // Configure a file export target on a short interval, then build a real
        // collector (which spawns the periodic exporter).
        let mut config = test_config();
        config.export.target = ExportTarget::File(path.clone());
        config.export.interval = std::time::Duration::from_millis(20);
        let collector = ObservabilityCollector::new(config).unwrap();

        // Register a flow and record an invocation through the collector.
        collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(0)])
            .unwrap();
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            1_000,
            InvocationStatus::Ok,
        );

        // Let the exporter run, then tear the collector down (stops the task).
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        drop(collector);

        let contents = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            contents.contains("\"elements_total\":1"),
            "the collector must export the flow's recorded metrics; got: {contents}",
        );
    }

    #[test]
    fn test_collector_set_level() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        collector.set_level(ObservabilityLevel::Diagnostic);
        assert_eq!(collector.current_level(), ObservabilityLevel::Diagnostic);
    }

    #[test]
    fn test_collector_register_flow() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        let observer = collector
            .register_flow(
                FlowId::new(1),
                &[ComponentId::new(1), ComponentId::new(2)],
                &[StreamId::new(1)],
            )
            .unwrap();

        assert_eq!(observer.flow_id, FlowId::new(1));
        assert_eq!(observer.metrics.components.len(), 2);
    }

    #[test]
    fn test_collector_register_duplicate_flow_fails() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        let result =
            collector.register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)]);
        assert!(result.is_err());
    }

    #[test]
    fn test_collector_implements_event_sink() {
        let collector = ObservabilityCollector::new_for_testing(test_config());

        // Register a flow first.
        let _obs = collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        // Exercise EventSink methods.
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            1000,
            2000,
            InvocationStatus::Ok,
        );

        collector.record_element_transfer(FlowId::new(1), StreamId::new(1), 0, 5);
        collector.record_backpressure(FlowId::new(1), StreamId::new(1), true, 64, 3000);
        collector.record_copy(
            FlowId::new(1),
            ResourceId::new(0, 0),
            ComponentId::new(1),
            ComponentId::new(2),
            1024,
            CopyReason::CrossComponent,
        );

        // Verify metrics were updated.
        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        assert_eq!(snap.elements_total, 1);
        assert_eq!(snap.copies_total, 1);
        assert_eq!(snap.copy_bytes_total, 1024);
    }

    #[test]
    fn test_record_flow_latency_populates_end_to_end_histogram() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        // Before any latency is recorded, the histogram is empty.
        assert_eq!(
            collector.snapshot(FlowId::new(1)).unwrap().latency_p50_ns,
            0
        );

        // Record a spread of end-to-end latencies (1µs .. 1000µs).
        for i in 1..=1000u64 {
            collector.record_flow_latency(FlowId::new(1), i * 1_000);
        }

        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        // Percentiles are now populated and monotonically non-decreasing,
        // including the newly added p90. (Percentile-vs-max is not asserted:
        // bucketed percentiles are boundary estimates, not raw values.)
        assert!(snap.latency_p50_ns > 0, "p50 must be populated");
        assert!(snap.latency_p50_ns <= snap.latency_p90_ns);
        assert!(snap.latency_p90_ns <= snap.latency_p95_ns);
        assert!(snap.latency_p95_ns <= snap.latency_p99_ns);
        assert!(snap.latency_p99_ns <= snap.latency_p999_ns);
        assert!(snap.latency_max_ns > 0, "max must be populated");
    }

    #[test]
    fn test_record_flow_latency_skipped_when_level_off() {
        let mut config = test_config();
        config.level = ObservabilityLevel::Off;
        let collector = ObservabilityCollector::new_for_testing(config);
        collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        collector.record_flow_latency(FlowId::new(1), 5_000);

        assert_eq!(
            collector.snapshot(FlowId::new(1)).unwrap().latency_p50_ns,
            0
        );
    }

    #[test]
    fn test_on_flow_start_registers_flow_for_recording() {
        let collector = ObservabilityCollector::new_for_testing(test_config());

        // Drive registration through the `EventSink::on_flow_start` hook (the
        // path the reactor uses) rather than the inherent `register_flow`.
        EventSink::on_flow_start(
            &collector,
            FlowId::new(1),
            &[ComponentId::new(1), ComponentId::new(2)],
            &[StreamId::new(0)],
        );

        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            1000,
            2000,
            InvocationStatus::Ok,
        );
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(2),
            2000,
            2500,
            InvocationStatus::Ok,
        );

        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        assert_eq!(
            snap.elements_total, 2,
            "on_flow_start must pre-allocate metrics so invocations record"
        );
    }

    #[test]
    fn test_record_without_on_flow_start_is_silently_skipped() {
        // A flow that was never registered has no metric slots; recording must
        // be a no-op (and must not panic), which is exactly why the reactor
        // calls `on_flow_start` before the driver emits.
        let collector = ObservabilityCollector::new_for_testing(test_config());

        collector.record_invocation(
            FlowId::new(42),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );

        assert!(collector.snapshot(FlowId::new(42)).is_none());
    }

    #[test]
    fn test_on_flow_start_duplicate_does_not_panic() {
        // A duplicate registration is logged and ignored, never panics.
        let collector = ObservabilityCollector::new_for_testing(test_config());
        EventSink::on_flow_start(&collector, FlowId::new(1), &[ComponentId::new(1)], &[]);
        EventSink::on_flow_start(&collector, FlowId::new(1), &[ComponentId::new(1)], &[]);

        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );
        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        assert_eq!(snap.elements_total, 1);
    }

    #[test]
    fn test_collector_off_level_skips_recording() {
        let mut config = test_config();
        config.level = ObservabilityLevel::Off;
        let collector = ObservabilityCollector::new_for_testing(config);

        let _obs = collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );

        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        assert_eq!(snap.elements_total, 0);
    }

    #[test]
    fn test_collector_deregister_flow() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        let snap = collector.deregister_flow(FlowId::new(1)).unwrap();
        assert_eq!(snap.flow_id, FlowId::new(1));
        assert!(collector.snapshot(FlowId::new(1)).is_none());
    }

    #[test]
    fn test_collector_event_sink_level() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        assert_eq!(collector.level(), ObservabilityLevel::Production);
    }

    #[test]
    fn test_flow_observer_record_span() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        let mut observer = collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        observer.record_span(ComponentId::new(1), 0, 100, 0, 0);
        assert_eq!(observer.ring_buffer.len(), 1);
    }

    #[test]
    fn test_flow_observer_promote() {
        let collector = ObservabilityCollector::new_for_testing(test_config());
        let mut observer = collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        // With 1% sample rate, most flows won't be sampled initially.
        observer.promote();
        assert!(observer.is_sampled());
    }

    #[test]
    fn test_level_switching_atomic() {
        let collector = ObservabilityCollector::new_for_testing(test_config());

        let _obs = collector
            .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
            .unwrap();

        // Off → no recording.
        collector.set_level(ObservabilityLevel::Off);
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );
        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        assert_eq!(snap.elements_total, 0);

        // Switch to Production → recording resumes.
        collector.set_level(ObservabilityLevel::Production);
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );
        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        assert_eq!(snap.elements_total, 1);

        // Switch to Diagnostic → still recording.
        collector.set_level(ObservabilityLevel::Diagnostic);
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );
        let snap = collector.snapshot(FlowId::new(1)).unwrap();
        assert_eq!(snap.elements_total, 2);
    }
}
