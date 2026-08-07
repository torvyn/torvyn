//! `torvyn trace` — run a pipeline with per-element tracing enabled.
//!
//! Runs the flow at [`ObservabilityLevel::Diagnostic`], which is the level at
//! which the runtime retains a span per component invocation rather than
//! folding invocations into aggregate histograms. Those spans are then
//! grouped by the element that produced them, so the output shows each
//! element's actual path through the pipeline.
//!
//! # What the numbers are
//!
//! Every figure comes from the run that just happened:
//!
//! - **Per-span durations** are the reactor's own invocation timings, taken
//!   around the guest call.
//! - **Element totals** are the span durations for one element summed across
//!   stages — the time that element spent *inside components*, which is less
//!   than its wall-clock journey because it excludes time queued between
//!   stages.
//! - **Summary latency percentiles** are the flow's end-to-end latency
//!   histogram, measured from an element's pipeline-entry timestamp to its
//!   consumption at the sink, so they *do* include queueing.
//! - **Copies** and **backpressure events** come from the flow's metrics.
//!
//! The two latency views differ for a real reason, and the report labels them
//! accordingly rather than presenting one as the other.
//!
//! # Sampling and buffer capacity
//!
//! Tracing runs at a sample rate of 1.0 so every flow this command starts is
//! traced. The span buffer is sized from `--limit` where one is given; without
//! one it holds a bounded window, and if the run overflows it the report says
//! so rather than presenting the tail as the whole run.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use torvyn_observability::{CompactSpanRecord, FlowMetricsSnapshot};
use torvyn_types::{EventSink, FlowId, ObservabilityLevel};

use crate::cli::{TraceArgs, TraceFormat};
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};

/// How long to wait for the traced flow to reach a terminal state.
const FLOW_COMPLETION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Poll interval while waiting for the flow to finish.
const FLOW_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// Span-buffer capacity used when `--limit` is not given.
///
/// 8192 slots is 512 KiB per flow, which comfortably covers an interactive
/// trace while staying bounded for a source that never completes.
const DEFAULT_SPAN_CAPACITY: usize = 8192;

/// Upper bound on the span buffer, so a large `--limit` cannot ask for an
/// unbounded allocation. 1 Mi slots is 64 MiB.
const MAX_SPAN_CAPACITY: usize = 1 << 20;

/// Result of `torvyn trace`.
#[derive(Debug, Serialize)]
pub struct TraceResult {
    /// Number of elements the run produced spans for.
    pub elements_traced: u64,
    /// Number of elements included in [`Self::traces`]. Equal to
    /// `elements_traced` unless `--limit` bounded the report.
    pub elements_shown: u64,
    /// Mean end-to-end latency in microseconds, from the flow's latency
    /// histogram (includes time queued between stages).
    pub avg_latency_us: f64,
    /// p50 end-to-end latency in microseconds.
    pub p50_latency_us: f64,
    /// p99 end-to-end latency in microseconds.
    pub p99_latency_us: f64,
    /// Total copies observed.
    pub total_copies: u64,
    /// Total bytes copied.
    pub total_copy_bytes: u64,
    /// Number of backpressure events across all streams.
    pub backpressure_events: u64,
    /// Whether the span buffer overflowed, meaning the traces below are the
    /// most recent window of the run rather than the whole run.
    pub truncated: bool,
    /// W3C trace id for the run, lower-case hex. Empty when the flow was not
    /// traced.
    pub trace_id: String,
    /// Per-stream backpressure detail. Populated only with
    /// `--show-backpressure`; the aggregate count is always in
    /// [`Self::backpressure_events`].
    pub backpressure_by_stream: Vec<StreamBackpressure>,
    /// Per-element traces.
    pub traces: Vec<ElementTrace>,
    /// Flow name.
    pub flow_name: String,
}

/// Backpressure detail for one stream connection.
#[derive(Debug, Serialize)]
pub struct StreamBackpressure {
    /// Stream connection identifier.
    pub stream_id: u64,
    /// Times backpressure activated on this stream.
    pub events: u64,
    /// Deepest the queue got.
    pub queue_depth_peak: u64,
    /// Cumulative time this stream spent backpressured, in nanoseconds.
    pub stalled_ns: u64,
}

/// Trace for a single element through the pipeline.
#[derive(Debug, Serialize)]
pub struct ElementTrace {
    /// The element's origin sequence number, assigned at the source and
    /// carried unchanged through every stage.
    pub element_id: u64,
    /// Per-component spans, ordered by start time.
    pub spans: Vec<ComponentSpan>,
    /// Sum of this element's span durations, in microseconds. This is time
    /// spent inside components and excludes time queued between stages.
    pub total_latency_us: f64,
}

/// A single component's processing span for one element.
#[derive(Debug, Serialize)]
pub struct ComponentSpan {
    /// Component name from the topology.
    pub component: String,
    /// Operation type (`pull`, `process`, `push`, …).
    pub operation: String,
    /// Duration in microseconds.
    pub duration_us: f64,
    /// W3C span id, lower-case hex.
    pub span_id: String,
    /// The flow's root span id, lower-case hex — the parent of every
    /// invocation span in the run.
    pub parent_span_id: String,
    /// Invocation start, nanoseconds since the Unix epoch.
    pub start_unix_nano: u64,
    /// Invocation end, nanoseconds since the Unix epoch.
    pub end_unix_nano: u64,
    /// Whether the invocation failed.
    pub error: bool,
}

impl HumanRenderable for TraceResult {
    fn render_human(&self, ctx: &OutputContext) {
        for trace in &self.traces {
            eprintln!();
            let elem_label = format!("elem-{}", trace.element_id);
            if ctx.color_enabled {
                eprint!("  {}  ", console::style(&elem_label).bold());
            } else {
                eprint!("  {elem_label}  ");
            }

            for (i, span) in trace.spans.iter().enumerate() {
                let connector = if trace.spans.len() == 1 {
                    "──"
                } else if i == 0 {
                    "┬─"
                } else if i == trace.spans.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };

                let marker = if span.error { "  [error]" } else { "" };

                if i > 0 {
                    eprint!("          ");
                }
                eprintln!(
                    "{connector} {:<14} {:<8} {:.1}µs{marker}",
                    span.component, span.operation, span.duration_us,
                );
            }
            eprintln!(
                "          in-component total: {:.1}µs",
                trace.total_latency_us
            );
        }

        terminal::print_header(ctx, "Trace Summary");
        let elements = if self.elements_shown == self.elements_traced {
            format!("{}", self.elements_traced)
        } else {
            format!(
                "{} (showing the first {} — summary below covers all {})",
                self.elements_traced, self.elements_shown, self.elements_traced
            )
        };
        terminal::print_kv(ctx, "Elements traced", &elements);
        terminal::print_kv(
            ctx,
            "End-to-end latency",
            &format!(
                "mean {:.1}µs (p50: {:.1}µs, p99: {:.1}µs)",
                self.avg_latency_us, self.p50_latency_us, self.p99_latency_us
            ),
        );
        terminal::print_kv(
            ctx,
            "Copies",
            &format!("{} ({} bytes)", self.total_copies, self.total_copy_bytes),
        );
        terminal::print_kv(
            ctx,
            "Backpressure",
            &format!("{} events", self.backpressure_events),
        );
        if !self.trace_id.is_empty() {
            terminal::print_kv(ctx, "Trace ID", &self.trace_id);
        }
        for stream in &self.backpressure_by_stream {
            terminal::print_kv(
                ctx,
                &format!("Stream {}", stream.stream_id),
                &format!(
                    "{} backpressure event(s), peak queue depth {}, {:.1}µs stalled",
                    stream.events,
                    stream.queue_depth_peak,
                    ns_to_us(stream.stalled_ns)
                ),
            );
        }
        if self.truncated {
            terminal::print_kv(
                ctx,
                "Note",
                "span buffer overflowed — showing the most recent elements only; \
                 raise --limit or trace fewer elements for a complete trace",
            );
        }
    }
}

/// Execute the `torvyn trace` command.
///
/// COLD PATH (setup), delegates to the runtime.
///
/// # Errors
/// - Same as `torvyn run`, plus a timeout if the flow does not finish.
pub async fn execute(
    args: &TraceArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<TraceResult>, CliError> {
    let manifest_path = &args.manifest;

    if !manifest_path.exists() {
        return Err(CliError::Config {
            detail: format!("Manifest not found: {}", manifest_path.display()),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Run this command from a Torvyn project directory.".into(),
        });
    }

    let manifest_content = std::fs::read_to_string(manifest_path).map_err(|e| CliError::Io {
        detail: e.to_string(),
        path: Some(manifest_path.display().to_string()),
    })?;

    let manifest = torvyn_config::ComponentManifest::from_toml_str(
        &manifest_content,
        manifest_path.to_str().unwrap_or("Torvyn.toml"),
    )
    .map_err(|errors| CliError::Config {
        detail: format!("Manifest has {} error(s)", errors.len()),
        file: Some(manifest_path.display().to_string()),
        suggestion: "Run `torvyn check` first.".into(),
    })?;

    let flow_name = args
        .flow
        .clone()
        .or_else(|| manifest.flow.keys().next().cloned())
        .ok_or_else(|| crate::commands::no_flow_defined(manifest_path, &manifest))?;

    // Node count sizes the span buffer: one span per stage per element. The
    // manifest keeps flow tables as raw TOML, so this reads the node table
    // directly and falls back to a single stage if the shape is unexpected —
    // a wrong estimate costs buffer capacity, not correctness.
    let stage_count = manifest
        .flow
        .get(&flow_name)
        .and_then(|flow| flow.get("nodes"))
        .and_then(|nodes| nodes.as_table())
        .map_or(1, |nodes| nodes.len())
        .max(1);

    let limit_label = args
        .limit
        .map_or_else(|| "no limit".into(), |l| format!("limit: {l} elements"));

    if !ctx.quiet && ctx.format == crate::cli::OutputFormat::Human {
        eprintln!("▶ Tracing flow \"{flow_name}\" ({limit_label})");
    }

    // Reject the flags this command cannot honour. Accepting a flag and
    // ignoring it leaves a user believing they got something they did not,
    // which is the failure mode this command was rebuilt to remove.
    if args.show_buffers {
        return Err(CliError::Config {
            detail: "--show-buffers is not implemented".into(),
            file: None,
            suggestion: "Buffer content snapshots require the runtime to retain payload \
                         bytes after an element is consumed, which it deliberately does not \
                         do — buffers return to the pool as soon as a stage releases them. \
                         Re-run without --show-buffers; per-element copy counts and byte \
                         totals are in the summary."
                .into(),
        });
    }
    if args.input.is_some() {
        return Err(CliError::Config {
            detail: "--input is not implemented".into(),
            file: None,
            suggestion: "Overriding a source's input is not wired up for any command yet. \
                         Set the source's `config` in the flow's node table in Torvyn.toml \
                         and re-run without --input."
                .into(),
        });
    }

    // Every flow this command starts must be traced, and the span buffer has
    // to be large enough to hold one span per stage per element.
    let collector_config = trace_collector_config(args.limit, stage_count);
    let span_capacity = collector_config.tracing.ring_buffer_capacity;

    let mut host = torvyn_host::HostBuilder::new()
        .with_config_file(manifest_path)
        .with_collector_config(collector_config)
        .build()
        .await
        .map_err(|e| CliError::Runtime {
            detail: format!("Failed to initialize host: {e}"),
            context: None,
        })?;

    let flow_id = host
        .start_flow(&flow_name)
        .await
        .map_err(|e| CliError::Runtime {
            detail: format!("Failed to start flow: {e}"),
            context: Some(flow_name.clone()),
        })?;

    // Wait for the flow to drain. `TorvynHost::run` would start every flow in
    // the manifest — including this one a second time — so the wait is done
    // directly here.
    wait_for_flow(&host, flow_id).await?;

    // Read the run's spans and metrics *before* shutting down: shutdown
    // deregisters the flow, which releases both.
    // Ask the buffer whether it wrapped *before* draining it: draining resets
    // the write position, and inferring overflow from the drained length
    // would misreport a run that exactly filled the buffer.
    let truncated = host.observability().flow_spans_wrapped(flow_id);
    let spans = host.observability().drain_flow_spans(flow_id);
    let snapshot = host.observability().snapshot(flow_id);
    let trace_id = host
        .observability()
        .flow_trace_context(flow_id)
        .map(|ctx| ctx.trace_id.to_string())
        .unwrap_or_default();
    let stage_names = stage_lookup(&host, flow_id).await;

    host.shutdown().await.ok();

    let result = build_result(TracedRun {
        flow_name,
        trace_id,
        truncated,
        limit: args.limit,
        show_backpressure: args.show_backpressure,
        spans: &spans,
        stage_names: &stage_names,
        snapshot: snapshot.as_ref(),
    });

    if let Some(path) = &args.output_trace {
        write_trace(path, args.trace_format, &result)?;
        if !ctx.quiet && ctx.format == crate::cli::OutputFormat::Human {
            eprintln!("▶ Trace written to {}", path.display());
        }
    } else if args.trace_format != TraceFormat::Pretty {
        let rendered = render_trace(args.trace_format, &result)?;
        println!("{rendered}");
    }

    let mut warnings = Vec::new();
    if result.traces.is_empty() {
        warnings.push(
            "No spans were recorded. The flow produced no elements, or it completed before \
             any component was invoked."
                .to_owned(),
        );
    }
    if truncated {
        warnings.push(format!(
            "The span buffer holds {span_capacity} spans and the run filled it; older elements \
             were evicted. Pass --limit to size the buffer for the run."
        ));
    }

    Ok(CommandResult {
        success: true,
        command: "trace".into(),
        data: result,
        warnings,
    })
}

/// Build the collector configuration for a traced run.
fn trace_collector_config(
    limit: Option<u64>,
    stage_count: usize,
) -> torvyn_observability::ObservabilityConfig {
    let base = torvyn_observability::ObservabilityConfig::default();
    let capacity = span_capacity_for(limit, stage_count);
    torvyn_observability::ObservabilityConfig {
        level: ObservabilityLevel::Diagnostic,
        tracing: torvyn_observability::config::TracingConfig {
            // Trace every flow this command starts: a user asking for a trace
            // and receiving nothing because head sampling declined the flow
            // would be indistinguishable from a broken pipeline.
            sample_rate: 1.0,
            ring_buffer_capacity: capacity,
            ..base.tracing
        },
        ..base
    }
}

/// Choose a span-buffer capacity: one span per stage per element, rounded up
/// to a power of two, bounded at both ends.
///
/// `--limit` can only *raise* the capacity, never lower it. It bounds how
/// many elements are reported, not how many the source produces — the source
/// decides that — so sizing the buffer down to the limit would let a longer
/// run wrap and evict the earliest elements, and the report would then show
/// the *last* N elements while claiming to show N. Keeping the floor means a
/// small limit still captures the run from its start.
///
/// The buffer's contract requires a power of two of at least 8.
fn span_capacity_for(limit: Option<u64>, stage_count: usize) -> usize {
    let Some(limit) = limit else {
        return DEFAULT_SPAN_CAPACITY;
    };
    let wanted = limit.saturating_mul(stage_count.max(1) as u64);
    let wanted = usize::try_from(wanted).unwrap_or(MAX_SPAN_CAPACITY);
    wanted
        .clamp(DEFAULT_SPAN_CAPACITY, MAX_SPAN_CAPACITY)
        .checked_next_power_of_two()
        .unwrap_or(MAX_SPAN_CAPACITY)
        .min(MAX_SPAN_CAPACITY)
}

/// Poll until the traced flow reaches a terminal state.
async fn wait_for_flow(host: &torvyn_host::TorvynHost, flow_id: FlowId) -> Result<(), CliError> {
    let deadline = tokio::time::Instant::now() + FLOW_COMPLETION_TIMEOUT;
    loop {
        match host.flow_state(flow_id).await {
            // A reaped flow is gone from the reactor's table, which only
            // happens after it terminated.
            Err(_) => return Ok(()),
            Ok(state) if state.is_terminal() => return Ok(()),
            Ok(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(CliError::Runtime {
                detail: format!(
                    "Flow did not finish within {}s. A source that never completes cannot be \
                     traced to completion; use --limit or a finite source.",
                    FLOW_COMPLETION_TIMEOUT.as_secs()
                ),
                context: Some(format!("flow {flow_id}")),
            });
        }
        tokio::time::sleep(FLOW_POLL_INTERVAL).await;
    }
}

/// Map each component identity to its topology node name and operation label.
async fn stage_lookup(
    host: &torvyn_host::TorvynHost,
    flow_id: FlowId,
) -> BTreeMap<u64, (String, String)> {
    host.list_flows()
        .await
        .into_iter()
        .find(|record| record.flow_id == flow_id)
        .map(|record| {
            record
                .stages
                .iter()
                .map(|stage| {
                    (
                        stage.component_id.as_u64(),
                        (stage.name.clone(), stage.operation().to_owned()),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Everything one traced run produced, ready to be shaped into a report.
struct TracedRun<'a> {
    flow_name: String,
    trace_id: String,
    /// Whether the span buffer wrapped, evicting earlier elements.
    truncated: bool,
    /// `--limit`, which bounds how many elements are reported.
    limit: Option<u64>,
    /// `--show-backpressure`, which adds per-stream detail.
    show_backpressure: bool,
    spans: &'a [CompactSpanRecord],
    /// Component id to `(node name, operation)`.
    stage_names: &'a BTreeMap<u64, (String, String)>,
    snapshot: Option<&'a FlowMetricsSnapshot>,
}

/// Assemble the command result from the run's spans and metrics.
fn build_result(run: TracedRun<'_>) -> TraceResult {
    let TracedRun {
        flow_name,
        trace_id,
        truncated,
        limit,
        show_backpressure,
        spans,
        stage_names,
        snapshot,
    } = run;
    // Group by the element's origin sequence, which every stage's span
    // carries, so one element's spans land together.
    let mut by_element: BTreeMap<u64, Vec<&CompactSpanRecord>> = BTreeMap::new();
    for span in spans {
        by_element
            .entry(span.element_sequence)
            .or_default()
            .push(span);
    }

    let mut traces: Vec<ElementTrace> = by_element
        .into_iter()
        .map(|(element_id, mut records)| {
            records.sort_by_key(|record| record.start_ns);
            let total_ns: u64 = records.iter().map(|record| record.duration_ns()).sum();
            ElementTrace {
                element_id,
                spans: records
                    .iter()
                    .map(|record| {
                        let component = record.component_id.as_u64();
                        let (name, operation) =
                            stage_names.get(&component).cloned().unwrap_or_else(|| {
                                (format!("component-{component}"), "invoke".to_owned())
                            });
                        ComponentSpan {
                            component: name,
                            operation,
                            duration_us: ns_to_us(record.duration_ns()),
                            span_id: record.span_id.to_string(),
                            parent_span_id: record.parent_span_id.to_string(),
                            start_unix_nano: record.start_ns,
                            end_unix_nano: record.end_ns,
                            error: record.is_error(),
                        }
                    })
                    .collect(),
                total_latency_us: ns_to_us(total_ns),
            }
        })
        .collect();

    let elements_traced = traces.len() as u64;

    // `--limit` bounds the report, not the run: a source decides how many
    // elements it produces. Traces are ordered by origin sequence, so
    // truncating keeps the *first* N elements.
    if let Some(limit) = limit {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        traces.truncate(limit);
    }

    TraceResult {
        elements_traced,
        elements_shown: traces.len() as u64,
        avg_latency_us: snapshot.map_or(0.0, |s| s.latency_mean_ns / 1000.0),
        p50_latency_us: snapshot.map_or(0.0, |s| ns_to_us(s.latency_p50_ns)),
        p99_latency_us: snapshot.map_or(0.0, |s| ns_to_us(s.latency_p99_ns)),
        total_copies: snapshot.map_or(0, |s| s.copies_total),
        total_copy_bytes: snapshot.map_or(0, |s| s.copy_bytes_total),
        backpressure_events: snapshot.map_or(0, |s| {
            s.streams.iter().map(|st| st.backpressure_events).sum()
        }),
        backpressure_by_stream: if show_backpressure {
            snapshot.map_or_else(Vec::new, |s| {
                s.streams
                    .iter()
                    .map(|st| StreamBackpressure {
                        stream_id: st.stream_id.as_u64(),
                        events: st.backpressure_events,
                        queue_depth_peak: st.queue_depth_peak,
                        stalled_ns: st.backpressure_duration_ns,
                    })
                    .collect()
            })
        } else {
            Vec::new()
        },
        truncated,
        trace_id,
        traces,
        flow_name,
    }
}

/// Render the trace in the requested machine-readable format.
fn render_trace(format: TraceFormat, result: &TraceResult) -> Result<String, CliError> {
    let value = match format {
        TraceFormat::Pretty | TraceFormat::Json => serde_json::to_string_pretty(result),
        TraceFormat::Otlp => serde_json::to_string_pretty(&otlp_document(result)),
    };
    value.map_err(|e| CliError::Runtime {
        detail: format!("Failed to serialize trace: {e}"),
        context: None,
    })
}

/// Write the trace to `path` in the requested format.
fn write_trace(path: &Path, format: TraceFormat, result: &TraceResult) -> Result<(), CliError> {
    let rendered = render_trace(format, result)?;
    std::fs::write(path, rendered).map_err(|e| CliError::Io {
        detail: e.to_string(),
        path: Some(path.display().to_string()),
    })
}

/// Build an OTLP `ExportTraceServiceRequest` body from the trace.
///
/// The result is the JSON an OTLP/HTTP collector accepts at
/// `POST /v1/traces` with `Content-Type: application/json`.
fn otlp_document(result: &TraceResult) -> torvyn_observability::export::otlp::OtlpExportRequest {
    use torvyn_observability::export::otlp::{
        build_export_request, OtlpAttribute, OtlpSpan, OtlpStatus, OtlpValue,
    };

    let spans = result
        .traces
        .iter()
        .flat_map(|trace| {
            trace.spans.iter().map(move |span| OtlpSpan {
                trace_id: result.trace_id.clone(),
                span_id: span.span_id.clone(),
                parent_span_id: span.parent_span_id.clone(),
                name: format!("{}/{}", span.component, span.operation),
                // Absolute epoch timestamps, which is what an OTLP consumer
                // needs to place the span on a timeline alongside spans from
                // other systems.
                start_time_unix_nano: span.start_unix_nano,
                end_time_unix_nano: span.end_unix_nano,
                // OTLP status codes: 0 = Unset, 1 = Ok, 2 = Error.
                status: OtlpStatus {
                    code: if span.error { 2 } else { 1 },
                    message: None,
                },
                attributes: vec![
                    OtlpAttribute {
                        key: "torvyn.element.sequence".into(),
                        value: OtlpValue {
                            int_value: Some(trace.element_id as i64),
                            string_value: None,
                        },
                    },
                    OtlpAttribute {
                        key: "torvyn.component.name".into(),
                        value: OtlpValue {
                            int_value: None,
                            string_value: Some(span.component.clone()),
                        },
                    },
                    OtlpAttribute {
                        key: "torvyn.flow.name".into(),
                        value: OtlpValue {
                            int_value: None,
                            string_value: Some(result.flow_name.clone()),
                        },
                    },
                ],
            })
        })
        .collect();

    build_export_request(spans)
}

/// Nanoseconds to microseconds.
#[inline]
fn ns_to_us(ns: u64) -> f64 {
    ns as f64 / 1000.0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::{ComponentId, SpanId};

    fn span(component: u64, sequence: u64, start_ns: u64, end_ns: u64) -> CompactSpanRecord {
        CompactSpanRecord {
            span_id: SpanId::new([component as u8; 8]),
            parent_span_id: SpanId::new([0xAB; 8]),
            component_id: ComponentId::new(component),
            start_ns,
            end_ns,
            status_code: 0,
            element_sequence: sequence,
        }
    }

    fn stage_names() -> BTreeMap<u64, (String, String)> {
        [
            (1, ("reader".to_owned(), "pull".to_owned())),
            (2, ("mapper".to_owned(), "process".to_owned())),
            (3, ("writer".to_owned(), "push".to_owned())),
        ]
        .into_iter()
        .collect()
    }

    fn run<'a>(
        spans: &'a [CompactSpanRecord],
        names: &'a BTreeMap<u64, (String, String)>,
        limit: Option<u64>,
    ) -> TraceResult {
        build_result(TracedRun {
            flow_name: "demo".to_owned(),
            trace_id: "0123456789abcdef0123456789abcdef".to_owned(),
            truncated: false,
            limit,
            show_backpressure: false,
            spans,
            stage_names: names,
            snapshot: None,
        })
    }

    #[test]
    fn spans_group_into_one_trace_per_element() {
        // Two elements through three stages, deliberately out of order so the
        // grouping cannot be relying on input order.
        let spans = vec![
            span(3, 0, 300, 380),
            span(1, 0, 100, 150),
            span(2, 2, 220, 260),
            span(2, 0, 200, 240),
            span(1, 2, 120, 170),
            span(3, 2, 320, 400),
        ];
        let names = stage_names();
        let result = run(&spans, &names, None);

        assert_eq!(result.elements_traced, 2);
        assert_eq!(result.elements_shown, 2);
        // Elements are ordered by origin sequence.
        assert_eq!(result.traces[0].element_id, 0);
        assert_eq!(result.traces[1].element_id, 2);

        // Each element's spans are ordered by start time and named from the
        // topology, so the tree reads source → processor → sink.
        let ops: Vec<&str> = result.traces[0]
            .spans
            .iter()
            .map(|s| s.operation.as_str())
            .collect();
        assert_eq!(ops, ["pull", "process", "push"]);
        let components: Vec<&str> = result.traces[0]
            .spans
            .iter()
            .map(|s| s.component.as_str())
            .collect();
        assert_eq!(components, ["reader", "mapper", "writer"]);

        // In-component total is the sum of the element's span durations:
        // 50 ns + 40 ns + 80 ns = 170 ns.
        assert!(
            (result.traces[0].total_latency_us - 0.170).abs() < 1e-9,
            "expected 0.170µs, got {}",
            result.traces[0].total_latency_us,
        );
    }

    #[test]
    fn limit_reports_the_first_elements_not_the_last() {
        let spans: Vec<_> = (0..5).map(|i| span(1, i, i * 100, i * 100 + 10)).collect();
        let names = stage_names();
        let result = run(&spans, &names, Some(2));

        assert_eq!(result.elements_traced, 5, "the whole run is still counted");
        assert_eq!(result.elements_shown, 2);
        assert_eq!(result.traces.len(), 2);
        assert_eq!(result.traces[0].element_id, 0);
        assert_eq!(result.traces[1].element_id, 1);
    }

    #[test]
    fn unknown_components_fall_back_to_their_id() {
        let spans = vec![span(9, 0, 0, 50)];
        let names = stage_names();
        let result = run(&spans, &names, None);
        assert_eq!(result.traces[0].spans[0].component, "component-9");
        assert_eq!(result.traces[0].spans[0].operation, "invoke");
    }

    #[test]
    fn empty_run_produces_no_traces_and_no_invented_numbers() {
        let names = stage_names();
        let result = run(&[], &names, None);
        assert_eq!(result.elements_traced, 0);
        assert!(result.traces.is_empty());
        assert_eq!(result.total_copies, 0);
        assert_eq!(result.avg_latency_us, 0.0);
    }

    #[test]
    fn limit_only_raises_span_capacity() {
        // A small limit must not shrink the buffer below the default: the
        // source decides how many elements it produces, and a shrunken buffer
        // would wrap and evict the earliest elements.
        assert_eq!(span_capacity_for(Some(1), 3), DEFAULT_SPAN_CAPACITY);
        assert_eq!(span_capacity_for(None, 3), DEFAULT_SPAN_CAPACITY);

        // A large limit raises it, rounded up to a power of two.
        let large = span_capacity_for(Some(100_000), 3);
        assert!(large >= 100_000 * 3, "capacity must cover the request");
        assert!(
            large.is_power_of_two(),
            "the ring buffer requires a power of two"
        );
        assert!(large <= MAX_SPAN_CAPACITY);

        // An absurd limit is clamped rather than attempting a huge allocation.
        assert_eq!(span_capacity_for(Some(u64::MAX), 8), MAX_SPAN_CAPACITY);
    }

    #[test]
    fn collector_config_traces_every_flow_at_diagnostic_level() {
        let config = trace_collector_config(Some(10), 3);
        assert_eq!(config.level, ObservabilityLevel::Diagnostic);
        assert!(
            (config.tracing.sample_rate - 1.0).abs() < f64::EPSILON,
            "trace must not let head sampling drop the flow the user asked to trace",
        );
        assert!(config.validate().is_ok(), "the collector must accept it");
    }

    #[test]
    fn otlp_document_carries_absolute_timestamps_and_identity() {
        let spans = vec![span(
            1,
            0,
            1_700_000_000_000_000_000,
            1_700_000_000_000_050_000,
        )];
        let names = stage_names();
        let result = run(&spans, &names, None);
        let doc = otlp_document(&result);

        let exported = &doc.resource_spans[0].scope_spans[0].spans;
        assert_eq!(exported.len(), 1);
        let s = &exported[0];
        assert_eq!(s.trace_id.len(), 32, "W3C trace ids are 16 bytes of hex");
        assert_eq!(s.span_id.len(), 16, "W3C span ids are 8 bytes of hex");
        assert_eq!(s.parent_span_id.len(), 16);
        assert_eq!(s.name, "reader/pull");
        assert_eq!(s.start_time_unix_nano, 1_700_000_000_000_000_000);
        assert_eq!(s.end_time_unix_nano, 1_700_000_000_000_050_000);
        assert_eq!(s.status.code, 1, "OTLP status: 1 = Ok");
        assert!(s
            .attributes
            .iter()
            .any(|a| a.key == "torvyn.element.sequence"));

        // The document must round-trip through serde as the OTLP/HTTP JSON
        // body a collector accepts.
        let json = serde_json::to_value(&doc).expect("OTLP document must serialize");
        assert!(json["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["traceId"].is_string());
    }

    #[test]
    fn failed_invocations_are_marked_in_both_renderings() {
        let mut failed = span(2, 0, 0, 100);
        failed.status_code = 1;
        let names = stage_names();
        let result = run(&[failed], &names, None);

        assert!(result.traces[0].spans[0].error);
        let doc = otlp_document(&result);
        assert_eq!(
            doc.resource_spans[0].scope_spans[0].spans[0].status.code, 2,
            "OTLP status: 2 = Error",
        );
    }
}
