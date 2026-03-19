//! `torvyn init` — create a new Torvyn project.
//!
//! Scaffolds a complete project with WIT contracts, implementation stubs,
//! a Torvyn.toml manifest, and build configuration.

use crate::cli::InitArgs;
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
        eprintln!("    torvyn build              # Compile to WebAssembly component");
        eprintln!("    torvyn run --limit 10     # Run and see output");
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
    };

    Ok(CommandResult {
        success: true,
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
}
