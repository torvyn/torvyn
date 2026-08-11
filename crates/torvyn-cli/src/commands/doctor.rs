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
        failure: None,
        command: "doctor".into(),
        data: result,
        warnings: vec![],
    })
}

/// Check that a program is on `PATH` and report the version it prints.
///
/// `program` is the executable to run and `args` are the arguments to pass it.
/// The distinction matters: this used to take a combined slice and run
/// `args[0]` as the program, while its one caller passed the program
/// separately as `name` and only `["--version"]` as `args`. The result was an
/// attempt to execute a program literally called `--version`, so `torvyn
/// doctor` reported `rustc NOT found` on every machine — including ones that
/// had just compiled it.
///
/// A non-zero exit is also a failure. Spawning successfully only proves the
/// binary exists, not that it works.
fn check_command_version(category: &str, program: &str, args: &[&str]) -> DoctorCheck {
    let failure = |detail: String| DoctorCheck {
        category: category.into(),
        name: program.into(),
        passed: false,
        detail,
        fix: Some(format!("Install {program} and make sure it is on PATH")),
    };

    match std::process::Command::new(program).args(args).output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            DoctorCheck {
                category: category.into(),
                name: program.into(),
                passed: true,
                detail: if version.is_empty() {
                    "found".to_owned()
                } else {
                    version
                },
                fix: None,
            }
        }
        Ok(output) => failure(format!(
            "found but `{program} {}` exited with {}",
            args.join(" "),
            output
                .status
                .code()
                .map_or_else(|| "a signal".to_owned(), |c| format!("status {c}"))
        )),
        Err(_) => failure("NOT found".into()),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe used to run `args[0]` as the program and pass `program` as an
    /// argument, so every call executed a binary literally named `--version`
    /// and every tool was reported missing on every machine.
    #[test]
    fn finds_a_program_that_is_on_path() {
        // `rustc` is present wherever this test suite can be compiled.
        let check = check_command_version("Toolchain", "rustc", &["--version"]);
        assert!(check.passed, "rustc was not found: {}", check.detail);
        assert_eq!(check.name, "rustc");
        assert!(
            check.detail.contains("rustc"),
            "detail should carry the version string, got: {}",
            check.detail
        );
        assert!(check.fix.is_none(), "a passing check needs no fix");
    }

    #[test]
    fn reports_a_program_that_is_not_installed() {
        let check = check_command_version(
            "Toolchain",
            "torvyn-no-such-program-exists-here",
            &["--version"],
        );
        assert!(!check.passed);
        assert!(check.detail.contains("NOT found"), "{}", check.detail);
        assert!(
            check.fix.is_some(),
            "a failing check must say how to fix it"
        );
    }

    /// Spawning successfully only proves the binary exists. A tool that runs
    /// but exits non-zero is not usable, and saying "found" would be wrong.
    #[test]
    fn reports_a_program_that_exits_non_zero() {
        let check = check_command_version("Toolchain", "rustc", &["--definitely-not-a-flag"]);
        assert!(!check.passed);
        assert!(
            check.detail.contains("exited with"),
            "detail should say the command failed, got: {}",
            check.detail
        );
    }
}
