//! `torvyn build` — compile a project's declared components to WebAssembly.
//!
//! Reads the manifest's `[[component]]` declarations, builds each one with the
//! toolchain for its language, and copies the resulting Component Model binary
//! to `.torvyn/build/<name>.wasm`.
//!
//! That destination is the contract between this command and `torvyn run`: a
//! flow node naming a component resolves to exactly this path, so `run` needs
//! nothing from a component's own build files to find its output. It is also
//! why the copy happens at all rather than `run` reaching into a component's
//! `target/` directory — the compiler's output name is derived from the
//! component's package name, not from the manifest, and the two need not
//! match.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Serialize;
use torvyn_config::manifest::{BuildConfig, ComponentDecl};
use torvyn_pipeline::{artifact_path, BUILD_DIR};

use crate::cli::BuildArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};

/// Result of `torvyn build`.
#[derive(Debug, Serialize)]
pub struct BuildResult {
    /// One entry per component built, in manifest order.
    pub components: Vec<BuiltComponent>,
    /// Directory the artifacts were written to, relative to the project root.
    pub output_dir: String,
    /// Total wall-clock seconds.
    pub elapsed_secs: f64,
}

/// One component's build outcome.
#[derive(Debug, Serialize)]
pub struct BuiltComponent {
    /// Component name, as declared and as `run` will look it up.
    pub name: String,
    /// Source directory, relative to the project root.
    pub source: String,
    /// Where the artifact was written.
    pub artifact: String,
    /// Size of the artifact in bytes.
    pub size_bytes: u64,
    /// Seconds spent building this component.
    pub elapsed_secs: f64,
}

impl HumanRenderable for BuildResult {
    fn render_human(&self, ctx: &OutputContext) {
        for component in &self.components {
            terminal::print_success(
                ctx,
                &format!(
                    "{} → {} ({:.1} KiB, {:.1}s)",
                    component.name,
                    component.artifact,
                    component.size_bytes as f64 / 1024.0,
                    component.elapsed_secs,
                ),
            );
        }
        eprintln!();
        eprintln!(
            "  Built {} component(s) into {} in {:.1}s.",
            self.components.len(),
            self.output_dir,
            self.elapsed_secs,
        );
    }
}

/// Execute the `torvyn build` command.
///
/// COLD PATH — invokes external build toolchains.
///
/// # Errors
/// - [`CliError::Config`] if the manifest is missing, unparsable, or declares
///   no components.
/// - [`CliError::Io`] if an artifact cannot be located or copied.
/// - [`CliError::Runtime`] if a component's build command fails.
pub fn execute(
    args: &BuildArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<BuildResult>, CliError> {
    let manifest_path = &args.manifest;

    if !manifest_path.exists() {
        return Err(CliError::Config {
            detail: format!("Manifest not found: {}", manifest_path.display()),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Run this command from a Torvyn project directory, or pass \
                         --manifest <PATH>."
                .into(),
        });
    }

    let manifest_content = std::fs::read_to_string(manifest_path).map_err(|e| CliError::Io {
        detail: e.to_string(),
        path: Some(manifest_path.display().to_string()),
    })?;

    let manifest = torvyn_config::ComponentManifest::from_toml_str(
        &manifest_content,
        manifest_path.to_str().unwrap_or("Torvyn.toml"),
    )
    .map_err(|errors| CliError::Config {
        detail: format!("Manifest has {} error(s)", errors.len()),
        file: Some(manifest_path.display().to_string()),
        suggestion: "Run `torvyn check` first.".into(),
    })?;

    let project_root = project_root_of(manifest_path);

    // Select which components to build.
    let selected: Vec<&ComponentDecl> = match &args.component {
        Some(name) => {
            let found: Vec<&ComponentDecl> = manifest
                .components
                .iter()
                .filter(|decl| &decl.name == name)
                .collect();
            if found.is_empty() {
                return Err(CliError::Config {
                    detail: format!("No component named '{name}' is declared in the manifest"),
                    file: Some(manifest_path.display().to_string()),
                    suggestion: if manifest.components.is_empty() {
                        "The manifest declares no components. Add a [[component]] entry with a \
                         name and the path to its source."
                            .into()
                    } else {
                        format!(
                            "Declared components: {}",
                            manifest
                                .components
                                .iter()
                                .map(|d| d.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    },
                });
            }
            found
        }
        None => manifest.components.iter().collect(),
    };

    if selected.is_empty() {
        return Err(CliError::Config {
            detail: "The manifest declares no components to build".into(),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Add a [[component]] entry with a name and the path to its source. A \
                         project whose flow nodes reference components by `file://` URI does not \
                         need this command."
                .into(),
        });
    }

    let output_dir = project_root.join(BUILD_DIR);
    std::fs::create_dir_all(&output_dir).map_err(|e| CliError::Io {
        detail: format!("Cannot create build output directory: {e}"),
        path: Some(output_dir.display().to_string()),
    })?;

    let release = if args.debug {
        false
    } else {
        manifest.build.release
    };
    let started = Instant::now();
    let mut built = Vec::with_capacity(selected.len());

    for decl in selected {
        if !ctx.quiet && ctx.format == crate::cli::OutputFormat::Human {
            eprintln!("▶ Building {}", decl.name);
        }
        built.push(build_one(
            decl,
            &project_root,
            &manifest.build,
            release,
            manifest_path,
        )?);
    }

    Ok(CommandResult {
        success: true,
        command: "build".into(),
        data: BuildResult {
            components: built,
            output_dir: BUILD_DIR.to_owned(),
            elapsed_secs: started.elapsed().as_secs_f64(),
        },
        warnings: Vec::new(),
    })
}

/// Build one component and place its artifact where `run` will look for it.
fn build_one(
    decl: &ComponentDecl,
    project_root: &Path,
    build: &BuildConfig,
    release: bool,
    manifest_path: &Path,
) -> Result<BuiltComponent, CliError> {
    let source_dir = project_root.join(&decl.path);
    if !source_dir.is_dir() {
        return Err(CliError::Config {
            detail: format!(
                "Component '{}' declares path '{}', which is not a directory",
                decl.name, decl.path
            ),
            file: Some(manifest_path.display().to_string()),
            suggestion: format!(
                "Expected to find the component's source at {}. Fix the `path` in its \
                 [[component]] entry.",
                source_dir.display()
            ),
        });
    }

    let started = Instant::now();
    run_build_command(decl, &source_dir, build, release)?;

    let produced = locate_artifact(decl, &source_dir, build, release)?;
    let destination = artifact_path(project_root, &decl.name);
    std::fs::copy(&produced, &destination).map_err(|e| CliError::Io {
        detail: format!(
            "Cannot copy the built component from {} to {}: {e}",
            produced.display(),
            destination.display()
        ),
        path: Some(destination.display().to_string()),
    })?;

    let size_bytes = std::fs::metadata(&destination)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(BuiltComponent {
        name: decl.name.clone(),
        source: decl.path.clone(),
        artifact: format!("{BUILD_DIR}/{}.wasm", decl.name),
        size_bytes,
        elapsed_secs: started.elapsed().as_secs_f64(),
    })
}

/// Invoke the component's build toolchain.
fn run_build_command(
    decl: &ComponentDecl,
    source_dir: &Path,
    build: &BuildConfig,
    release: bool,
) -> Result<(), CliError> {
    let (program, args) = build_invocation(decl, build, release)?;

    let status = Command::new(&program)
        .args(&args)
        .current_dir(source_dir)
        .status()
        .map_err(|e| CliError::Runtime {
            detail: format!(
                "Cannot run `{program}` to build component '{}': {e}",
                decl.name
            ),
            context: Some(missing_tool_help(&program)),
        })?;

    if !status.success() {
        return Err(CliError::Runtime {
            detail: format!(
                "Building component '{}' failed: `{program} {}` exited with {}",
                decl.name,
                args.join(" "),
                status
                    .code()
                    .map_or_else(|| "a signal".to_owned(), |c| format!("status {c}")),
            ),
            context: Some(format!("in {}", source_dir.display())),
        });
    }

    Ok(())
}

/// Decide how to build a component: its explicit `build_command` if it has
/// one, otherwise the default toolchain for its language.
fn build_invocation(
    decl: &ComponentDecl,
    build: &BuildConfig,
    release: bool,
) -> Result<(String, Vec<String>), CliError> {
    if let Some(command) = &decl.build_command {
        let mut parts = command.split_whitespace().map(str::to_owned);
        let program = parts.next().ok_or_else(|| CliError::Config {
            detail: format!("Component '{}' has an empty build_command", decl.name),
            file: None,
            suggestion: "Set build_command to a command line, or remove it to use the default \
                         toolchain for the component's language."
                .into(),
        })?;
        return Ok((program, parts.collect()));
    }

    match decl.language.as_str() {
        "rust" => {
            let mut args = vec![
                "component".to_owned(),
                "build".to_owned(),
                "--target".to_owned(),
                build.target.clone(),
            ];
            if release {
                args.push("--release".to_owned());
            }
            args.extend(build.extra_args.iter().cloned());
            Ok(("cargo".to_owned(), args))
        }
        other => Err(CliError::Config {
            detail: format!(
                "Component '{}' declares language '{other}', which has no default build toolchain",
                decl.name
            ),
            file: None,
            suggestion: format!(
                "Rust components build with `cargo component` out of the box. For {other}, set \
                 `build_command` on the [[component]] entry to the command that produces a \
                 WebAssembly Component — for example a TinyGo or componentize-py invocation."
            ),
        }),
    }
}

/// Help text for a build tool that could not be launched.
fn missing_tool_help(program: &str) -> String {
    match program {
        "cargo" => "Install the Rust toolchain, then `cargo install cargo-component --locked` \
                    and `rustup target add wasm32-wasip2`. `torvyn doctor` checks this."
            .to_owned(),
        other => format!("Ensure `{other}` is installed and on PATH."),
    }
}

/// Find the `.wasm` a build produced.
///
/// `cargo component` currently emits under `wasm32-wasip1/` even when asked
/// for `wasm32-wasip2` — the core module is adapted in-process and the result
/// is still a valid Component Model artifact — so both directories are probed.
/// Within a directory the component's own package name is preferred, and a
/// lone `.wasm` is accepted as a fallback so a custom `build_command` that
/// names its output differently still works.
fn locate_artifact(
    decl: &ComponentDecl,
    source_dir: &Path,
    build: &BuildConfig,
    release: bool,
) -> Result<PathBuf, CliError> {
    let profile = if release { "release" } else { "debug" };
    let mut searched = Vec::new();

    let mut candidate_dirs = vec![source_dir.join("target").join(&build.target).join(profile)];
    if build.target == "wasm32-wasip2" {
        candidate_dirs.push(
            source_dir
                .join("target")
                .join("wasm32-wasip1")
                .join(profile),
        );
    }

    let expected_stem = package_name(source_dir).map(|name| name.replace('-', "_"));

    for dir in &candidate_dirs {
        searched.push(dir.display().to_string());
        if let Some(stem) = &expected_stem {
            let named = dir.join(format!("{stem}.wasm"));
            if named.is_file() {
                return Ok(named);
            }
        }
        if let Some(only) = sole_wasm_in(dir) {
            return Ok(only);
        }
    }

    Err(CliError::Io {
        detail: format!(
            "Component '{}' built successfully but no WebAssembly artifact was found. Looked in: \
             {}",
            decl.name,
            searched.join(", ")
        ),
        path: Some(source_dir.display().to_string()),
    })
}

/// The `[package] name` from a Cargo manifest, if the directory has one.
fn package_name(source_dir: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(source_dir.join("Cargo.toml")).ok()?;
    let parsed: toml::Value = manifest.parse().ok()?;
    parsed
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

/// The single `.wasm` in a directory, or `None` if there are none or several.
fn sole_wasm_in(dir: &Path) -> Option<PathBuf> {
    let mut found = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "wasm") && path.is_file() {
            if found.is_some() {
                // Ambiguous: refuse to guess.
                return None;
            }
            found = Some(path);
        }
    }
    found
}

/// The directory holding the manifest, which is the project root.
fn project_root_of(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, language: &str) -> ComponentDecl {
        ComponentDecl {
            name: name.to_owned(),
            path: format!("components/{name}"),
            language: language.to_owned(),
            ..ComponentDecl::default()
        }
    }

    #[test]
    fn rust_components_build_with_cargo_component() {
        let build = BuildConfig::default();
        let (program, args) = build_invocation(&decl("source", "rust"), &build, true).unwrap();
        assert_eq!(program, "cargo");
        assert_eq!(
            args,
            vec![
                "component",
                "build",
                "--target",
                "wasm32-wasip2",
                "--release"
            ]
        );
    }

    #[test]
    fn debug_profile_drops_the_release_flag() {
        let build = BuildConfig::default();
        let (_, args) = build_invocation(&decl("source", "rust"), &build, false).unwrap();
        assert!(!args.contains(&"--release".to_owned()));
    }

    #[test]
    fn extra_args_and_target_come_from_the_build_table() {
        let build = BuildConfig {
            target: "wasm32-wasip1".to_owned(),
            extra_args: vec!["--features".to_owned(), "fast".to_owned()],
            release: true,
        };
        let (_, args) = build_invocation(&decl("source", "rust"), &build, true).unwrap();
        assert!(args.windows(2).any(|w| w == ["--target", "wasm32-wasip1"]));
        assert!(args.windows(2).any(|w| w == ["--features", "fast"]));
    }

    #[test]
    fn an_explicit_build_command_wins() {
        let mut d = decl("go-source", "go");
        d.build_command = Some("tinygo build -target=wasip2 -o out.wasm .".to_owned());
        let (program, args) = build_invocation(&d, &BuildConfig::default(), true).unwrap();
        assert_eq!(program, "tinygo");
        assert_eq!(args, vec!["build", "-target=wasip2", "-o", "out.wasm", "."]);
    }

    #[test]
    fn an_unsupported_language_says_how_to_proceed() {
        let err = build_invocation(&decl("py", "python"), &BuildConfig::default(), true)
            .expect_err("no default toolchain for python");
        let message = format!("{err:?}");
        assert!(
            message.contains("build_command"),
            "the error must point at the escape hatch: {message}"
        );
    }

    #[test]
    fn project_root_is_the_manifest_directory() {
        assert_eq!(
            project_root_of(Path::new("/p/Torvyn.toml")),
            PathBuf::from("/p")
        );
        // A bare filename means the current directory, not the filesystem root.
        assert_eq!(
            project_root_of(Path::new("Torvyn.toml")),
            PathBuf::from(".")
        );
    }

    #[test]
    fn sole_wasm_refuses_to_guess_between_several() {
        let dir = std::env::temp_dir().join(format!("torvyn-build-sole-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert!(sole_wasm_in(&dir).is_none(), "no candidates");

        std::fs::write(dir.join("a.wasm"), b"\0asm").unwrap();
        assert_eq!(sole_wasm_in(&dir), Some(dir.join("a.wasm")));

        std::fs::write(dir.join("b.wasm"), b"\0asm").unwrap();
        assert!(sole_wasm_in(&dir).is_none(), "ambiguous, must not guess");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn package_name_is_read_from_cargo_toml() {
        let dir = std::env::temp_dir().join(format!("torvyn-build-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(package_name(&dir), None, "no Cargo.toml");

        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"hello-source\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(package_name(&dir), Some("hello-source".to_owned()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
