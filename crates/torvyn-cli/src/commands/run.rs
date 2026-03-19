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
        .ok_or_else(|| CliError::Config {
            detail: "No flow defined in manifest and no --flow specified".into(),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Add a [flow.*] section or use --flow <name>.".into(),
        })?;

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

    // Start the flow
    let _flow_id = host
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

    // Run until completion, limit, timeout, or Ctrl+C
    let ctrl_c = tokio::signal::ctrl_c();
    let run_future = host.run();

    let run_result = if let Some(timeout_dur) = timeout {
        tokio::select! {
            result = run_future => result,
            _ = tokio::time::sleep(timeout_dur) => {
                host.shutdown().await.ok();
                Ok(())
            },
            _ = ctrl_c => {
                eprintln!();
                host.shutdown().await.ok();
                Ok(())
            }
        }
    } else {
        tokio::select! {
            result = run_future => result,
            _ = ctrl_c => {
                eprintln!();
                host.shutdown().await.ok();
                Ok(())
            }
        }
    };

    run_result.map_err(|e| CliError::Runtime {
        detail: format!("Pipeline execution failed: {e}"),
        context: Some(flow_name.clone()),
    })?;

    let elapsed = start.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();

    // Placeholder metrics — FlowSummary doesn't yet expose detailed statistics
    let elements_processed = 0_u64;
    let error_count = 0_u64;
    let throughput = if elapsed_secs > 0.0 {
        elements_processed as f64 / elapsed_secs
    } else {
        0.0
    };

    let result = RunResult {
        duration_secs: elapsed_secs,
        elements_processed,
        throughput_elem_per_sec: throughput,
        error_count,
        peak_memory_bytes: 0,
        flow_name,
        component_count: 0,
        edge_count: 0,
    };

    Ok(CommandResult {
        success: true,
        command: "run".into(),
        data: result,
        warnings: vec![],
    })
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
}
