//! clap derive definitions for the Torvyn CLI.
//!
//! This module defines the complete argument schema for the `torvyn` binary.
//! Every subcommand, flag, and argument is defined here with embedded help text.

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Top-level CLI
// ---------------------------------------------------------------------------

/// Torvyn — Ownership-aware reactive streaming runtime
///
/// Build, validate, run, trace, benchmark, and package streaming components.
///
/// Getting started? Try: torvyn init my-first-pipeline --template full-pipeline
#[derive(Parser, Debug)]
#[command(name = "torvyn", version, about, long_about = None)]
#[command(propagate_version = true)]
#[command(
    after_help = "Getting started? Try: torvyn init my-first-pipeline --template full-pipeline"
)]
pub struct Cli {
    /// Global options available to all subcommands.
    #[command(flatten)]
    pub global: GlobalOpts,

    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

// ---------------------------------------------------------------------------
// Global options
// ---------------------------------------------------------------------------

/// Global options applied to every subcommand.
///
/// ## Invariants
/// - `verbose` and `quiet` are mutually exclusive (enforced by clap `conflicts_with`).
/// - `format` defaults to `OutputFormat::Human`.
/// - `color` defaults to `ColorChoice::Auto`.
#[derive(Parser, Debug, Clone)]
pub struct GlobalOpts {
    /// Increase output verbosity (show debug-level messages).
    #[arg(long, short = 'v', global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Suppress non-essential output (errors only).
    #[arg(long, short = 'q', global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Output format for command results.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// Color output control.
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Output format for CLI results.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Styled terminal output with colors, tables, and progress indicators.
    Human,
    /// Machine-readable JSON output to stdout.
    Json,
}

/// Color output control.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    /// Auto-detect terminal color support.
    Auto,
    /// Always use color output.
    Always,
    /// Never use color output.
    Never,
}

/// Project template type for `torvyn init`.
///
/// Every template here scaffolds a project that `torvyn build` and `torvyn run`
/// can complete, except [`TemplateKind::Empty`], which deliberately generates
/// no components at all.
///
/// `filter`, `router`, and `aggregator` were offered until they were found to
/// scaffold components the runtime cannot execute: they implement
/// `torvyn:streaming/{filter,router,aggregator}`, which exist in no contract
/// the engine binds. Such a component compiles and is then refused at
/// instantiation. Those roles are Phase 1 on the roadmap, and the templates
/// will return with the interfaces that make them runnable.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    /// Data producer (no input, one output).
    Source,
    /// Data consumer (one input, no output).
    Sink,
    /// Stateless data transformer.
    Transform,
    /// Complete multi-component pipeline with source + transform + sink.
    FullPipeline,
    /// Minimal skeleton for experienced users.
    Empty,
}

impl TemplateKind {
    /// Every template.
    ///
    /// Exists so tests can iterate the full set rather than a hand-written
    /// list that silently falls behind — a stale list is how three templates
    /// that could not run went unnoticed. Not read by the command itself,
    /// which takes the template clap parsed.
    #[allow(dead_code)]
    pub const ALL: &'static [Self] = &[
        Self::Source,
        Self::Sink,
        Self::Transform,
        Self::FullPipeline,
        Self::Empty,
    ];

    /// Returns a human-readable description of this template.
    #[allow(dead_code)]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Source => "Data producer (no input, one output)",
            Self::Sink => "Data consumer (one input, no output)",
            Self::Transform => "Stateless data transformer",
            Self::FullPipeline => "Complete pipeline with source + transform + sink",
            Self::Empty => "Minimal skeleton for experienced users",
        }
    }

    /// Components this template scaffolds *alongside* the one the user is
    /// building, so that the flow it generates can actually run.
    ///
    /// A pipeline needs a source and a sink. A scaffolded transform has
    /// nothing to read from and nowhere to write to, so a template for one
    /// ships the two ends with it; a scaffolded source ships only a sink, and
    /// a scaffolded sink only a source.
    ///
    /// These names occupy the manifest's component namespace, which is why
    /// [`Self::name_collides_with_companion`] exists.
    pub const fn companions(&self) -> &'static [&'static str] {
        match self {
            Self::Source => &["sink"],
            Self::Sink => &["source"],
            Self::Transform => &["source", "sink"],
            // Names every component it scaffolds itself; nothing is derived
            // from the project name, so nothing can collide.
            Self::FullPipeline | Self::Empty => &[],
        }
    }

    /// Whether the scaffolded manifest defines a flow, and therefore whether
    /// `torvyn run` applies to the project this template produces.
    pub const fn scaffolds_flow(&self) -> bool {
        !matches!(self, Self::Empty)
    }

    /// The companion this project name would collide with, if any.
    ///
    /// The user's own component is named after the project, so a project
    /// called `sink` would produce two components named `sink` — a manifest
    /// that cannot be resolved. Caught at `init` time, where the fix is to
    /// pick another name, rather than at `build` time where the cause is far
    /// from the effect.
    pub fn name_collides_with_companion(&self, project_name: &str) -> Option<&'static str> {
        self.companions()
            .iter()
            .copied()
            .find(|companion| *companion == project_name)
    }
}

/// Implementation language for `torvyn init`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Rust (primary, fully supported).
    Rust,
    /// Go (via TinyGo, limited support).
    Go,
    /// Python (via componentize-py, limited support).
    Python,
    /// Zig (experimental).
    Zig,
}

/// Shell type for shell completion generation.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// Bash shell completions.
    Bash,
    /// Zsh shell completions.
    Zsh,
    /// Fish shell completions.
    Fish,
    /// PowerShell completions.
    #[value(name = "powershell")]
    PowerShell,
}

/// Sections to show in `torvyn inspect`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectSection {
    /// Show all available information.
    All,
    /// Show exported and imported interfaces.
    Interfaces,
    /// Show required capabilities.
    Capabilities,
    /// Show component metadata (version, authors, etc.).
    Metadata,
    /// Show binary size breakdown.
    Size,
    /// Show WIT contract definitions.
    Contracts,
    /// Show embedded benchmark results.
    Benchmarks,
}

/// Report format for `torvyn bench`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    /// Styled terminal table.
    Pretty,
    /// JSON report.
    Json,
    /// CSV report.
    Csv,
    /// Markdown report.
    Markdown,
}

/// Trace output format for `torvyn trace`.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TraceFormat {
    /// Pretty-printed tree trace.
    Pretty,
    /// JSON trace spans.
    Json,
    /// OpenTelemetry Protocol export.
    Otlp,
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// All available subcommands for the `torvyn` binary.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new Torvyn project.
    ///
    /// Scaffolds a complete project with WIT contracts, implementation stubs,
    /// a Torvyn.toml manifest, and build configuration. The generated project
    /// compiles and runs out of the box.
    Init(InitArgs),

    /// Validate contracts, manifest, and project structure.
    ///
    /// Performs static analysis: parses WIT contracts, validates the manifest
    /// schema, resolves references, and checks capability declarations.
    Check(CheckArgs),

    /// Compile the project's components to WebAssembly.
    ///
    /// Builds every component declared in the manifest with the toolchain for
    /// its language and places the artifacts where `torvyn run` looks for
    /// them.
    Build(BuildArgs),

    /// Verify component composition compatibility.
    ///
    /// Given a pipeline configuration, verifies that all component interfaces
    /// are compatible and the topology is valid (DAG, connected, role-consistent).
    Link(LinkArgs),

    /// Execute a pipeline locally.
    ///
    /// Instantiates the Torvyn host runtime, loads components, and runs
    /// the pipeline. Displays element count, errors, and completion status.
    Run(RunArgs),

    /// Run with full diagnostic tracing.
    ///
    /// Like `run` but with full tracing enabled. Outputs flow timeline
    /// showing per-stage latency, resource transfers, backpressure events.
    Trace(TraceArgs),

    /// Benchmark a pipeline.
    ///
    /// Runs the pipeline under sustained load with warmup, then produces
    /// a report with p50/p95/p99/p99.9 latency, throughput, copy count,
    /// memory, and resource utilization.
    Bench(BenchArgs),

    /// Package as OCI artifact.
    ///
    /// Assembles compiled components, contracts, and metadata into a
    /// distributable artifact.
    Pack(PackArgs),

    /// Publish to a registry.
    ///
    /// Pushes a packaged artifact to a registry. For Phase 0: local
    /// directory registry only.
    Publish(PublishArgs),

    /// Inspect a component or artifact.
    ///
    /// Displays metadata, interfaces, capabilities, and size information
    /// for a compiled .wasm file or packaged artifact.
    Inspect(InspectArgs),

    /// Check development environment.
    ///
    /// Verifies required tools, correct versions, and common misconfigurations.
    /// Run `torvyn doctor --fix` to attempt automatic repair.
    Doctor(DoctorArgs),

    /// Generate shell completions.
    ///
    /// Prints a completion script to stdout for the specified shell.
    /// Example: `torvyn completions bash > ~/.bash_completion.d/torvyn`
    Completions(CompletionsArgs),
}

// ---------------------------------------------------------------------------
// Per-command argument structs
// ---------------------------------------------------------------------------

/// Arguments for `torvyn init`.
#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Directory name and project name.
    /// If omitted, uses the current directory name.
    pub project_name: Option<String>,

    /// Project template to use.
    #[arg(long, short = 't', value_enum, default_value_t = TemplateKind::Transform)]
    pub template: TemplateKind,

    /// Implementation language.
    #[arg(long, short = 'l', value_enum, default_value_t = Language::Rust)]
    pub language: Language,

    /// Skip git repository initialization.
    #[arg(long)]
    pub no_git: bool,

    /// Skip example implementation, generate contract stubs only.
    #[arg(long)]
    pub no_example: bool,

    /// Torvyn contract version to target.
    #[arg(long, default_value = "0.1.0")]
    pub contract_version: String,

    /// Overwrite existing directory contents.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `torvyn check`.
#[derive(Parser, Debug)]
pub struct CheckArgs {
    /// Path to Torvyn.toml.
    #[arg(long, default_value = "./Torvyn.toml")]
    pub manifest: PathBuf,

    /// Treat warnings as errors.
    #[arg(long)]
    pub strict: bool,
}

/// Arguments for `torvyn link`.
#[derive(Parser, Debug)]
pub struct LinkArgs {
    /// Path to Torvyn.toml with flow definition.
    #[arg(long, default_value = "./Torvyn.toml")]
    pub manifest: PathBuf,

    /// Specific flow to check (default: all flows).
    #[arg(long)]
    pub flow: Option<String>,

    /// Directory containing compiled .wasm components.
    #[arg(long)]
    pub components: Option<PathBuf>,

    /// Show full interface compatibility details.
    #[arg(long)]
    pub detail: bool,
}

/// Arguments for `torvyn build`.
#[derive(Parser, Debug)]
pub struct BuildArgs {
    /// Path to Torvyn.toml.
    #[arg(long, default_value = "./Torvyn.toml")]
    pub manifest: PathBuf,

    /// Build only this component (default: every declared component).
    #[arg(long)]
    pub component: Option<String>,

    /// Build without optimisations, overriding `release` in the `[build]`
    /// table. Faster to compile, slower to run.
    #[arg(long)]
    pub debug: bool,
}

/// Arguments for `torvyn run`.
#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Path to Torvyn.toml.
    #[arg(long, default_value = "./Torvyn.toml")]
    pub manifest: PathBuf,

    /// Flow to execute (default: first defined flow).
    #[arg(long)]
    pub flow: Option<String>,

    /// Override source input (file path, stdin, or generator).
    #[arg(long)]
    pub input: Option<String>,

    /// Override sink output (file path, stdout).
    #[arg(long)]
    pub output: Option<String>,

    /// Process at most N elements then exit.
    #[arg(long)]
    pub limit: Option<u64>,

    /// Maximum execution time (e.g., 30s, 5m).
    #[arg(long)]
    pub timeout: Option<String>,

    /// Override component configuration values.
    #[arg(long, value_name = "KEY=VALUE")]
    pub config: Vec<String>,

    /// Log verbosity. Not yet wired: the CLI installs no log subscriber, so
    /// this currently has no effect on what is printed.
    #[arg(long, default_value = "info")]
    pub log_level: String,
}

/// Arguments for `torvyn trace`.
#[derive(Parser, Debug)]
pub struct TraceArgs {
    /// Path to Torvyn.toml.
    #[arg(long, default_value = "./Torvyn.toml")]
    pub manifest: PathBuf,

    /// Flow to trace (default: first defined flow).
    #[arg(long)]
    pub flow: Option<String>,

    /// Override source input.
    #[arg(long)]
    pub input: Option<String>,

    /// Trace at most N elements.
    #[arg(long)]
    pub limit: Option<u64>,

    /// Write trace data to file (default: stdout).
    #[arg(long)]
    pub output_trace: Option<PathBuf>,

    /// Trace output format.
    #[arg(long, value_enum, default_value_t = TraceFormat::Pretty)]
    pub trace_format: TraceFormat,

    /// Include buffer content snapshots in trace.
    #[arg(long)]
    pub show_buffers: bool,

    /// Highlight backpressure events.
    #[arg(long)]
    pub show_backpressure: bool,
}

/// Arguments for `torvyn bench`.
#[derive(Parser, Debug)]
pub struct BenchArgs {
    /// Path to Torvyn.toml.
    #[arg(long, default_value = "./Torvyn.toml")]
    pub manifest: PathBuf,

    /// Flow to benchmark (default: first defined flow).
    #[arg(long)]
    pub flow: Option<String>,

    /// Benchmark duration (default: 10s).
    #[arg(long, default_value = "10s")]
    pub duration: String,

    /// Warmup period excluded from results (default: 2s).
    #[arg(long, default_value = "2s")]
    pub warmup: String,

    /// Override source input (for reproducible benchmarks).
    #[arg(long)]
    pub input: Option<String>,

    /// Write report to file (default: stdout).
    #[arg(long)]
    pub report: Option<PathBuf>,

    /// Report format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Pretty)]
    pub report_format: ReportFormat,

    /// Compare against a previous benchmark result.
    #[arg(long)]
    pub compare: Option<PathBuf>,

    /// Save result as a named baseline for future comparison.
    #[arg(long)]
    pub baseline: Option<String>,
}

/// Arguments for `torvyn pack`.
#[derive(Parser, Debug)]
pub struct PackArgs {
    /// Path to Torvyn.toml.
    #[arg(long, default_value = "./Torvyn.toml")]
    pub manifest: PathBuf,

    /// Specific component to pack (default: all in project).
    #[arg(long)]
    pub component: Option<String>,

    /// Output artifact path (default: .torvyn/artifacts/).
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// OCI tag (default: derived from manifest version).
    #[arg(long)]
    pub tag: Option<String>,

    /// No effect: WIT contracts are always included in an artifact.
    #[arg(long)]
    pub include_source: bool,

    /// Sign artifact (requires signing key configuration).
    #[arg(long)]
    pub sign: bool,
}

/// Arguments for `torvyn publish`.
#[derive(Parser, Debug)]
pub struct PublishArgs {
    /// Path to packed artifact (default: latest in .torvyn/artifacts/).
    #[arg(long)]
    pub artifact: Option<PathBuf>,

    /// Target registry URL (default: from config).
    #[arg(long)]
    pub registry: Option<String>,

    /// Override tag.
    #[arg(long)]
    pub tag: Option<String>,

    /// Validate publish without actually pushing.
    #[arg(long)]
    pub dry_run: bool,

    /// Overwrite existing tag.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `torvyn inspect`.
#[derive(Parser, Debug)]
pub struct InspectArgs {
    /// Path to .wasm file, OCI artifact, or registry reference.
    pub target: String,

    /// What section to show.
    #[arg(long, value_enum, default_value_t = InspectSection::All)]
    pub show: InspectSection,
}

/// Arguments for `torvyn doctor`.
#[derive(Parser, Debug)]
pub struct DoctorArgs {
    /// Attempt to fix common issues automatically.
    #[arg(long)]
    pub fix: bool,
}

/// Arguments for `torvyn completions`.
#[derive(Parser, Debug)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    pub shell: ShellKind,
}
