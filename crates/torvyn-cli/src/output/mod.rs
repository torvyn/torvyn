//! Output formatting for the Torvyn CLI.
//!
//! All commands produce structured result types that implement [`serde::Serialize`].
//! This module provides [`OutputContext`] which renders these results in the
//! user's selected format (human or JSON).

pub mod json;
pub mod table;
pub mod terminal;

use crate::cli::{ColorChoice, GlobalOpts, OutputFormat};
use console::Term;
use serde::Serialize;

/// Output context carrying terminal capabilities and user preferences.
///
/// ## Invariants
/// - `format` matches the user's `--format` flag.
/// - `color_enabled` accounts for `--color`, `NO_COLOR` env, and terminal detection.
/// - `is_tty` is true only when stdout is an interactive terminal.
/// - `verbose` and `quiet` are never both true.
#[derive(Debug, Clone)]
pub struct OutputContext {
    /// Selected output format.
    pub format: OutputFormat,
    /// Whether color output is enabled (resolved from flag + env + terminal).
    pub color_enabled: bool,
    /// Whether stdout is an interactive terminal.
    #[allow(dead_code)]
    pub is_tty: bool,
    /// Terminal width in columns. Falls back to 80.
    #[allow(dead_code)]
    pub term_width: u16,
    /// Whether verbose output is requested.
    pub verbose: bool,
    /// Whether quiet mode is requested (errors only).
    #[allow(dead_code)]
    pub quiet: bool,
}

impl OutputContext {
    /// Construct an [`OutputContext`] from the global CLI options.
    ///
    /// COLD PATH — called once per invocation.
    pub fn from_global_opts(opts: &GlobalOpts) -> Self {
        let term = Term::stdout();
        let is_tty = term.is_term();

        let color_enabled = match opts.color {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => {
                is_tty
                    && std::env::var("NO_COLOR").is_err()
                    && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
            }
        };

        let term_width = if is_tty { term.size().1.max(40) } else { 80 };

        Self {
            format: opts.format,
            color_enabled,
            is_tty,
            term_width,
            verbose: opts.verbose,
            quiet: opts.quiet,
        }
    }

    /// Print a structured result in the selected format.
    ///
    /// COLD PATH — called once per command result.
    pub fn render<T: Serialize + HumanRenderable>(&self, result: &T) {
        match self.format {
            OutputFormat::Json => {
                json::print_json(result);
            }
            OutputFormat::Human => {
                result.render_human(self);
            }
        }
    }

    /// Print a progress message (suppressed in quiet mode and JSON mode).
    #[allow(dead_code)]
    pub fn print_status(&self, symbol: &str, message: &str) {
        if self.quiet || self.format == OutputFormat::Json {
            return;
        }
        if self.color_enabled {
            let styled_symbol = console::style(symbol).green().bold();
            eprintln!("{styled_symbol} {message}");
        } else {
            eprintln!("{symbol} {message}");
        }
    }

    /// Print a warning message.
    pub fn print_warning(&self, message: &str) {
        if self.format == OutputFormat::Json {
            return;
        }
        if self.color_enabled {
            let prefix = console::style("warning:").yellow().bold();
            eprintln!("{prefix} {message}");
        } else {
            eprintln!("warning: {message}");
        }
    }

    /// Print a fatal error message (always shown, even in quiet mode).
    pub fn print_fatal(&self, message: &str) {
        if self.color_enabled {
            let prefix = console::style("error:").red().bold();
            eprintln!("{prefix} {message}");
        } else {
            eprintln!("error: {message}");
        }
    }

    /// Print a debug message (only in verbose mode).
    pub fn print_debug(&self, message: &str) {
        if !self.verbose || self.format == OutputFormat::Json {
            return;
        }
        if self.color_enabled {
            let prefix = console::style("debug:").dim();
            eprintln!("{prefix} {message}");
        } else {
            eprintln!("debug: {message}");
        }
    }

    /// Create an `indicatif` progress bar. Returns `None` if inappropriate.
    #[allow(dead_code)]
    pub fn progress_bar(&self, total: u64, message: &str) -> Option<indicatif::ProgressBar> {
        if !self.is_tty || self.quiet || self.format == OutputFormat::Json {
            return None;
        }
        let pb = indicatif::ProgressBar::new(total);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
                )
                .expect("valid progress bar template")
                .progress_chars("=>-"),
        );
        pb.set_message(message.to_string());
        Some(pb)
    }

    /// Create a spinner for unbounded operations. Returns `None` if inappropriate.
    #[allow(dead_code)]
    pub fn spinner(&self, message: &str) -> Option<indicatif::ProgressBar> {
        if !self.is_tty || self.quiet || self.format == OutputFormat::Json {
            return None;
        }
        let sp = indicatif::ProgressBar::new_spinner();
        sp.set_style(
            indicatif::ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .expect("valid spinner template"),
        );
        sp.set_message(message.to_string());
        sp.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(sp)
    }
}

/// Trait for types that can be rendered to the terminal in human-readable format.
pub trait HumanRenderable {
    /// Render this value to the terminal.
    fn render_human(&self, ctx: &OutputContext);
}

/// The success/failure outcome of a command, with structured output.
#[derive(Debug, Serialize)]
pub struct CommandResult<T: Serialize> {
    /// Whether the command succeeded.
    pub success: bool,
    /// The command name that produced this result.
    pub command: String,
    /// Command-specific result data.
    pub data: T,
    /// Warnings produced during execution.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl<T: Serialize + HumanRenderable> HumanRenderable for CommandResult<T> {
    fn render_human(&self, ctx: &OutputContext) {
        self.data.render_human(ctx);
        for w in &self.warnings {
            ctx.print_warning(w);
        }
    }
}
