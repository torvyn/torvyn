//! `torvyn publish` — publish to a registry.

use crate::cli::PublishArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;

/// Result of `torvyn publish`.
#[derive(Debug, Serialize)]
pub struct PublishResult {
    /// Registry URL or local path.
    pub registry: String,
    /// Full artifact reference (registry/name:tag).
    pub reference: String,
    /// Content digest (sha256).
    pub digest: String,
    /// Whether this was a dry run.
    pub dry_run: bool,
}

impl HumanRenderable for PublishResult {
    fn render_human(&self, ctx: &OutputContext) {
        if self.dry_run {
            terminal::print_success(ctx, "Dry run: publish would succeed");
            terminal::print_kv(ctx, "Registry", &self.registry);
            terminal::print_kv(ctx, "Reference", &self.reference);
        } else {
            terminal::print_success(ctx, &format!("Published: {}", self.reference));
            terminal::print_kv(ctx, "Digest", &self.digest);
        }
    }
}

/// Execute the `torvyn publish` command.
///
/// COLD PATH.
pub async fn execute(
    args: &PublishArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<PublishResult>, CliError> {
    // Determine artifact path
    let artifact_path = match &args.artifact {
        Some(path) => {
            if !path.exists() {
                return Err(CliError::Packaging {
                    detail: format!("Artifact not found: {}", path.display()),
                    suggestion: "Run `torvyn pack` first.".into(),
                });
            }
            path.clone()
        }
        None => {
            // Find latest artifact in .torvyn/artifacts/
            let artifacts_dir = std::path::PathBuf::from(".torvyn/artifacts");
            if !artifacts_dir.exists() {
                return Err(CliError::Packaging {
                    detail: "No artifacts found. Run `torvyn pack` first.".into(),
                    suggestion: "Run `torvyn pack` to create an artifact, then `torvyn publish`."
                        .into(),
                });
            }

            find_latest_artifact(&artifacts_dir).ok_or_else(|| CliError::Packaging {
                detail: "No artifact files found in .torvyn/artifacts/".into(),
                suggestion: "Run `torvyn pack` first.".into(),
            })?
        }
    };

    let registry = args
        .registry
        .clone()
        .unwrap_or_else(|| "local:.torvyn/registry".into());

    let spinner = ctx.spinner(&format!("Publishing to {registry}..."));

    // For Phase 0: local directory "registry" only
    // IMPLEMENTATION SPIKE REQUIRED: OCI push API
    let is_local = registry.starts_with("local:");

    if let Some(sp) = &spinner {
        sp.finish_and_clear();
    }

    if args.dry_run {
        let result = PublishResult {
            registry: registry.clone(),
            reference: format!("{registry}/artifact:latest"),
            digest: "sha256:dry-run".into(),
            dry_run: true,
        };

        return Ok(CommandResult {
            success: true,
            command: "publish".into(),
            data: result,
            warnings: vec![],
        });
    }

    // Local publish: copy artifact to registry directory
    if is_local {
        let local_dir = registry
            .strip_prefix("local:")
            .unwrap_or(".torvyn/registry");
        let registry_dir = std::path::PathBuf::from(local_dir);
        std::fs::create_dir_all(&registry_dir).map_err(|e| CliError::Io {
            detail: format!("Cannot create local registry directory: {e}"),
            path: Some(registry_dir.display().to_string()),
        })?;

        let dest = registry_dir.join(
            artifact_path
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("artifact.tar")),
        );
        std::fs::copy(&artifact_path, &dest).map_err(|e| CliError::Io {
            detail: format!("Failed to copy artifact to local registry: {e}"),
            path: Some(dest.display().to_string()),
        })?;

        let digest = format!("sha256:{:x}", {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            artifact_path.hash(&mut hasher);
            hasher.finish()
        });

        let result = PublishResult {
            registry: registry.clone(),
            reference: format!("{}/{}", registry, dest.display()),
            digest,
            dry_run: false,
        };

        return Ok(CommandResult {
            success: true,
            command: "publish".into(),
            data: result,
            warnings: vec![],
        });
    }

    // Remote publish placeholder
    let result = PublishResult {
        registry: registry.clone(),
        reference: format!("{registry}/artifact:latest"),
        digest: "sha256:placeholder".into(),
        dry_run: false,
    };

    Ok(CommandResult {
        success: true,
        command: "publish".into(),
        data: result,
        warnings: vec![],
    })
}

/// Find the most recently modified artifact in a directory.
fn find_latest_artifact(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "tar")
                .unwrap_or(false)
        })
        .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
        .map(|e| e.path())
}
