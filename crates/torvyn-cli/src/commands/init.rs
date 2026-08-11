//! `torvyn init` — create a new Torvyn project.
//!
//! Scaffolds a complete project with WIT contracts, implementation stubs,
//! a Torvyn.toml manifest, and build configuration.
//!
//! Every template except `empty` scaffolds a project whose flow runs. That is
//! why a single-component template ships example components at the ends it
//! lacks: a transform on its own has nothing to read from and nowhere to write
//! to, so `torvyn run` — which this command prints as its third step — had
//! nothing to execute and failed with "No flow defined in manifest", telling
//! the user to hand-write a section into a file generated seconds earlier.

use crate::cli::{InitArgs, TemplateKind};
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use crate::templates::{self, TemplateVars};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Result of a successful `torvyn init`.
#[derive(Debug, Serialize)]
pub struct InitResult {
    /// The project name.
    pub project_name: String,
    /// The template used.
    pub template: String,
    /// The directory created.
    pub directory: PathBuf,
    /// Files created.
    pub files_created: Vec<PathBuf>,
    /// Whether git was initialized.
    pub git_initialized: bool,
    /// Whether the scaffolded manifest defines a flow, and so whether
    /// `torvyn build` and `torvyn run` apply to this project.
    pub runnable: bool,
}

impl HumanRenderable for InitResult {
    fn render_human(&self, ctx: &OutputContext) {
        terminal::print_success(
            ctx,
            &format!(
                "Created project \"{}\" with template \"{}\"",
                self.project_name, self.template
            ),
        );
        eprintln!();

        // Render directory tree
        let mut entries: Vec<(usize, &str, bool)> = Vec::new();
        entries.push((0, &self.project_name, true));
        for (i, file) in self.files_created.iter().enumerate() {
            let is_last = i == self.files_created.len() - 1;
            let display = file.to_str().unwrap_or("???");
            entries.push((1, display, is_last));
        }
        terminal::print_tree(ctx, &entries);

        eprintln!();
        eprintln!("  Next steps:");
        eprintln!("    cd {}", self.directory.display());
        eprintln!("    torvyn check              # Validate contracts and manifest");
        if self.runnable {
            eprintln!("    torvyn build              # Compile every component to WebAssembly");
            eprintln!("    torvyn run                # Run the pipeline and see its output");
        } else {
            // The `empty` template generates no components and no flow on
            // purpose. Printing the build and run steps would be advice that
            // cannot be followed.
            eprintln!("    # This template scaffolds no components. Add a [[component]] entry");
            eprintln!("    # and a [flow.*] section to Torvyn.toml, then build and run it.");
        }
    }
}

/// Execute the `torvyn init` command.
///
/// COLD PATH.
pub async fn execute(
    args: &InitArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<InitResult>, CliError> {
    // Determine project name and directory
    let project_name = match &args.project_name {
        Some(name) => name.clone(),
        None => {
            let cwd = std::env::current_dir().map_err(|e| CliError::Io {
                detail: format!("Cannot determine current directory: {e}"),
                path: None,
            })?;
            cwd.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| CliError::Config {
                    detail: "Cannot determine project name from current directory".into(),
                    file: None,
                    suggestion: "Provide a project name: torvyn init my-project".into(),
                })?
        }
    };

    // Validate project name
    validate_project_name(&project_name)?;
    reject_companion_collision(args.template, &project_name)?;

    let target_dir = if args.project_name.is_some() {
        PathBuf::from(&project_name)
    } else {
        PathBuf::from(".")
    };

    // Check if directory exists and is non-empty
    if target_dir.exists() && target_dir != Path::new(".") && !args.force {
        let entries: Vec<_> = std::fs::read_dir(&target_dir)
            .map_err(|e| CliError::Io {
                detail: e.to_string(),
                path: Some(target_dir.display().to_string()),
            })?
            .collect();

        if !entries.is_empty() {
            return Err(CliError::Config {
                detail: format!(
                    "Directory \"{}\" already exists and is not empty",
                    target_dir.display()
                ),
                file: None,
                suggestion: "Use --force to overwrite, or choose a different name.".into(),
            });
        }
    }

    ctx.print_debug(&format!(
        "Creating project '{}' with template '{:?}'",
        project_name, args.template
    ));

    // Create project directory
    std::fs::create_dir_all(&target_dir).map_err(|e| CliError::Io {
        detail: format!("Failed to create directory: {e}"),
        path: Some(target_dir.display().to_string()),
    })?;

    // Get and expand template
    let template = templates::get_template(args.template);
    let vars = TemplateVars::new(&project_name, &args.contract_version);
    let files_created =
        templates::expand_template(&template, &vars, &target_dir).map_err(|e| CliError::Io {
            detail: format!("Failed to write template files: {e}"),
            path: Some(target_dir.display().to_string()),
        })?;

    // Initialize git
    let git_initialized = if !args.no_git {
        init_git_repo(&target_dir).unwrap_or(false)
    } else {
        false
    };

    let result = InitResult {
        project_name: project_name.clone(),
        template: format!("{:?}", args.template).to_lowercase(),
        directory: target_dir,
        files_created,
        git_initialized,
        runnable: args.template.scaffolds_flow(),
    };

    Ok(CommandResult {
        success: true,
        failure: None,
        command: "init".into(),
        data: result,
        warnings: vec![],
    })
}

/// Validate that a project name is acceptable.
fn validate_project_name(name: &str) -> Result<(), CliError> {
    if name.is_empty() || name.len() > 64 {
        return Err(CliError::Config {
            detail: format!(
                "Project name must be 1\u{2013}64 characters, got {}",
                name.len()
            ),
            file: None,
            suggestion: "Choose a shorter name.".into(),
        });
    }

    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CliError::Config {
            detail: format!("Project name contains invalid characters: \"{name}\""),
            file: None,
            suggestion: "Use only alphanumeric characters, hyphens, and underscores.".into(),
        });
    }

    if name.starts_with('-') || name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(CliError::Config {
            detail: format!("Project name cannot start with a hyphen or digit: \"{name}\""),
            file: None,
            suggestion: "Start with a letter or underscore.".into(),
        });
    }

    Ok(())
}

/// Refuse a project name that would collide with a component the template
/// scaffolds alongside it.
///
/// The user's component is named after the project, so `torvyn init sink
/// --template transform` would declare two components called `sink`. Caught
/// here, where the fix is obvious, rather than at build time where the cause
/// is far from the effect.
fn reject_companion_collision(template: TemplateKind, project_name: &str) -> Result<(), CliError> {
    let Some(companion) = template.name_collides_with_companion(project_name) else {
        return Ok(());
    };
    Err(CliError::Config {
        detail: format!(
            "Project name \"{project_name}\" collides with the example component this template \
             scaffolds alongside it, which is also called \"{companion}\""
        ),
        file: None,
        suggestion: format!(
            "Choose another name, for example \"my-{project_name}\". Or use `--template \
             full-pipeline`, which names every component it generates itself."
        ),
    })
}

/// Attempt to initialize a git repository.
fn init_git_repo(dir: &Path) -> Result<bool, std::io::Error> {
    let status = std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match status {
        Ok(s) => Ok(s.success()),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_project_name_valid() {
        assert!(validate_project_name("my-project").is_ok());
        assert!(validate_project_name("hello_world").is_ok());
        assert!(validate_project_name("a").is_ok());
    }

    #[test]
    fn test_validate_project_name_empty() {
        assert!(validate_project_name("").is_err());
    }

    #[test]
    fn test_validate_project_name_too_long() {
        let name: String = "a".repeat(65);
        assert!(validate_project_name(&name).is_err());
    }

    #[test]
    fn test_validate_project_name_invalid_chars() {
        assert!(validate_project_name("my project").is_err());
        assert!(validate_project_name("hello/world").is_err());
    }

    #[test]
    fn test_validate_project_name_starts_with_hyphen() {
        assert!(validate_project_name("-hello").is_err());
    }

    #[test]
    fn test_validate_project_name_starts_with_digit() {
        assert!(validate_project_name("1hello").is_err());
    }

    /// A project named after a companion would declare two components with the
    /// same name. Rejected here, where the message can say what to do.
    #[test]
    fn rejects_a_project_named_after_a_companion() {
        for (template, colliding) in [
            (TemplateKind::Transform, "source"),
            (TemplateKind::Transform, "sink"),
            (TemplateKind::Source, "sink"),
            (TemplateKind::Sink, "source"),
        ] {
            let err = reject_companion_collision(template, colliding)
                .expect_err("{template:?} scaffolds a component called {colliding}");
            let rendered = format!("{err:?}");
            assert!(
                rendered.contains(colliding),
                "the error should name the collision: {rendered}"
            );
        }
    }

    /// The collision is specific: a scaffolded source ships only a sink, so a
    /// project called `source` is fine there, and templates that name every
    /// component themselves never collide at all.
    #[test]
    fn allows_a_name_that_does_not_collide() {
        assert!(reject_companion_collision(TemplateKind::Source, "source").is_ok());
        assert!(reject_companion_collision(TemplateKind::Sink, "sink").is_ok());
        assert!(reject_companion_collision(TemplateKind::Transform, "my-transform").is_ok());
        assert!(reject_companion_collision(TemplateKind::FullPipeline, "source").is_ok());
        assert!(reject_companion_collision(TemplateKind::Empty, "sink").is_ok());
    }

    /// `torvyn run` and `torvyn build` are only worth printing for a project
    /// that has something to build and run.
    #[test]
    fn next_steps_follow_what_the_template_generates() {
        assert!(TemplateKind::Transform.scaffolds_flow());
        assert!(TemplateKind::Source.scaffolds_flow());
        assert!(TemplateKind::Sink.scaffolds_flow());
        assert!(TemplateKind::FullPipeline.scaffolds_flow());
        assert!(!TemplateKind::Empty.scaffolds_flow());
    }
}
