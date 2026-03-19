//! Diagnostic rendering for CLI errors.
//!
//! Renders errors in the four-part format mandated by Doc 07 §5.1:
//! what went wrong, where, why, how to fix.

use crate::errors::CliError;
use crate::output::OutputContext;

/// Render a [`CliError`] to stderr with styled diagnostics.
///
/// COLD PATH — called at most once per invocation.
pub fn render_cli_error(ctx: &OutputContext, err: &CliError) {
    match err {
        CliError::Config {
            detail,
            file,
            suggestion,
        } => {
            print_error_header(ctx, detail);
            if let Some(f) = file {
                print_location(ctx, f);
            }
            print_help(ctx, suggestion);
        }
        CliError::Contract {
            detail,
            diagnostics,
        } => {
            print_error_header(ctx, detail);
            for d in diagnostics {
                eprintln!("  {d}");
            }
        }
        CliError::Link {
            detail,
            diagnostics,
        } => {
            print_error_header(ctx, detail);
            for d in diagnostics {
                eprintln!("  {d}");
            }
        }
        CliError::Runtime { detail, context } => {
            print_error_header(ctx, detail);
            if let Some(c) = context {
                eprintln!("  context: {c}");
            }
        }
        CliError::Packaging { detail, suggestion } => {
            print_error_header(ctx, detail);
            print_help(ctx, suggestion);
        }
        CliError::Security { detail, suggestion } => {
            print_error_header(ctx, detail);
            print_help(ctx, suggestion);
        }
        CliError::Environment { detail, fix } => {
            print_error_header(ctx, detail);
            print_help(ctx, &format!("fix: {fix}"));
        }
        CliError::Io { detail, path } => {
            print_error_header(ctx, detail);
            if let Some(p) = path {
                print_location(ctx, p);
            }
        }
        CliError::Internal { detail } => {
            print_error_header(ctx, &format!("Internal error: {detail}"));
            print_help(
                ctx,
                "This is likely a bug. Please report it at https://github.com/torvyn/torvyn/issues",
            );
        }
        CliError::NotImplemented { command } => {
            print_error_header(ctx, &format!("Command '{command}' is not yet implemented"));
            print_help(
                ctx,
                "This command will be available in a future release (Part B).",
            );
        }
    }
}

/// Print the error header line: `error: <message>`
fn print_error_header(ctx: &OutputContext, message: &str) {
    if ctx.color_enabled {
        eprintln!("\n{} {message}\n", console::style("error:").red().bold());
    } else {
        eprintln!("\nerror: {message}\n");
    }
}

/// Print a file location line.
fn print_location(ctx: &OutputContext, location: &str) {
    if ctx.color_enabled {
        eprintln!("  {} {location}", console::style("-->").dim());
    } else {
        eprintln!("  --> {location}");
    }
}

/// Print a help/suggestion line.
fn print_help(ctx: &OutputContext, help: &str) {
    if ctx.color_enabled {
        eprintln!("  {} {help}", console::style("help:").cyan().bold());
    } else {
        eprintln!("  help: {help}");
    }
}
