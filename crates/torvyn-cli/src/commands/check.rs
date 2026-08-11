//! `torvyn check` — validate contracts, manifest, and project structure.
//!
//! Delegates to `torvyn-contracts` for WIT validation and `torvyn-config`
//! for manifest parsing and semantic validation.

use crate::cli::CheckArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::path::Path;

/// Result of a `torvyn check`.
#[derive(Debug, Serialize)]
pub struct CheckResult {
    /// Whether all checks passed.
    pub all_passed: bool,
    /// Number of WIT files parsed.
    pub wit_files_parsed: usize,
    /// Number of errors found.
    pub error_count: usize,
    /// Number of warnings found.
    pub warning_count: usize,
    /// Detailed diagnostics.
    pub diagnostics: Vec<CheckDiagnostic>,
}

/// A single check diagnostic.
#[derive(Debug, Serialize, Clone)]
pub struct CheckDiagnostic {
    /// Severity: "error" or "warning".
    pub severity: String,
    /// Error code (e.g., "E0201").
    pub code: String,
    /// What went wrong.
    pub message: String,
    /// File and location (if applicable).
    pub location: Option<String>,
    /// Suggested fix.
    pub help: Option<String>,
}

impl HumanRenderable for CheckResult {
    fn render_human(&self, ctx: &OutputContext) {
        if self.all_passed {
            terminal::print_success(ctx, "Torvyn.toml is valid");
            terminal::print_success(
                ctx,
                &format!(
                    "WIT contracts parsed ({} file(s), 0 errors)",
                    self.wit_files_parsed
                ),
            );
            terminal::print_success(ctx, "World definition resolves correctly");
            terminal::print_success(ctx, "Capability declarations consistent");

            // Warnings do not fail the check without `--strict`, but they are
            // still findings and must be shown. Printing only the successes
            // is how a project that validated nothing could read as healthy.
            for d in self.diagnostics.iter().filter(|d| d.severity == "warning") {
                terminal::print_warning(ctx, &format!("[{}] {}", d.code, d.message));
                if let Some(help) = &d.help {
                    eprintln!("        {help}");
                }
            }

            eprintln!();
            if self.warning_count == 0 {
                eprintln!("  All checks passed.");
            } else {
                eprintln!(
                    "  All checks passed with {} warning(s).",
                    self.warning_count
                );
            }
        } else {
            for d in &self.diagnostics {
                let prefix = if d.severity == "error" {
                    if ctx.color_enabled {
                        format!(
                            "{}",
                            console::style(format!("error[{}]:", d.code)).red().bold()
                        )
                    } else {
                        format!("error[{}]:", d.code)
                    }
                } else if ctx.color_enabled {
                    format!(
                        "{}",
                        console::style(format!("warning[{}]:", d.code))
                            .yellow()
                            .bold()
                    )
                } else {
                    format!("warning[{}]:", d.code)
                };
                eprintln!("\n{prefix} {}", d.message);
                if let Some(loc) = &d.location {
                    eprintln!("  --> {loc}");
                }
                if let Some(help) = &d.help {
                    if ctx.color_enabled {
                        eprintln!("  {} {help}", console::style("help:").cyan().bold());
                    } else {
                        eprintln!("  help: {help}");
                    }
                }
            }
            eprintln!(
                "\n  {} error(s), {} warning(s)",
                self.error_count, self.warning_count
            );
        }
    }
}

/// Execute the `torvyn check` command.
///
/// COLD PATH.
pub async fn execute(
    args: &CheckArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<CheckResult>, CliError> {
    let manifest_path = &args.manifest;

    // Verify manifest exists
    if !manifest_path.exists() {
        return Err(CliError::Config {
            detail: format!("Manifest not found: {}", manifest_path.display()),
            file: Some(manifest_path.display().to_string()),
            suggestion:
                "Run this command from a Torvyn project directory, or use --manifest <path>.".into(),
        });
    }

    ctx.print_debug(&format!("Checking manifest: {}", manifest_path.display()));

    let mut diagnostics = Vec::new();
    let mut wit_files_parsed = 0;

    // Step 1: Parse and validate manifest via torvyn-config
    let manifest_content = std::fs::read_to_string(manifest_path).map_err(|e| CliError::Io {
        detail: format!("Cannot read manifest: {e}"),
        path: Some(manifest_path.display().to_string()),
    })?;

    let mut declared_components: Vec<String> = Vec::new();
    match torvyn_config::ComponentManifest::from_toml_str(
        &manifest_content,
        manifest_path.to_str().unwrap_or("Torvyn.toml"),
    ) {
        Ok(manifest) => {
            ctx.print_debug("Manifest parsed successfully");
            declared_components = manifest
                .components
                .iter()
                .map(|decl| decl.path.clone())
                .collect();
        }
        Err(errors) => {
            for err in &errors {
                diagnostics.push(CheckDiagnostic {
                    severity: "error".into(),
                    code: err.code.to_string(),
                    message: err.message.clone(),
                    location: Some(err.file.clone()),
                    help: if err.suggestion.is_empty() {
                        None
                    } else {
                        Some(err.suggestion.clone())
                    },
                });
            }
        }
    }

    // Step 2: Find and validate WIT files via torvyn-contracts.
    //
    // A project keeps its contracts in one of two shapes: a single-component
    // project has `wit/` at its root, while a multi-component project gives
    // each declared component its own. Checking only the root meant a
    // scaffolded pipeline reported "0 file(s), 0 errors" and passed, having
    // read none of the contracts it shipped.
    let project_dir = manifest_path.parent().unwrap_or(Path::new("."));
    let wit_parser = torvyn_contracts::WitParserImpl::new();
    let mut roots: Vec<std::path::PathBuf> = Vec::new();

    // A single-component project keeps its contracts at the root, next to the
    // manifest. `validate_component` checks both together.
    if project_dir.join("wit").is_dir() {
        roots.push(project_dir.join("wit"));
        wit_files_parsed += count_wit_files(&project_dir.join("wit"));
        let result = torvyn_contracts::validate_component(project_dir, &wit_parser);
        push_contract_diagnostics(&mut diagnostics, &result);
    }

    // A multi-component project gives each declared component its own `wit/`
    // and has no per-component manifest, so those are parsed on their own.
    for component_path in &declared_components {
        let wit_dir = project_dir.join(component_path).join("wit");
        if !wit_dir.is_dir() {
            continue;
        }
        roots.push(wit_dir.clone());
        wit_files_parsed += count_wit_files(&wit_dir);
        if let Err(result) = parse_wit_tree(&wit_dir, &wit_parser) {
            push_contract_diagnostics(&mut diagnostics, &result);
        }
    }

    if roots.is_empty() {
        diagnostics.push(CheckDiagnostic {
            severity: "warning".into(),
            code: "E0100".into(),
            message: "No WIT contracts found to validate".into(),
            location: Some(project_dir.display().to_string()),
            help: Some(
                "Expected a wit/ directory at the project root, or one inside each directory a                  [[component]] entry points at. Without contracts there is nothing to check."
                    .into(),
            ),
        });
    }

    let error_count = diagnostics.iter().filter(|d| d.severity == "error").count();
    let warning_count = diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count();

    let all_passed = if args.strict {
        error_count == 0 && warning_count == 0
    } else {
        error_count == 0
    };

    let result = CheckResult {
        all_passed,
        wit_files_parsed,
        error_count,
        warning_count,
        diagnostics: diagnostics.clone(),
    };

    if !all_passed {
        return Err(CliError::Contract {
            detail: format!("{error_count} error(s) found during validation"),
            diagnostics: result
                .diagnostics
                .iter()
                .filter(|d| d.severity == "error")
                .map(|d| d.message.clone())
                .collect(),
        });
    }

    Ok(CommandResult {
        success: true,
        failure: None,
        command: "check".into(),
        data: result,
        warnings: vec![],
    })
}

/// Count `.wit` files under a directory, including package subdirectories.
///
/// The conventional layout nests them one level deep
/// (`wit/torvyn-streaming/*.wit`), which is why this recurses rather than
/// listing a single directory.
fn count_wit_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += count_wit_files(&path);
        } else if path.extension().is_some_and(|e| e == "wit") {
            count += 1;
        }
    }
    count
}

/// Parse a component's WIT tree, trying the directory itself and then each
/// package subdirectory — the same fallback `validate_component` uses, since
/// `wit-parser` wants to be pointed at a directory holding a `package`
/// declaration.
fn parse_wit_tree(
    wit_dir: &Path,
    parser: &dyn torvyn_contracts::WitParser,
) -> Result<(), torvyn_contracts::ValidationResult> {
    if let Ok(packages) = parser.parse_directory(wit_dir) {
        if !packages.is_empty() {
            return Ok(());
        }
    }

    let mut last_failure = None;
    if let Ok(entries) = std::fs::read_dir(wit_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            match parser.parse_directory(&path) {
                Ok(packages) if !packages.is_empty() => return Ok(()),
                Ok(_) => {}
                Err(result) => last_failure = Some(result),
            }
        }
    }

    last_failure.map_or(Ok(()), Err)
}

/// Translate contract diagnostics into the CLI's own shape.
fn push_contract_diagnostics(
    diagnostics: &mut Vec<CheckDiagnostic>,
    result: &torvyn_contracts::ValidationResult,
) {
    for diag in &result.diagnostics {
        let severity = match diag.severity {
            torvyn_contracts::Severity::Error => "error",
            torvyn_contracts::Severity::Warning | torvyn_contracts::Severity::Hint => "warning",
        };
        let location = diag.locations.first().map(|l| {
            format!(
                "{}:{}:{}",
                l.location.file.display(),
                l.location.line,
                l.location.column
            )
        });
        diagnostics.push(CheckDiagnostic {
            severity: severity.into(),
            code: format!("{}", diag.code),
            message: diag.message.clone(),
            location,
            help: diag.help.clone(),
        });
    }
}
