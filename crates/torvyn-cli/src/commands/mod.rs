//! Command dispatch and shared types.
//!
//! Routes parsed CLI commands to their implementations.

pub mod bench;
pub mod build;
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
use std::path::Path;

/// The error every command that needs a flow reports when the manifest has
/// none.
///
/// A manifest that declares components but no flow is the common case — the
/// user has components and has not said how to connect them — and it is worth
/// distinguishing from an empty project, because the fix differs. Showing the
/// block to add matters more than naming the section: "Add a [flow.*] section"
/// tells someone who already knows the syntax something they knew, and
/// everyone else nothing.
///
/// The example names the project's own components where it has them, so the
/// block is one the reader can paste and adapt rather than invent.
///
/// COLD PATH — constructed once, on the way out.
pub fn no_flow_defined(
    manifest_path: &Path,
    manifest: &torvyn_config::ComponentManifest,
) -> CliError {
    let components: Vec<&str> = manifest
        .components
        .iter()
        .map(|c| c.name.as_str())
        .collect();

    let detail = if components.is_empty() {
        "This project defines no flow, and no components to build one from".to_owned()
    } else {
        format!(
            "This project defines {} but no flow, so there is no pipeline",
            match components.as_slice() {
                [one] => format!("the component \"{one}\""),
                many => format!("components {}", quoted_list(many)),
            }
        )
    };

    let suggestion = format!(
        "Add a flow to {}, for example:\n\n{}",
        manifest_path.display(),
        flow_example(&components)
    );

    CliError::Config {
        detail,
        file: Some(manifest_path.display().to_string()),
        suggestion,
    }
}

/// Render `["a", "b", "c"]` as `"a", "b" and "c"`.
fn quoted_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => format!("\"{one}\""),
        [rest @ .., last] => format!(
            "{} and \"{last}\"",
            rest.iter()
                .map(|i| format!("\"{i}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A `[flow.main]` block wired to the project's own components where they
/// exist, and to placeholder names where they do not.
///
/// A flow needs a source and a sink, so the example always shows both ends
/// even when the project has only one component: the missing end is the thing
/// the reader has to supply, and leaving it out hides that.
fn flow_example(components: &[&str]) -> String {
    let source = components.first().copied().unwrap_or("my-source");
    let sink = components.get(1).copied().unwrap_or("my-sink");
    format!(
        "  [flow.main]\n\
         \n\
         \x20 [flow.main.nodes.source]\n\
         \x20 component = \"{source}\"\n\
         \x20 interface = \"torvyn:streaming/source\"\n\
         \n\
         \x20 [flow.main.nodes.sink]\n\
         \x20 component = \"{sink}\"\n\
         \x20 interface = \"torvyn:streaming/sink\"\n\
         \n\
         \x20 [[flow.main.edges]]\n\
         \x20 from = {{ node = \"source\", port = \"output\" }}\n\
         \x20 to = {{ node = \"sink\", port = \"input\" }}"
    )
}

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
        Command::Build(args) => {
            let result = build::execute(args, ctx)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(components: &[&str]) -> torvyn_config::ComponentManifest {
        let mut toml = String::from(
            "[torvyn]\nname = \"p\"\nversion = \"0.1.0\"\ncontract_version = \"0.1.0\"\n",
        );
        for name in components {
            toml.push_str(&format!(
                "\n[[component]]\nname = \"{name}\"\npath = \"components/{name}\"\nlanguage = \"rust\"\n"
            ));
        }
        torvyn_config::ComponentManifest::from_toml_str(&toml, "Torvyn.toml")
            .expect("fixture manifest parses")
    }

    fn rendered(err: &CliError) -> String {
        match err {
            CliError::Config {
                detail, suggestion, ..
            } => format!("{detail}\n{suggestion}"),
            other => panic!("expected a Config error, got {other:?}"),
        }
    }

    /// The example must name the project's own components, so it is a block
    /// the reader can paste rather than a syntax reminder.
    #[test]
    fn the_example_flow_uses_the_projects_own_components() {
        let manifest = manifest_with(&["tokenizer", "printer"]);
        let text = rendered(&no_flow_defined(Path::new("Torvyn.toml"), &manifest));

        assert!(
            text.contains("tokenizer") && text.contains("printer"),
            "{text}"
        );
        assert!(text.contains("[flow.main]"), "{text}");
        assert!(text.contains("[[flow.main.edges]]"), "{text}");
        assert!(
            text.contains("from = { node = \"source\", port = \"output\" }"),
            "the example must show the edge shape the manifest actually uses:\n{text}"
        );
    }

    /// A project with no components at all is a different situation from one
    /// that has components and has not connected them, and the message says so.
    #[test]
    fn distinguishes_an_empty_project_from_an_unconnected_one() {
        let empty = rendered(&no_flow_defined(
            Path::new("Torvyn.toml"),
            &manifest_with(&[]),
        ));
        assert!(empty.contains("no components"), "{empty}");

        let unconnected = rendered(&no_flow_defined(
            Path::new("Torvyn.toml"),
            &manifest_with(&["only-one"]),
        ));
        assert!(unconnected.contains("only-one"), "{unconnected}");
        assert!(!unconnected.contains("no components"), "{unconnected}");
    }

    /// A one-component project still needs both ends, so the example shows a
    /// placeholder for the end the project does not have rather than omitting
    /// it — the missing end is the thing the reader has to supply.
    #[test]
    fn a_single_component_project_still_sees_both_ends() {
        let text = rendered(&no_flow_defined(
            Path::new("Torvyn.toml"),
            &manifest_with(&["my-source"]),
        ));
        assert!(text.contains("nodes.source"), "{text}");
        assert!(text.contains("nodes.sink"), "{text}");
    }

    #[test]
    fn quoted_list_reads_as_prose() {
        assert_eq!(quoted_list(&[]), "");
        assert_eq!(quoted_list(&["a"]), "\"a\"");
        assert_eq!(quoted_list(&["a", "b"]), "\"a\" and \"b\"");
        assert_eq!(quoted_list(&["a", "b", "c"]), "\"a\", \"b\" and \"c\"");
    }
}
