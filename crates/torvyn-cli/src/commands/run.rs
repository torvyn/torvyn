//! `torvyn run` — execute a pipeline locally.
//!
//! Instantiates the Torvyn host runtime, loads components, and runs
//! the pipeline. Displays real-time throughput and summary on exit.

use crate::cli::RunArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::time::{Duration, Instant};

/// Result of `torvyn run`.
#[derive(Debug, Serialize)]
pub struct RunResult {
    /// Total execution duration in seconds.
    pub duration_secs: f64,
    /// Total elements processed.
    pub elements_processed: u64,
    /// Average throughput in elements/second.
    pub throughput_elem_per_sec: f64,
    /// Total errors encountered.
    pub error_count: u64,
    /// Peak memory usage in bytes (across all components).
    pub peak_memory_bytes: u64,
    /// Flow name that was executed.
    pub flow_name: String,
    /// Number of components in the flow.
    pub component_count: usize,
    /// Number of edges in the flow.
    pub edge_count: usize,
}

impl HumanRenderable for RunResult {
    fn render_human(&self, ctx: &OutputContext) {
        terminal::print_header(ctx, "Summary");
        terminal::print_kv(ctx, "Duration", &format!("{:.2}s", self.duration_secs));
        terminal::print_kv(ctx, "Elements", &format!("{}", self.elements_processed));
        terminal::print_kv(
            ctx,
            "Throughput",
            &format!("{:.0} elem/s", self.throughput_elem_per_sec),
        );
        terminal::print_kv(ctx, "Errors", &format!("{}", self.error_count));
        terminal::print_kv(
            ctx,
            "Peak memory",
            &terminal::format_bytes(self.peak_memory_bytes),
        );
    }
}

/// Execute the `torvyn run` command.
///
/// COLD PATH (setup), then delegates to HOT PATH runtime.
///
/// # Preconditions
/// - Manifest file must exist.
/// - Components must be compiled (or builds are triggered implicitly).
///
/// # Postconditions
/// - Pipeline runs to completion (or until limit/timeout/Ctrl+C).
/// - Returns summary statistics.
///
/// # Errors
/// - [`CliError::Config`] if manifest is missing or invalid.
/// - [`CliError::Runtime`] if pipeline execution fails.
pub async fn execute(
    args: &RunArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<RunResult>, CliError> {
    let manifest_path = &args.manifest;

    if !manifest_path.exists() {
        return Err(CliError::Config {
            detail: format!("Manifest not found: {}", manifest_path.display()),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Run this command from a Torvyn project directory.".into(),
        });
    }

    // Parse manifest
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

    // Determine which flow to run
    let flow_name = args
        .flow
        .clone()
        .or_else(|| manifest.flow.keys().next().cloned())
        .ok_or_else(|| crate::commands::no_flow_defined(manifest_path, &manifest))?;

    // Reject the options this command cannot honour. Accepting a flag and
    // ignoring it leaves the user believing they got something they did not —
    // a bounded run, a redirected source — and the pipeline behaves as though
    // the flag were never typed.
    if let Some(unsupported) = unsupported_option(args) {
        return Err(unsupported);
    }

    // Parse timeout
    let timeout = args
        .timeout
        .as_ref()
        .map(|s| parse_duration(s))
        .transpose()
        .map_err(|e| CliError::Config {
            detail: format!("Invalid timeout: {e}"),
            file: None,
            suggestion: "Use a duration like '30s', '5m', or '1h'.".into(),
        })?;

    let spinner = ctx.spinner(&format!("Starting flow \"{flow_name}\"..."));

    let config_path = manifest_path.to_path_buf();
    let mut host = torvyn_host::HostBuilder::new()
        .with_config_file(&config_path)
        .build()
        .await
        .map_err(|e| CliError::Runtime {
            detail: format!("Failed to initialize host: {e}"),
            context: None,
        })?;

    if let Some(sp) = &spinner {
        sp.finish_and_clear();
    }

    // Start only the selected flow. `host.run()` is intentionally not used
    // here: it starts *every* configured flow (ignoring `--flow`) and would
    // double-start this one, since it is already running via `start_flow`.
    let flow_id = host
        .start_flow(&flow_name)
        .await
        .map_err(|e| CliError::Runtime {
            detail: format!("Failed to start flow \"{flow_name}\": {e}"),
            context: Some(flow_name.clone()),
        })?;

    if !ctx.quiet && ctx.format == crate::cli::OutputFormat::Human {
        eprintln!("▶ Running flow \"{}\"", flow_name);
        eprintln!();
    }

    let start = Instant::now();

    // Wait for the flow to reach a terminal state, stopping early on timeout or
    // Ctrl+C. Only this flow is active, so `wait_for_all_flows` waits for it.
    let ctrl_c = tokio::signal::ctrl_c();
    match timeout {
        Some(timeout_dur) => {
            tokio::select! {
                () = host.wait_for_all_flows() => {}
                _ = tokio::time::sleep(timeout_dur) => {}
                _ = ctrl_c => { eprintln!(); }
            }
        }
        None => {
            tokio::select! {
                () = host.wait_for_all_flows() => {}
                _ = ctrl_c => { eprintln!(); }
            }
        }
    }

    let elapsed = start.elapsed();

    // Graceful shutdown drains the flow if it was stopped early (timeout/Ctrl+C)
    // and is a no-op once it has already completed.
    host.shutdown().await.map_err(|e| CliError::Runtime {
        detail: format!("Graceful shutdown failed: {e}"),
        context: Some(flow_name.clone()),
    })?;

    // Report the flow's recorded metrics. The collector retains a flow's
    // metrics after it terminates, so the snapshot is available post-shutdown.
    let result = match host.observability().snapshot(flow_id) {
        Some(snapshot) => build_run_result(
            flow_name,
            elapsed,
            snapshot.elements_total,
            snapshot.errors_total,
            snapshot.components.len(),
            snapshot.streams.len(),
        ),
        None => build_run_result(flow_name, elapsed, 0, 0, 0, 0),
    };

    Ok(CommandResult {
        success: true,
        command: "run".into(),
        data: result,
        warnings: vec![],
    })
}

/// Assemble the run summary from a flow's recorded metrics.
///
/// COLD PATH — called once when a run finishes.
fn build_run_result(
    flow_name: String,
    duration: Duration,
    elements_processed: u64,
    error_count: u64,
    component_count: usize,
    edge_count: usize,
) -> RunResult {
    let duration_secs = duration.as_secs_f64();
    let throughput_elem_per_sec = if duration_secs > 0.0 {
        elements_processed as f64 / duration_secs
    } else {
        0.0
    };

    RunResult {
        duration_secs,
        elements_processed,
        throughput_elem_per_sec,
        error_count,
        // Per-flow peak memory is not yet tracked by the collector.
        peak_memory_bytes: 0,
        flow_name,
        component_count,
        edge_count,
    }
}

/// Parse a duration string like "30s", "5m", "1h".
///
/// COLD PATH.
///
/// # Postconditions
/// - Returns a [`Duration`] on success.
/// - Returns an error string on invalid format.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".into());
    }

    let (num_str, unit) = if let Some(n) = s.strip_suffix("ms") {
        (n, "ms")
    } else if let Some(n) = s.strip_suffix('s') {
        (n, "s")
    } else if let Some(n) = s.strip_suffix('m') {
        (n, "m")
    } else if let Some(n) = s.strip_suffix('h') {
        (n, "h")
    } else {
        return Err(format!(
            "unrecognized duration unit in \"{s}\". Use s, m, h, or ms."
        ));
    };

    let num: f64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in duration: \"{num_str}\""))?;

    if num < 0.0 {
        return Err("duration must be non-negative".into());
    }

    let millis = match unit {
        "ms" => num,
        "s" => num * 1_000.0,
        "m" => num * 60_000.0,
        "h" => num * 3_600_000.0,
        _ => unreachable!(),
    };

    Ok(Duration::from_millis(millis as u64))
}

/// Report the first option that is accepted by the parser but not implemented.
///
/// These are declared in [`RunArgs`] and documented in the CLI reference, so
/// they cannot simply be removed without breaking `--help` and the docs. Until
/// each is wired up, failing loudly is the honest behaviour: the alternative
/// is a run that quietly ignores what the user asked for.
fn unsupported_option(args: &RunArgs) -> Option<CliError> {
    let unsupported = |option: &str, detail: &str, suggestion: &str| {
        Some(CliError::Config {
            detail: format!("{option} is not implemented: {detail}"),
            file: None,
            suggestion: suggestion.to_owned(),
        })
    };

    if args.limit.is_some() {
        return unsupported(
            "--limit",
            "the runtime has no element budget to stop a flow at a count",
            "A source decides how many elements it produces; give it a bounded count through \
             the node's `config` in Torvyn.toml, or use --timeout to bound the run by time.",
        );
    }
    if args.input.is_some() {
        return unsupported(
            "--input",
            "overriding a source's input is not wired up",
            "Set the source node's `config` in Torvyn.toml and re-run without --input.",
        );
    }
    if args.output.is_some() {
        return unsupported(
            "--output",
            "overriding a sink's destination is not wired up",
            "Set the sink node's `config` in Torvyn.toml and re-run without --output.",
        );
    }
    if !args.config.is_empty() {
        return unsupported(
            "--config",
            "per-component configuration overrides are not wired up",
            "Set the node's `config` value in Torvyn.toml and re-run without --config.",
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn test_parse_duration_millis() {
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("-5s").is_err());
    }

    #[test]
    fn test_build_run_result_maps_metrics_and_throughput() {
        let result = build_run_result("pipeline".into(), Duration::from_secs(2), 100, 3, 4, 3);
        assert_eq!(result.flow_name, "pipeline");
        assert_eq!(result.elements_processed, 100);
        assert_eq!(result.error_count, 3);
        assert_eq!(result.component_count, 4);
        assert_eq!(result.edge_count, 3);
        assert_eq!(result.duration_secs, 2.0);
        // 100 elements over 2 seconds.
        assert!((result.throughput_elem_per_sec - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_run_result_zero_duration_has_zero_throughput() {
        // A zero-duration run must not divide by zero.
        let result = build_run_result("empty".into(), Duration::ZERO, 0, 0, 0, 0);
        assert_eq!(result.throughput_elem_per_sec, 0.0);
        assert_eq!(result.elements_processed, 0);
    }
}
