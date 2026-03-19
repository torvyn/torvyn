//! `torvyn trace` — run with full diagnostic tracing.
//!
//! Executes a pipeline with full tracing enabled, producing detailed
//! per-element, per-component diagnostic output.

use crate::cli::TraceArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;

/// Result of `torvyn trace`.
#[derive(Debug, Serialize)]
pub struct TraceResult {
    /// Number of elements traced.
    pub elements_traced: u64,
    /// Average latency in microseconds.
    pub avg_latency_us: f64,
    /// p50 latency in microseconds.
    pub p50_latency_us: f64,
    /// p99 latency in microseconds.
    pub p99_latency_us: f64,
    /// Total copies observed.
    pub total_copies: u64,
    /// Buffer reuse percentage.
    pub buffer_reuse_pct: f64,
    /// Number of backpressure events.
    pub backpressure_events: u64,
    /// Per-element traces.
    pub traces: Vec<ElementTrace>,
    /// Flow name.
    pub flow_name: String,
}

/// Trace for a single element through the pipeline.
#[derive(Debug, Serialize)]
pub struct ElementTrace {
    /// Element sequence number.
    pub element_id: u64,
    /// Per-component spans.
    pub spans: Vec<ComponentSpan>,
    /// Total end-to-end latency for this element in microseconds.
    pub total_latency_us: f64,
}

/// A single component's processing span for one element.
#[derive(Debug, Serialize)]
pub struct ComponentSpan {
    /// Component name.
    pub component: String,
    /// Operation type (e.g., "pull", "process", "push").
    pub operation: String,
    /// Duration in microseconds.
    pub duration_us: f64,
    /// Buffer description (e.g., "B001 (512B, created)").
    pub buffer_info: Option<String>,
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
                let connector = if i == 0 {
                    "┬─"
                } else if i == trace.spans.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };

                let buf_str = span
                    .buffer_info
                    .as_deref()
                    .map(|b| format!("  {b}"))
                    .unwrap_or_default();

                if i > 0 {
                    eprint!("          ");
                }
                eprintln!(
                    "{connector} {:<14} {:<8} {:.1}µs{buf_str}",
                    span.component, span.operation, span.duration_us,
                );
            }
            eprintln!("          total: {:.1}µs", trace.total_latency_us);
        }

        terminal::print_header(ctx, "Trace Summary");
        terminal::print_kv(ctx, "Elements traced", &format!("{}", self.elements_traced));
        terminal::print_kv(
            ctx,
            "Avg latency",
            &format!(
                "{:.1}µs (p50: {:.1}µs, p99: {:.1}µs)",
                self.avg_latency_us, self.p50_latency_us, self.p99_latency_us
            ),
        );
        terminal::print_kv(ctx, "Copies", &format!("{}", self.total_copies));
        terminal::print_kv(
            ctx,
            "Buffer reuse",
            &format!("{:.0}%", self.buffer_reuse_pct),
        );
        terminal::print_kv(
            ctx,
            "Backpressure",
            &format!("{} events", self.backpressure_events),
        );
    }
}

/// Execute the `torvyn trace` command.
///
/// COLD PATH (setup), delegates to runtime.
///
/// # Postconditions
/// - Returns `TraceResult` with per-element trace data.
///
/// # Errors
/// - Same as `torvyn run`, plus trace data collection failures.
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
        .ok_or_else(|| CliError::Config {
            detail: "No flow defined in manifest".into(),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Add a [flow.*] section or use --flow <name>.".into(),
        })?;

    let limit_label = args
        .limit
        .map(|l| format!("limit: {l} elements"))
        .unwrap_or_else(|| "no limit".into());

    if !ctx.quiet && ctx.format == crate::cli::OutputFormat::Human {
        eprintln!("▶ Tracing flow \"{flow_name}\" ({limit_label})");
    }

    let obs_config = torvyn_config::ObservabilityConfig {
        tracing_enabled: true,
        tracing_sample_rate: 1.0,
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

    let _flow_id = host
        .start_flow(&flow_name)
        .await
        .map_err(|e| CliError::Runtime {
            detail: format!("Failed to start flow: {e}"),
            context: Some(flow_name.clone()),
        })?;

    // Run the flow
    host.run().await.map_err(|e| CliError::Runtime {
        detail: format!("Pipeline execution failed: {e}"),
        context: Some(flow_name.clone()),
    })?;

    // Placeholder result — real trace data will come from observability APIs
    let result = TraceResult {
        elements_traced: 0,
        avg_latency_us: 0.0,
        p50_latency_us: 0.0,
        p99_latency_us: 0.0,
        total_copies: 0,
        buffer_reuse_pct: 0.0,
        backpressure_events: 0,
        traces: vec![],
        flow_name,
    };

    Ok(CommandResult {
        success: true,
        command: "trace".into(),
        data: result,
        warnings: vec![],
    })
}
