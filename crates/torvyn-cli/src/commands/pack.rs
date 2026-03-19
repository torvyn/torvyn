//! `torvyn pack` — package as OCI artifact.

use crate::cli::PackArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::path::PathBuf;

/// Result of `torvyn pack`.
#[derive(Debug, Serialize)]
pub struct PackResult {
    /// Component or project name.
    pub name: String,
    /// Version string.
    pub version: String,
    /// Path to the created artifact file.
    pub artifact_path: PathBuf,
    /// Size of the artifact in bytes.
    pub artifact_size_bytes: u64,
    /// Layers in the packed artifact.
    pub layers: Vec<PackLayer>,
}

/// A single layer in the packed artifact.
#[derive(Debug, Serialize)]
pub struct PackLayer {
    /// Layer name (e.g., "component", "contracts", "metadata").
    pub name: String,
    /// Size of the layer in bytes.
    pub size_bytes: u64,
}

impl HumanRenderable for PackResult {
    fn render_human(&self, ctx: &OutputContext) {
        terminal::print_success(ctx, &format!("Packed: {}:{}", self.name, self.version));
        terminal::print_kv(ctx, "Artifact", &self.artifact_path.display().to_string());
        terminal::print_kv(
            ctx,
            "Size",
            &terminal::format_bytes(self.artifact_size_bytes),
        );
        if !self.layers.is_empty() {
            eprintln!("  Layers:");
            for layer in &self.layers {
                eprintln!(
                    "    - {} ({})",
                    layer.name,
                    terminal::format_bytes(layer.size_bytes)
                );
            }
        }
    }
}

/// Execute the `torvyn pack` command.
///
/// COLD PATH.
pub async fn execute(
    args: &PackArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<PackResult>, CliError> {
    let manifest_path = &args.manifest;

    if !manifest_path.exists() {
        return Err(CliError::Config {
            detail: format!("Manifest not found: {}", manifest_path.display()),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Run this command from a Torvyn project directory.".into(),
        });
    }

    let spinner = ctx.spinner("Checking contracts...");

    // Validate first
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

    if let Some(sp) = &spinner {
        sp.finish_and_clear();
    }

    let project_dir = manifest_path.parent().unwrap_or(std::path::Path::new("."));

    let output_dir = args
        .output
        .clone()
        .unwrap_or_else(|| project_dir.join(".torvyn").join("artifacts"));

    std::fs::create_dir_all(&output_dir).map_err(|e| CliError::Io {
        detail: format!("Cannot create output directory: {e}"),
        path: Some(output_dir.display().to_string()),
    })?;

    let name = manifest.torvyn.name.clone();
    let version = manifest.torvyn.version.clone();
    let tag = args.tag.clone().unwrap_or_else(|| version.clone());

    // Placeholder: write manifest as minimal artifact
    // IMPLEMENTATION SPIKE REQUIRED: torvyn_packaging::assemble_artifact API
    let artifact_filename = format!("{name}-{tag}.tar");
    let artifact_path = output_dir.join(&artifact_filename);

    let artifact_json = serde_json::json!({
        "name": name,
        "version": version,
        "tag": tag,
    });
    std::fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&artifact_json).unwrap(),
    )
    .map_err(|e| CliError::Io {
        detail: format!("Failed to write artifact: {e}"),
        path: Some(artifact_path.display().to_string()),
    })?;

    let artifact_size = std::fs::metadata(&artifact_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let result = PackResult {
        name,
        version,
        artifact_path,
        artifact_size_bytes: artifact_size,
        layers: vec![],
    };

    Ok(CommandResult {
        success: true,
        command: "pack".into(),
        data: result,
        warnings: vec![],
    })
}
