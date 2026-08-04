//! `torvyn publish` — publish an artifact to a registry.
//!
//! Only the local-directory registry is implemented. A remote push now fails
//! with an explanation instead of reporting a successful publish that never
//! happened; the digest it used to print for that case was the literal string
//! `sha256:placeholder`.
//!
//! The local path's digest was equally untrustworthy: it hashed the artifact's
//! *path* with `DefaultHasher` and labelled the 64-bit result `sha256:`. Two
//! different builds written to the same path digested identically, and the
//! value matched nothing any other tool would compute. Digests are now the
//! real SHA-256 of the artifact's bytes, which is what makes them worth
//! printing — a published reference can be verified against one.

use crate::cli::PublishArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::path::{Path, PathBuf};
use torvyn_packaging::ContentDigest;

/// Result of `torvyn publish`.
#[derive(Debug, Serialize)]
pub struct PublishResult {
    /// Registry URL or local path.
    pub registry: String,
    /// Full artifact reference (registry/name:tag).
    pub reference: String,
    /// SHA-256 digest of the artifact's bytes, as `sha256:{hex}`.
    pub digest: String,
    /// Size of the published artifact in bytes.
    pub size_bytes: u64,
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
        }
        terminal::print_kv(ctx, "Digest", &self.digest);
        terminal::print_kv(ctx, "Size", &terminal::format_bytes(self.size_bytes));
    }
}

/// Execute the `torvyn publish` command.
///
/// COLD PATH.
pub async fn execute(
    args: &PublishArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<PublishResult>, CliError> {
    let artifact_path = resolve_artifact(args)?;

    let registry = args
        .registry
        .clone()
        .unwrap_or_else(|| "local:.torvyn/registry".into());

    // A remote registry is not implemented. Fail before doing any work, so the
    // failure is about the registry rather than something incidental.
    let Some(local_dir) = registry.strip_prefix("local:") else {
        return Err(CliError::Packaging {
            detail: format!(
                "publishing to a remote registry is not implemented (requested `{registry}`)"
            ),
            suggestion: "Publish to a local directory registry instead, e.g. \
                         `--registry local:.torvyn/registry`."
                .into(),
        });
    };
    let local_dir = if local_dir.is_empty() {
        ".torvyn/registry"
    } else {
        local_dir
    };

    let spinner = ctx.spinner(&format!("Publishing to {registry}..."));

    // Digest the artifact as it is on disk. Doing this before the copy means a
    // dry run reports the same digest a real publish would.
    let digest = ContentDigest::of_file(&artifact_path).map_err(|e| CliError::Io {
        detail: format!("Cannot read artifact to compute its digest: {e}"),
        path: Some(artifact_path.display().to_string()),
    })?;
    let size_bytes = std::fs::metadata(&artifact_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let file_name = artifact_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact.torvyn".into());

    if args.dry_run {
        if let Some(sp) = &spinner {
            sp.finish_and_clear();
        }
        return Ok(CommandResult {
            success: true,
            command: "publish".into(),
            data: PublishResult {
                reference: format!("{registry}/{file_name}"),
                registry,
                digest: digest.prefixed,
                size_bytes,
                dry_run: true,
            },
            warnings: vec![],
        });
    }

    let registry_dir = PathBuf::from(local_dir);
    std::fs::create_dir_all(&registry_dir).map_err(|e| CliError::Io {
        detail: format!("Cannot create local registry directory: {e}"),
        path: Some(registry_dir.display().to_string()),
    })?;

    let dest = registry_dir.join(&file_name);
    std::fs::copy(&artifact_path, &dest).map_err(|e| CliError::Io {
        detail: format!("Failed to copy artifact to local registry: {e}"),
        path: Some(dest.display().to_string()),
    })?;

    // Verify what landed rather than trusting the copy. A registry entry whose
    // digest does not match what was published is worse than a failed publish,
    // because everything downstream treats the digest as authoritative.
    let written = ContentDigest::of_file(&dest).map_err(|e| CliError::Io {
        detail: format!("Cannot read the published artifact back: {e}"),
        path: Some(dest.display().to_string()),
    })?;
    if written.prefixed != digest.prefixed {
        return Err(CliError::Packaging {
            detail: format!(
                "published artifact does not match its source: expected {}, found {}",
                digest.prefixed, written.prefixed
            ),
            suggestion: "Check that nothing else is writing to the registry directory.".into(),
        });
    }

    if let Some(sp) = &spinner {
        sp.finish_and_clear();
    }

    Ok(CommandResult {
        success: true,
        command: "publish".into(),
        data: PublishResult {
            reference: format!("{registry}/{file_name}"),
            registry,
            digest: digest.prefixed,
            size_bytes,
            dry_run: false,
        },
        warnings: vec![],
    })
}

/// Determine which artifact to publish.
fn resolve_artifact(args: &PublishArgs) -> Result<PathBuf, CliError> {
    if let Some(path) = &args.artifact {
        if !path.exists() {
            return Err(CliError::Packaging {
                detail: format!("Artifact not found: {}", path.display()),
                suggestion: "Run `torvyn pack` first.".into(),
            });
        }
        return Ok(path.clone());
    }

    let artifacts_dir = PathBuf::from(".torvyn/artifacts");
    if !artifacts_dir.exists() {
        return Err(CliError::Packaging {
            detail: "No artifacts found. Run `torvyn pack` first.".into(),
            suggestion: "Run `torvyn pack` to create an artifact, then `torvyn publish`.".into(),
        });
    }

    find_latest_artifact(&artifacts_dir).ok_or_else(|| CliError::Packaging {
        detail: "No artifact files found in .torvyn/artifacts/".into(),
        suggestion: "Run `torvyn pack` first.".into(),
    })
}

/// Whether a file name looks like a Torvyn artifact.
///
/// `pack` writes `<name>-<version>.torvyn`. `.tar` is accepted as well so
/// artifacts produced before that extension settled still resolve.
fn is_artifact(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("torvyn" | "tar")
    )
}

/// Find the most recently modified artifact in a directory.
fn find_latest_artifact(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| is_artifact(&e.path()))
        .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
        .map(|e| e.path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_artifact_extensions() {
        assert!(is_artifact(Path::new("source-0.1.0.torvyn")));
        assert!(is_artifact(Path::new("legacy.tar")));
        assert!(!is_artifact(Path::new("component.wasm")));
        assert!(!is_artifact(Path::new("Torvyn.toml")));
        assert!(!is_artifact(Path::new("no-extension")));
    }

    #[test]
    fn finds_nothing_in_an_empty_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(find_latest_artifact(dir.path()).is_none());
    }

    #[test]
    fn finds_the_artifact_among_other_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("component.wasm"), b"not an artifact").unwrap();
        std::fs::write(dir.path().join("source-0.1.0.torvyn"), b"artifact").unwrap();

        let found = find_latest_artifact(dir.path()).expect("artifact");
        assert_eq!(found.file_name().unwrap(), "source-0.1.0.torvyn");
    }

    /// The digest must be the artifact's content, not its path — the previous
    /// implementation hashed the path, so a rebuilt artifact at the same path
    /// kept the digest of the one it replaced.
    #[test]
    fn digest_tracks_content_not_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("artifact.torvyn");

        std::fs::write(&path, b"first build").unwrap();
        let first = ContentDigest::of_file(&path).expect("digest");

        std::fs::write(&path, b"second build").unwrap();
        let second = ContentDigest::of_file(&path).expect("digest");

        assert_ne!(first.prefixed, second.prefixed);
        assert_eq!(first.hex.len(), 64, "a sha256 digest is 64 hex characters");
        assert!(ContentDigest::parse(&second.prefixed).is_some());
    }
}
