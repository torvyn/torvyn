//! Torvyn CLI — the developer-facing command-line interface for the
//! Torvyn ownership-aware reactive streaming runtime.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cli;
mod commands;
mod errors;
mod output;
mod templates;

use clap::Parser;
use cli::Cli;
use commands::execute_command;
use output::OutputContext;

/// Main entry point. Parses CLI arguments and dispatches.
///
/// # Exit codes
/// - 0: Success
/// - 1: Command failed (validation error, runtime error, etc.)
/// - 2: Usage error (bad arguments, missing required flags)
/// - 3: Environment error (missing tools, configuration issues)
fn main() {
    let cli = Cli::parse();

    let output_ctx = OutputContext::from_global_opts(&cli.global);

    // Build the tokio runtime for async commands.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            output_ctx.print_fatal(&format!("Failed to initialize async runtime: {e}"));
            std::process::exit(3);
        });

    let exit_code = rt.block_on(async {
        match execute_command(&cli.command, &cli.global, &output_ctx).await {
            Ok(()) => 0,
            Err(cli_err) => {
                cli_err.render(&output_ctx);
                cli_err.exit_code()
            }
        }
    });

    std::process::exit(exit_code);
}
