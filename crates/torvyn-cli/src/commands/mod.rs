//! Command dispatch and shared types.
//!
//! Routes parsed CLI commands to their implementations.

pub mod bench;
pub mod check;
pub mod doctor;
pub mod init;
pub mod inspect;
pub mod link;
pub mod pack;
pub mod publish;
pub mod run;
pub mod trace;

use crate::cli::{Command, GlobalOpts};
use crate::errors::CliError;
use crate::output::OutputContext;

/// Execute the given CLI command.
///
/// COLD PATH — called once per invocation.
pub async fn execute_command(
    command: &Command,
    _global: &GlobalOpts,
    ctx: &OutputContext,
) -> Result<(), CliError> {
    match command {
        Command::Init(args) => {
            let result = init::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Check(args) => {
            let result = check::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Link(args) => {
            let result = link::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Run(args) => {
            let result = run::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Trace(args) => {
            let result = trace::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Bench(args) => {
            let result = bench::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Pack(args) => {
            let result = pack::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Publish(args) => {
            let result = publish::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Inspect(args) => {
            let result = inspect::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Doctor(args) => {
            let result = doctor::execute(args, ctx).await?;
            ctx.render(&result);
        }
        Command::Completions(args) => {
            generate_completions(args);
        }
    }
    Ok(())
}

/// Generate shell completions and print to stdout.
fn generate_completions(args: &crate::cli::CompletionsArgs) {
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};

    let mut cmd = crate::cli::Cli::command();
    let shell = match args.shell {
        crate::cli::ShellKind::Bash => Shell::Bash,
        crate::cli::ShellKind::Zsh => Shell::Zsh,
        crate::cli::ShellKind::Fish => Shell::Fish,
        crate::cli::ShellKind::PowerShell => Shell::PowerShell,
    };
    generate(shell, &mut cmd, "torvyn", &mut std::io::stdout());
}
