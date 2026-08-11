//! `torvyn run` — execute a pipeline locally.
//!
//! Instantiates the Torvyn host runtime, loads components, and runs
//! the pipeline. Displays real-time throughput and summary on exit.
//!
//! A pipeline that fails says so and exits non-zero. It used to exit 0 with a
//! summary that mentioned only a bare error count: a flow whose first element
//! killed it produced no output, reported `Errors: 1` between throughput and
//! peak memory, and returned success. The failure was recorded — the driver
//! logs it and the reactor stores the reason — but the CLI installs no log
//! subscriber and never asked the reactor, so nothing reached the user.

use crate::cli::RunArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::time::{Duration, Instant};
use torvyn_host::{CancellationReason, FlowOutcome, FlowStage};

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
    /// The state the flow ended in.
    pub flow_state: String,
    /// Why it ended, when the runtime recorded a reason.
    ///
    /// Always serialised for machine consumers, which have no error line to
    /// read it from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_stopped_because: Option<String>,
    /// Whether the flow ended in a failed state.
    ///
    /// Kept out of the human summary — the error printed after it says so far
    /// more usefully — but present in the JSON so a consumer need not parse
    /// the state string.
    pub failed: bool,
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
        // The state is what says whether the numbers above describe a
        // completed pipeline or one that stopped partway through.
        terminal::print_kv(ctx, "Flow state", &self.flow_state);

        // The reason is worth a line only when nothing else will say it. A
        // completed flow's reason is "source completed", which the state
        // already conveys; a failed flow's reason is restated below as an
        // error, in terms of the node names the manifest uses rather than the
        // component ids the reason carries.
        if !self.failed && self.flow_state != "Completed" {
            if let Some(reason) = &self.flow_stopped_because {
                terminal::print_kv(ctx, "Stopped because", reason);
            }
        }
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

    // How the flow ended, read before shutdown so it reflects the flow's own
    // outcome rather than the shutdown that followed it. The reactor keeps
    // this after reaping the driver task, which is why it is answerable here.
    let outcome = host.flow_outcome(flow_id).await.ok();

    // The flow's stages, so a failure can name the node the manifest declares
    // rather than the positional component id the runtime uses internally.
    let stages = flow_stages(&host, flow_id).await;

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
            flow_name.clone(),
            elapsed,
            snapshot.elements_total,
            snapshot.errors_total,
            snapshot.components.len(),
            snapshot.streams.len(),
            outcome.as_ref(),
        ),
        None => build_run_result(flow_name.clone(), elapsed, 0, 0, 0, 0, outcome.as_ref()),
    };

    let elements = result.elements_processed;
    let command_result = CommandResult {
        success: true,
        failure: None,
        command: "run".into(),
        data: result,
        warnings: vec![],
    };

    // A pipeline that failed must say so and exit non-zero. The summary is
    // rendered either way — how far the flow got before it stopped is the
    // first thing worth knowing.
    Ok(match outcome.filter(FlowOutcome::failed) {
        Some(failed) => command_result.failed(
            describe_failure(&flow_name, &failed, &stages),
            format!(
                "{elements} element(s) had been processed when the flow stopped. \
                 Run `torvyn trace` to see each element's path through the pipeline."
            ),
        ),
        None => command_result,
    })
}

/// The stages of a flow, for turning a component id into the node name the
/// manifest declares.
///
/// COLD PATH.
pub(crate) async fn flow_stages(
    host: &torvyn_host::TorvynHost,
    flow_id: torvyn_types::FlowId,
) -> Vec<FlowStage> {
    host.list_flows()
        .await
        .into_iter()
        .find(|record| record.flow_id == flow_id)
        .map(|record| record.stages)
        .unwrap_or_default()
}

/// State a flow's failure in one line, naming the component where the runtime
/// knows which one, and the error it returned.
///
/// `stages` carries each component's identity alongside the node name the
/// manifest gives it, so the message reads `component "transform"` rather than
/// the positional id the runtime uses internally.
///
/// Shared with `trace` and `bench`: all three watch the same flow reach the
/// same terminal state, and all three used to ignore it.
///
/// COLD PATH — called once, on failure.
pub(crate) fn describe_failure(
    flow_name: &str,
    outcome: &FlowOutcome,
    stages: &[FlowStage],
) -> String {
    let Some(reason) = &outcome.reason else {
        return format!("flow \"{flow_name}\" failed");
    };

    match reason {
        CancellationReason::DownstreamError { component, error } => {
            let who = stages
                .iter()
                .find(|stage| stage.component_id == *component)
                .map_or_else(
                    || format!("component {component}"),
                    |stage| format!("component \"{}\"", stage.name),
                );
            format!("flow \"{flow_name}\" failed: {who} returned {error}")
        }
        other => format!("flow \"{flow_name}\" failed: {other}"),
    }
}

/// Assemble the run summary from a flow's recorded metrics.
///
/// COLD PATH — called once when a run finishes.
#[allow(clippy::too_many_arguments)]
fn build_run_result(
    flow_name: String,
    duration: Duration,
    elements_processed: u64,
    error_count: u64,
    component_count: usize,
    edge_count: usize,
    outcome: Option<&FlowOutcome>,
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
        // A flow the host has no record of is reported as unknown rather than
        // assumed to have completed.
        flow_state: outcome
            .map_or_else(|| "unknown".to_owned(), |outcome| outcome.state.to_string()),
        flow_stopped_because: outcome
            .and_then(|o| o.reason.as_ref())
            .map(ToString::to_string),
        failed: outcome.is_some_and(FlowOutcome::failed),
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

    use torvyn_types::{ComponentId, ComponentRole, FlowState, ProcessError};

    fn completed() -> FlowOutcome {
        FlowOutcome {
            state: FlowState::Completed,
            reason: Some(CancellationReason::SourceComplete),
        }
    }

    fn failed_with(component: u64, error: ProcessError) -> FlowOutcome {
        FlowOutcome {
            state: FlowState::Failed,
            reason: Some(CancellationReason::DownstreamError {
                component: ComponentId::new(component),
                error,
            }),
        }
    }

    fn stage(id: u64, name: &str, role: ComponentRole) -> FlowStage {
        FlowStage {
            component_id: ComponentId::new(id),
            name: name.to_owned(),
            role,
        }
    }

    #[test]
    fn test_build_run_result_maps_metrics_and_throughput() {
        let outcome = completed();
        let result = build_run_result(
            "pipeline".into(),
            Duration::from_secs(2),
            100,
            3,
            4,
            3,
            Some(&outcome),
        );
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
        let result = build_run_result("empty".into(), Duration::ZERO, 0, 0, 0, 0, None);
        assert_eq!(result.throughput_elem_per_sec, 0.0);
        assert_eq!(result.elements_processed, 0);
    }

    /// The summary must say which state the flow ended in. Without it the
    /// numbers above are ambiguous: 2 elements and 1 error reads the same for a
    /// pipeline that recovered and one that died on its second element.
    #[test]
    fn the_summary_reports_the_terminal_state() {
        let outcome = completed();
        let result = build_run_result(
            "f".into(),
            Duration::from_secs(1),
            10,
            0,
            2,
            1,
            Some(&outcome),
        );
        assert_eq!(result.flow_state, "Completed");
        assert_eq!(
            result.flow_stopped_because.as_deref(),
            Some("source completed")
        );
    }

    /// A flow the host has no record of must be reported as unknown rather
    /// than assumed to have completed.
    #[test]
    fn an_unknown_outcome_is_not_reported_as_success() {
        let result = build_run_result("f".into(), Duration::from_secs(1), 0, 0, 0, 0, None);
        assert_eq!(result.flow_state, "unknown");
        assert!(result.flow_stopped_because.is_none());
    }

    /// The failure must name the node the manifest declares, not the
    /// positional id the runtime uses internally.
    #[test]
    fn a_failure_names_the_component_and_the_error() {
        let stages = [
            stage(1, "source", ComponentRole::Source),
            stage(2, "transform", ComponentRole::Processor),
            stage(3, "sink", ComponentRole::Sink),
        ];
        let outcome = failed_with(2, ProcessError::InvalidInput("bad payload".into()));

        let message = describe_failure("main", &outcome, &stages);
        assert!(message.contains("\"main\""), "{message}");
        assert!(message.contains("component \"transform\""), "{message}");
        assert!(message.contains("bad payload"), "{message}");
    }

    /// Every error variant must survive to the message. They used to be
    /// flattened into `Internal` before the reason was stored, so a malformed
    /// input and a runtime bug produced the same post-mortem.
    #[test]
    fn every_error_variant_reaches_the_message() {
        let stages = [stage(1, "worker", ComponentRole::Processor)];
        for (error, expected) in [
            (ProcessError::InvalidInput("x".into()), "invalid input"),
            (ProcessError::Unavailable("x".into()), "unavailable"),
            (ProcessError::Internal("x".into()), "internal"),
            (ProcessError::DeadlineExceeded, "deadline"),
            (ProcessError::Fatal("x".into()), "fatal"),
        ] {
            let message = describe_failure("main", &failed_with(1, error), &stages);
            assert!(
                message.to_lowercase().contains(expected),
                "expected {expected:?} in: {message}"
            );
        }
    }

    /// A failure the runtime cannot attribute to a component still has to be
    /// stated, not swallowed.
    #[test]
    fn an_unattributed_failure_is_still_reported() {
        let outcome = FlowOutcome {
            state: FlowState::Failed,
            reason: Some(CancellationReason::ResourceExhaustion {
                detail: "buffer pool empty".into(),
            }),
        };
        let message = describe_failure("main", &outcome, &[]);
        assert!(message.contains("buffer pool empty"), "{message}");

        let no_reason = FlowOutcome {
            state: FlowState::Failed,
            reason: None,
        };
        assert!(describe_failure("main", &no_reason, &[]).contains("failed"));
    }

    /// A component the flow's stage list does not cover must still be named,
    /// by id rather than not at all.
    #[test]
    fn an_unknown_component_falls_back_to_its_id() {
        let outcome = failed_with(9, ProcessError::Fatal("gone".into()));
        let message = describe_failure("main", &outcome, &[]);
        assert!(message.contains('9'), "{message}");
        assert!(message.contains("gone"), "{message}");
    }
}
