//! `torvyn bench` — benchmark a pipeline.
//!
//! Runs a pipeline under sustained load with warmup period, then produces
//! a performance report with latency percentiles, throughput, resource
//! usage, and scheduling statistics.

use crate::cli::BenchArgs;
use crate::commands::run::parse_duration;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;
use torvyn_observability::FlowMetricsSnapshot;

/// Default stream queue capacity (the runtime's `StreamConfig` default).
const DEFAULT_QUEUE_CAPACITY: u64 = 64;

/// Complete benchmark report.
#[derive(Debug, Serialize)]
pub struct BenchReport {
    /// Flow that was benchmarked.
    pub flow_name: String,
    /// Warmup duration in seconds.
    pub warmup_secs: f64,
    /// Measurement duration in seconds.
    pub measurement_secs: f64,
    /// Throughput section.
    pub throughput: ThroughputReport,
    /// Latency section.
    pub latency: LatencyReport,
    /// Per-component latency.
    pub per_component: Vec<ComponentBenchRow>,
    /// Resource section.
    pub resources: ResourceReport,
    /// Scheduling section.
    pub scheduling: SchedulingReport,
    /// File where results were saved (if any).
    pub saved_to: Option<PathBuf>,
}

/// Throughput metrics.
#[derive(Debug, Serialize)]
pub struct ThroughputReport {
    /// Elements processed per second.
    pub elements_per_sec: f64,
    /// Bytes processed per second.
    pub bytes_per_sec: f64,
}

/// Latency percentile metrics.
#[derive(Debug, Serialize)]
pub struct LatencyReport {
    /// 50th percentile latency in microseconds.
    pub p50_us: f64,
    /// 90th percentile latency in microseconds.
    pub p90_us: f64,
    /// 95th percentile latency in microseconds.
    pub p95_us: f64,
    /// 99th percentile latency in microseconds.
    pub p99_us: f64,
    /// 99.9th percentile latency in microseconds.
    pub p999_us: f64,
    /// Maximum latency in microseconds.
    pub max_us: f64,
}

/// Per-component latency in a benchmark.
#[derive(Debug, Serialize)]
pub struct ComponentBenchRow {
    /// Component name.
    pub component: String,
    /// 50th percentile latency in microseconds.
    pub p50_us: f64,
    /// 99th percentile latency in microseconds.
    pub p99_us: f64,
}

/// Resource usage metrics.
#[derive(Debug, Serialize)]
pub struct ResourceReport {
    /// Total buffer allocations.
    pub buffer_allocs: u64,
    /// Buffer pool reuse percentage.
    pub pool_reuse_pct: f64,
    /// Total buffer copies.
    pub total_copies: u64,
    /// Peak memory usage in bytes.
    pub peak_memory_bytes: u64,
}

/// Scheduling metrics.
#[derive(Debug, Serialize)]
pub struct SchedulingReport {
    /// Total scheduler wakeups.
    pub total_wakeups: u64,
    /// Number of backpressure events.
    pub backpressure_events: u64,
    /// Peak queue depth observed.
    pub queue_peak: u64,
    /// Configured queue capacity.
    pub queue_capacity: u64,
}

impl HumanRenderable for BenchReport {
    fn render_human(&self, ctx: &OutputContext) {
        terminal::print_header(ctx, "Throughput");
        terminal::print_kv(
            ctx,
            "Elements/s",
            &format!("{:.0}", self.throughput.elements_per_sec),
        );
        terminal::print_kv(
            ctx,
            "Bytes/s",
            &terminal::format_bytes(self.throughput.bytes_per_sec as u64),
        );

        terminal::print_header(ctx, "Latency (µs)");
        terminal::print_kv(ctx, "p50", &format!("{:.1}", self.latency.p50_us));
        terminal::print_kv(ctx, "p90", &format!("{:.1}", self.latency.p90_us));
        terminal::print_kv(ctx, "p95", &format!("{:.1}", self.latency.p95_us));
        terminal::print_kv(ctx, "p99", &format!("{:.1}", self.latency.p99_us));
        terminal::print_kv(ctx, "p999", &format!("{:.1}", self.latency.p999_us));
        terminal::print_kv(ctx, "max", &format!("{:.1}", self.latency.max_us));

        if !self.per_component.is_empty() {
            terminal::print_header(ctx, "Per-Component Latency (µs, p50)");
            for row in &self.per_component {
                terminal::print_kv(ctx, &row.component, &format!("{:.1}", row.p50_us));
            }
        }

        terminal::print_header(ctx, "Resources");
        terminal::print_kv(
            ctx,
            "Buffer allocs",
            &format!("{}", self.resources.buffer_allocs),
        );
        terminal::print_kv(
            ctx,
            "Pool reuse rate",
            &format!("{:.1}%", self.resources.pool_reuse_pct),
        );
        terminal::print_kv(
            ctx,
            "Total copies",
            &format!("{}", self.resources.total_copies),
        );
        terminal::print_kv(
            ctx,
            "Peak memory",
            &terminal::format_bytes(self.resources.peak_memory_bytes),
        );

        terminal::print_header(ctx, "Scheduling");
        terminal::print_kv(
            ctx,
            "Total wakeups",
            &format!("{}", self.scheduling.total_wakeups),
        );
        terminal::print_kv(
            ctx,
            "Backpressure events",
            &format!("{}", self.scheduling.backpressure_events),
        );
        terminal::print_kv(
            ctx,
            "Queue peak",
            &format!(
                "{} / {}",
                self.scheduling.queue_peak, self.scheduling.queue_capacity
            ),
        );

        if let Some(path) = &self.saved_to {
            eprintln!();
            eprintln!("  Result saved to: {}", path.display());
        }
    }
}

/// Execute the `torvyn bench` command.
///
/// COLD PATH (setup), delegates to runtime.
///
/// # Postconditions
/// - Returns `BenchReport` with full metrics.
/// - Saves report to `.torvyn/bench/` with ISO 8601 timestamp filename.
///
/// # Errors
/// - Same as `torvyn run`.
pub async fn execute(
    args: &BenchArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<BenchReport>, CliError> {
    let manifest_path = &args.manifest;

    if !manifest_path.exists() {
        return Err(CliError::Config {
            detail: format!("Manifest not found: {}", manifest_path.display()),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Run this command from a Torvyn project directory.".into(),
        });
    }

    let warmup_dur = parse_duration(&args.warmup).map_err(|e| CliError::Config {
        detail: format!("Invalid warmup duration: {e}"),
        file: None,
        suggestion: "Use a duration like '2s' or '5s'.".into(),
    })?;

    let bench_dur = parse_duration(&args.duration).map_err(|e| CliError::Config {
        detail: format!("Invalid benchmark duration: {e}"),
        file: None,
        suggestion: "Use a duration like '10s' or '30s'.".into(),
    })?;

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
        .ok_or_else(|| CliError::Config {
            detail: "No flow defined in manifest".into(),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Add a [flow.*] section.".into(),
        })?;

    if !ctx.quiet && ctx.format == crate::cli::OutputFormat::Human {
        eprintln!(
            "▶ Benchmarking flow \"{}\" (warmup: {:.0}s, duration: {:.0}s)",
            flow_name,
            warmup_dur.as_secs_f64(),
            bench_dur.as_secs_f64(),
        );
    }

    // Initialize host with metrics collection enabled
    let obs_config = torvyn_config::ObservabilityConfig {
        metrics_enabled: true,
        ..Default::default()
    };

    let mut host = torvyn_host::HostBuilder::new()
        .with_config_file(manifest_path)
        .with_observability_config(obs_config)
        .build()
        .await
        .map_err(|e| CliError::Runtime {
            detail: format!("Failed to initialize host: {e}"),
            context: None,
        })?;

    // Start the flow.
    let flow_id = host
        .start_flow(&flow_name)
        .await
        .map_err(|e| CliError::Runtime {
            detail: format!("Failed to start flow: {e}"),
            context: Some(flow_name.clone()),
        })?;

    // Warmup phase — let the pipeline reach steady state, then snapshot so the
    // measurement excludes warmup.
    tokio::time::sleep(warmup_dur).await;
    let warmup_snapshot = host.observability().snapshot(flow_id);

    // Measurement phase.
    let bench_start = Instant::now();
    tokio::time::sleep(bench_dur).await;
    let bench_elapsed = bench_start.elapsed();
    let end_snapshot = host.observability().snapshot(flow_id);

    // Shutdown.
    host.shutdown().await.ok();

    // Build the report from the metrics recorded over the measurement window.
    // Counters (elements, copies, bytes) are the delta after warmup; latency
    // percentiles and peaks come from the end snapshot.
    let report = match (warmup_snapshot, end_snapshot) {
        (Some(start_snap), Some(end_snap)) => build_bench_report(
            flow_name.clone(),
            warmup_dur.as_secs_f64(),
            bench_elapsed.as_secs_f64(),
            &torvyn_observability::metrics::delta(&start_snap, &end_snap),
        ),
        _ => empty_bench_report(
            flow_name.clone(),
            warmup_dur.as_secs_f64(),
            bench_elapsed.as_secs_f64(),
        ),
    };

    // Save report to .torvyn/bench/
    let project_dir = manifest_path.parent().unwrap_or(std::path::Path::new("."));
    let bench_dir = project_dir.join(".torvyn").join("bench");
    std::fs::create_dir_all(&bench_dir).ok();

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let report_path = bench_dir.join(format!("{timestamp}.json"));

    let saved_to = if let Ok(json) = serde_json::to_string_pretty(&report) {
        if std::fs::write(&report_path, &json).is_ok() {
            Some(report_path)
        } else {
            None
        }
    } else {
        None
    };

    let mut final_report = report;
    final_report.saved_to = saved_to;

    Ok(CommandResult {
        success: true,
        command: "bench".into(),
        data: final_report,
        warnings: vec![],
    })
}

/// Assemble a benchmark report from the metrics recorded over the measurement
/// window.
///
/// `measured` is the delta between the post-warmup and end snapshots: counters
/// (elements, copies, bytes) are windowed to exclude warmup, while latency
/// percentiles, per-component stats, and stream peaks are carried from the end
/// snapshot.
///
/// COLD PATH — called once when a benchmark finishes.
fn build_bench_report(
    flow_name: String,
    warmup_secs: f64,
    measurement_secs: f64,
    measured: &FlowMetricsSnapshot,
) -> BenchReport {
    let per_sec = |n: u64| {
        if measurement_secs > 0.0 {
            n as f64 / measurement_secs
        } else {
            0.0
        }
    };
    let ns_to_us = |ns: u64| ns as f64 / 1_000.0;

    let per_component = measured
        .components
        .iter()
        .map(|c| ComponentBenchRow {
            component: format!("component {}", c.component_id.as_u64()),
            p50_us: ns_to_us(c.processing_time_p50_ns),
            p99_us: ns_to_us(c.processing_time_p99_ns),
        })
        .collect();

    let peak_memory_bytes = measured
        .components
        .iter()
        .map(|c| c.memory_peak)
        .max()
        .unwrap_or(0);
    let backpressure_events: u64 = measured.streams.iter().map(|s| s.backpressure_events).sum();
    let queue_peak = measured
        .streams
        .iter()
        .map(|s| s.queue_depth_peak)
        .max()
        .unwrap_or(0);

    BenchReport {
        flow_name,
        warmup_secs,
        measurement_secs,
        throughput: ThroughputReport {
            elements_per_sec: per_sec(measured.elements_total),
            bytes_per_sec: per_sec(measured.copy_bytes_total),
        },
        latency: LatencyReport {
            p50_us: ns_to_us(measured.latency_p50_ns),
            p90_us: ns_to_us(measured.latency_p90_ns),
            p95_us: ns_to_us(measured.latency_p95_ns),
            p99_us: ns_to_us(measured.latency_p99_ns),
            p999_us: ns_to_us(measured.latency_p999_ns),
            max_us: ns_to_us(measured.latency_max_ns),
        },
        per_component,
        resources: ResourceReport {
            // Buffer-pool allocation and reuse counters are not yet surfaced by
            // the resource manager; reported as 0 until that inspection lands.
            buffer_allocs: 0,
            pool_reuse_pct: 0.0,
            total_copies: measured.copies_total,
            peak_memory_bytes,
        },
        scheduling: SchedulingReport {
            // Scheduler wakeup counts are not yet surfaced by the reactor.
            total_wakeups: 0,
            backpressure_events,
            queue_peak,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
        },
        saved_to: None,
    }
}

/// A zero-valued report, used only when no metrics snapshot is available (for
/// example if the flow failed to register). COLD PATH.
fn empty_bench_report(flow_name: String, warmup_secs: f64, measurement_secs: f64) -> BenchReport {
    BenchReport {
        flow_name,
        warmup_secs,
        measurement_secs,
        throughput: ThroughputReport {
            elements_per_sec: 0.0,
            bytes_per_sec: 0.0,
        },
        latency: LatencyReport {
            p50_us: 0.0,
            p90_us: 0.0,
            p95_us: 0.0,
            p99_us: 0.0,
            p999_us: 0.0,
            max_us: 0.0,
        },
        per_component: vec![],
        resources: ResourceReport {
            buffer_allocs: 0,
            pool_reuse_pct: 0.0,
            total_copies: 0,
            peak_memory_bytes: 0,
        },
        scheduling: SchedulingReport {
            total_wakeups: 0,
            backpressure_events: 0,
            queue_peak: 0,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
        },
        saved_to: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_observability::metrics::{ComponentMetricsSnapshot, StreamMetricsSnapshot};
    use torvyn_types::{ComponentId, FlowId, StreamId};

    /// A `measured` snapshot (a delta) with distinctive values in every field
    /// the report maps.
    fn sample_measured() -> FlowMetricsSnapshot {
        let component = |id: u64, p50: u64, p99: u64, mem_peak: u64| ComponentMetricsSnapshot {
            component_id: ComponentId::new(id),
            invocations: 10,
            errors: 0,
            processing_time_p50_ns: p50,
            processing_time_p95_ns: p50,
            processing_time_p99_ns: p99,
            processing_time_mean_ns: p50 as f64,
            fuel_consumed: 0,
            memory_current: 0,
            memory_peak: mem_peak,
        };
        let stream = |id: u64, bp: u64, peak: u64| StreamMetricsSnapshot {
            stream_id: StreamId::new(id),
            elements: 100,
            queue_depth: 0,
            queue_depth_peak: peak,
            backpressure_events: bp,
            backpressure_duration_ns: 0,
        };
        FlowMetricsSnapshot {
            flow_id: FlowId::new(1),
            elements_total: 200,
            errors_total: 0,
            copies_total: 800,
            copy_bytes_total: 1_600,
            latency_p50_ns: 1_000,
            latency_p90_ns: 2_000,
            latency_p95_ns: 3_000,
            latency_p99_ns: 4_000,
            latency_p999_ns: 5_000,
            latency_min_ns: 500,
            latency_max_ns: 6_000,
            latency_mean_ns: 1_500.0,
            components: vec![
                component(1, 500, 1_500, 1_000),
                component(2, 700, 1_700, 2_000),
            ],
            streams: vec![stream(0, 3, 10), stream(1, 2, 20)],
        }
    }

    #[test]
    fn test_build_bench_report_maps_all_metrics() {
        let measured = sample_measured();
        let report = build_bench_report("bench-flow".into(), 2.0, 2.0, &measured);

        // Throughput: counters over a 2s window.
        assert_eq!(report.throughput.elements_per_sec, 100.0); // 200 / 2s
        assert_eq!(report.throughput.bytes_per_sec, 800.0); // 1600 / 2s

        // Flow-level latency: ns -> µs.
        assert_eq!(report.latency.p50_us, 1.0);
        assert_eq!(report.latency.p90_us, 2.0);
        assert_eq!(report.latency.p95_us, 3.0);
        assert_eq!(report.latency.p99_us, 4.0);
        assert_eq!(report.latency.p999_us, 5.0);
        assert_eq!(report.latency.max_us, 6.0);

        // Per-component rows, in order, with processing-time percentiles.
        assert_eq!(report.per_component.len(), 2);
        assert_eq!(report.per_component[0].component, "component 1");
        assert_eq!(report.per_component[0].p50_us, 0.5);
        assert_eq!(report.per_component[0].p99_us, 1.5);
        assert_eq!(report.per_component[1].component, "component 2");

        // Resources: copies over the window; peak memory is the max across stages.
        assert_eq!(report.resources.total_copies, 800);
        assert_eq!(report.resources.peak_memory_bytes, 2_000);
        // Genuinely-untracked fields stay zero (never fabricated).
        assert_eq!(report.resources.buffer_allocs, 0);
        assert_eq!(report.resources.pool_reuse_pct, 0.0);

        // Scheduling: backpressure summed, queue peak maxed.
        assert_eq!(report.scheduling.backpressure_events, 5); // 3 + 2
        assert_eq!(report.scheduling.queue_peak, 20); // max(10, 20)
        assert_eq!(report.scheduling.queue_capacity, DEFAULT_QUEUE_CAPACITY);
        assert_eq!(report.scheduling.total_wakeups, 0);
    }

    #[test]
    fn test_build_bench_report_zero_duration_is_safe() {
        let measured = sample_measured();
        let report = build_bench_report("bench-flow".into(), 0.0, 0.0, &measured);
        // No division by zero — throughput collapses to 0.
        assert_eq!(report.throughput.elements_per_sec, 0.0);
        assert_eq!(report.throughput.bytes_per_sec, 0.0);
        // Latency percentiles are still reported (they are not rate-based).
        assert_eq!(report.latency.p50_us, 1.0);
    }

    #[test]
    fn test_empty_bench_report_is_all_zero() {
        let report = empty_bench_report("f".into(), 1.0, 5.0);
        assert_eq!(report.throughput.elements_per_sec, 0.0);
        assert_eq!(report.latency.p99_us, 0.0);
        assert!(report.per_component.is_empty());
        assert_eq!(report.resources.total_copies, 0);
        assert_eq!(report.scheduling.queue_capacity, DEFAULT_QUEUE_CAPACITY);
    }
}
