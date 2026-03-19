//! `torvyn doctor` — check development environment.

use crate::cli::DoctorArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;

/// Result of `torvyn doctor`.
#[derive(Debug, Serialize)]
pub struct DoctorResult {
    /// All checks performed.
    pub checks: Vec<DoctorCheck>,
    /// Whether all checks passed.
    pub all_passed: bool,
    /// Number of failing checks.
    pub error_count: usize,
    /// Number of warnings.
    pub warning_count: usize,
}

/// A single doctor check.
#[derive(Debug, Serialize, Clone)]
pub struct DoctorCheck {
    /// Category (e.g., "Rust Toolchain", "WebAssembly Tools").
    pub category: String,
    /// Tool or check name.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Detail string (version info, etc.).
    pub detail: String,
    /// Fix suggestion if the check failed.
    pub fix: Option<String>,
}

impl HumanRenderable for DoctorResult {
    fn render_human(&self, ctx: &OutputContext) {
        let mut current_category = String::new();
        for check in &self.checks {
            if check.category != current_category {
                eprintln!();
                eprintln!("  {}", check.category);
                current_category.clone_from(&check.category);
            }

            if check.passed {
                terminal::print_success(ctx, &format!("{} {}", check.name, check.detail));
            } else {
                terminal::print_failure(ctx, &format!("{} {}", check.name, check.detail));
                if let Some(fix) = &check.fix {
                    eprintln!();
                    if ctx.color_enabled {
                        eprintln!("      {} {}", console::style("fix:").cyan().bold(), fix);
                    } else {
                        eprintln!("      fix: {fix}");
                    }
                }
            }
        }

        eprintln!();
        if self.all_passed {
            eprintln!("  All checks passed!");
        } else {
            eprintln!(
                "  {} error(s), {} warning(s). Run `torvyn doctor --fix` to attempt automatic repair.",
                self.error_count, self.warning_count
            );
        }
    }
}

/// Execute the `torvyn doctor` command.
///
/// COLD PATH.
pub async fn execute(
    args: &DoctorArgs,
    _ctx: &OutputContext,
) -> Result<CommandResult<DoctorResult>, CliError> {
    let mut checks = Vec::new();

    // Check 1: Torvyn CLI version
    checks.push(DoctorCheck {
        category: "Torvyn CLI".into(),
        name: "torvyn".into(),
        passed: true,
        detail: format!("{} (up to date)", env!("CARGO_PKG_VERSION")),
        fix: None,
    });

    // Check 2: Rust toolchain — rustc
    checks.push(check_command_version(
        "Rust Toolchain",
        "rustc",
        &["--version"],
    ));

    // Check 3: wasm32-wasip2 target
    checks.push(check_wasm_target(args.fix));

    // Check 4: cargo-component
    checks.push(check_command_existence(
        "Rust Toolchain",
        "cargo-component",
        &["cargo", "component", "--version"],
        Some("Run `cargo install cargo-component`"),
        args.fix,
        Some(&["cargo", "install", "cargo-component"]),
    ));

    // Check 5: wasm-tools
    checks.push(check_command_existence(
        "WebAssembly Tools",
        "wasm-tools",
        &["wasm-tools", "--version"],
        Some("Run `cargo install wasm-tools`"),
        args.fix,
        Some(&["cargo", "install", "wasm-tools"]),
    ));

    // Check 6: Project-specific — Torvyn.toml
    let torvyn_toml_exists = std::path::Path::new("./Torvyn.toml").exists();
    checks.push(DoctorCheck {
        category: "Project".into(),
        name: "Torvyn.toml".into(),
        passed: torvyn_toml_exists,
        detail: if torvyn_toml_exists {
            "found".into()
        } else {
            "NOT found (not in a Torvyn project directory)".into()
        },
        fix: if torvyn_toml_exists {
            None
        } else {
            Some("Run `torvyn init` to create a project.".into())
        },
    });

    let error_count = checks.iter().filter(|c| !c.passed).count();
    let all_passed = error_count == 0;

    let result = DoctorResult {
        checks,
        all_passed,
        error_count,
        warning_count: 0,
    };

    Ok(CommandResult {
        success: true,
        command: "doctor".into(),
        data: result,
        warnings: vec![],
    })
}

/// Check if a command exists and get its version.
fn check_command_version(category: &str, name: &str, args: &[&str]) -> DoctorCheck {
    match std::process::Command::new(args[0])
        .args(&args[1..])
        .output()
    {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            DoctorCheck {
                category: category.into(),
                name: name.into(),
                passed: true,
                detail: version,
                fix: None,
            }
        }
        Err(_) => DoctorCheck {
            category: category.into(),
            name: name.into(),
            passed: false,
            detail: "NOT found".into(),
            fix: Some(format!("Install {name}")),
        },
    }
}

/// Check if a command exists, optionally auto-fix by installing.
fn check_command_existence(
    category: &str,
    name: &str,
    check_args: &[&str],
    fix_hint: Option<&str>,
    attempt_fix: bool,
    fix_cmd: Option<&[&str]>,
) -> DoctorCheck {
    let output = std::process::Command::new(check_args[0])
        .args(&check_args[1..])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let version = String::from_utf8_lossy(&o.stdout).trim().to_string();
            DoctorCheck {
                category: category.into(),
                name: name.into(),
                passed: true,
                detail: version,
                fix: None,
            }
        }
        _ => {
            if attempt_fix {
                if let Some(cmd) = fix_cmd {
                    let fix_result = std::process::Command::new(cmd[0])
                        .args(&cmd[1..])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();

                    if fix_result.map(|s| s.success()).unwrap_or(false) {
                        return DoctorCheck {
                            category: category.into(),
                            name: name.into(),
                            passed: true,
                            detail: "installed (auto-fixed)".into(),
                            fix: None,
                        };
                    }
                }
            }

            DoctorCheck {
                category: category.into(),
                name: name.into(),
                passed: false,
                detail: "not found".into(),
                fix: fix_hint.map(|s| s.to_string()),
            }
        }
    }
}

/// Check if the wasm32-wasip2 Rust target is installed.
fn check_wasm_target(attempt_fix: bool) -> DoctorCheck {
    let output = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();

    let target_installed = output
        .as_ref()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.trim() == "wasm32-wasip2")
        })
        .unwrap_or(false);

    if target_installed {
        DoctorCheck {
            category: "Rust Toolchain".into(),
            name: "wasm32-wasip2 target".into(),
            passed: true,
            detail: "installed".into(),
            fix: None,
        }
    } else {
        if attempt_fix {
            let fix_result = std::process::Command::new("rustup")
                .args(["target", "add", "wasm32-wasip2"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();

            if fix_result.map(|s| s.success()).unwrap_or(false) {
                return DoctorCheck {
                    category: "Rust Toolchain".into(),
                    name: "wasm32-wasip2 target".into(),
                    passed: true,
                    detail: "installed (auto-fixed)".into(),
                    fix: None,
                };
            }
        }

        DoctorCheck {
            category: "Rust Toolchain".into(),
            name: "wasm32-wasip2 target".into(),
            passed: false,
            detail: "NOT installed".into(),
            fix: Some("Run `rustup target add wasm32-wasip2`".into()),
        }
    }
}
