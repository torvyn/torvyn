//! `torvyn pack` — assemble each component into a distributable artifact.
//!
//! An artifact is a gzip-compressed tar holding the component binary, the
//! artifact manifest, the WIT contracts it was built against, and a SLSA
//! in-toto provenance record. `torvyn_packaging::pack_with_manifest` does the
//! assembly; this command's job is to gather the inputs and derive an artifact
//! manifest for each component.
//!
//! # Project manifests and artifact manifests
//!
//! They are different documents and the distinction is the reason this command
//! is more than a function call. A project's `Torvyn.toml` describes a
//! workspace: which components exist, how they are wired into flows, what
//! capabilities each flow node is granted. An artifact manifest describes one
//! packaged component: its identity, the contract packages it implements, the
//! capabilities it requires, and what built it. Packing derives the latter
//! from the former, once per component.
//!
//! # What this replaced
//!
//! The previous implementation wrote a 57-byte JSON object — name, version,
//! tag — to a file named `<name>-<tag>.tar`, which was not a tar and contained
//! no component, and reported success. Its comment asked for an
//! "implementation spike" on an `assemble_artifact` API; the API existed under
//! the name `pack`.

use std::path::{Path, PathBuf};

use serde::Serialize;
use torvyn_config::manifest::ComponentDecl;
use torvyn_packaging::manifest::{ArtifactManifest, BuildInfoSpec, CapabilitiesSpec};
use torvyn_packaging::{pack_with_manifest, PackInput, ProvenanceRecord};
use torvyn_pipeline::artifact_path;

use crate::cli::PackArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};

/// Result of `torvyn pack`.
#[derive(Debug, Serialize)]
pub struct PackResult {
    /// One entry per component packed, in manifest order.
    pub artifacts: Vec<PackedArtifact>,
    /// Directory the artifacts were written to.
    pub output_dir: PathBuf,
}

/// One packed component.
#[derive(Debug, Serialize)]
pub struct PackedArtifact {
    /// Component name.
    pub name: String,
    /// Version, from the project manifest.
    pub version: String,
    /// Path to the created artifact.
    pub artifact_path: PathBuf,
    /// Size of the artifact in bytes.
    pub artifact_size_bytes: u64,
    /// SHA-256 digest of the artifact, `sha256:<hex>`.
    pub digest: String,
    /// Digest of each layer inside the artifact.
    pub layers: Vec<PackLayer>,
}

/// A single layer inside a packed artifact.
#[derive(Debug, Serialize)]
pub struct PackLayer {
    /// Layer name, as it appears in the archive.
    pub name: String,
    /// SHA-256 digest of the layer's contents.
    pub digest: String,
}

impl HumanRenderable for PackResult {
    fn render_human(&self, ctx: &OutputContext) {
        for artifact in &self.artifacts {
            terminal::print_success(
                ctx,
                &format!("Packed {}:{}", artifact.name, artifact.version),
            );
            terminal::print_kv(
                ctx,
                "  Artifact",
                &artifact.artifact_path.display().to_string(),
            );
            terminal::print_kv(
                ctx,
                "  Size",
                &terminal::format_bytes(artifact.artifact_size_bytes),
            );
            terminal::print_kv(ctx, "  Digest", &artifact.digest);
            for layer in &artifact.layers {
                terminal::print_kv(ctx, &format!("  {}", layer.name), &layer.digest);
            }
        }
        eprintln!();
        eprintln!(
            "  Packed {} artifact(s) into {}.",
            self.artifacts.len(),
            self.output_dir.display()
        );
    }
}

/// Execute the `torvyn pack` command.
///
/// COLD PATH.
///
/// # Errors
/// - [`CliError::Config`] if the manifest is missing, unparsable, declares no
///   components, or names a component that is not declared.
/// - [`CliError::Packaging`] if a component has not been built, or assembly fails.
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

    let project_dir = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    let selected = select_components(&manifest, args.component.as_deref(), manifest_path)?;

    let output_dir = args
        .output
        .clone()
        .unwrap_or_else(|| project_dir.join(".torvyn").join("artifacts"));

    // Every selected component must be built before anything is written, so a
    // pack that cannot finish leaves no half-populated output directory and no
    // empty one either. Reporting all of them at once also beats making the
    // user rebuild and re-run to discover the next missing component.
    let unbuilt: Vec<_> = selected
        .iter()
        .map(|decl| (&decl.name, artifact_path(&project_dir, &decl.name)))
        .filter(|(_, path)| !path.is_file())
        .collect();
    if !unbuilt.is_empty() {
        let list = unbuilt
            .iter()
            .map(|(name, path)| format!("'{}' (expected {})", name, path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        let detail = if unbuilt.len() == 1 {
            format!("Component {list} has not been built")
        } else {
            format!("{} components have not been built: {list}", unbuilt.len())
        };
        return Err(CliError::Packaging {
            detail,
            suggestion: "Run `torvyn build` to compile the project's components, then \
                         `torvyn pack` again."
                .into(),
        });
    }

    std::fs::create_dir_all(&output_dir).map_err(|e| CliError::Io {
        detail: format!("Cannot create output directory: {e}"),
        path: Some(output_dir.display().to_string()),
    })?;

    let version = args
        .tag
        .clone()
        .unwrap_or_else(|| manifest.torvyn.version.clone());

    let mut warnings = Vec::new();
    if args.include_source {
        warnings.push(
            "--include-source has no effect: an artifact always carries the component's WIT \
             contracts, because a contract-first runtime cannot verify a component without them."
                .to_owned(),
        );
    }
    if args.sign {
        warnings.push(
            "--sign was requested but signing is not implemented: the only signing provider \
             available marks artifacts as unsigned. Sigstore integration is a Phase 2 item. The \
             artifact was packed without a signature."
                .to_owned(),
        );
    }

    let mut artifacts = Vec::with_capacity(selected.len());
    for decl in selected {
        if !ctx.quiet && ctx.format == crate::cli::OutputFormat::Human {
            eprintln!("▶ Packing {}", decl.name);
        }
        artifacts.push(pack_one(
            decl,
            &manifest,
            &project_dir,
            &output_dir,
            &version,
        )?);
    }

    Ok(CommandResult {
        success: true,
        command: "pack".into(),
        data: PackResult {
            artifacts,
            output_dir,
        },
        warnings,
    })
}

/// Choose which components to pack.
fn select_components<'m>(
    manifest: &'m torvyn_config::ComponentManifest,
    requested: Option<&str>,
    manifest_path: &Path,
) -> Result<Vec<&'m ComponentDecl>, CliError> {
    let declared = || {
        manifest
            .components
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    match requested {
        Some(name) => manifest
            .components
            .iter()
            .filter(|decl| decl.name == name)
            .map(Ok)
            .collect::<Result<Vec<_>, CliError>>()
            .and_then(|found| {
                if found.is_empty() {
                    Err(CliError::Config {
                        detail: format!("No component named '{name}' is declared in the manifest"),
                        file: Some(manifest_path.display().to_string()),
                        suggestion: if manifest.components.is_empty() {
                            "The manifest declares no components.".into()
                        } else {
                            format!("Declared components: {}", declared())
                        },
                    })
                } else {
                    Ok(found)
                }
            }),
        None if manifest.components.is_empty() => Err(CliError::Config {
            detail: "The manifest declares no components to pack".into(),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Add a [[component]] entry with a name and the path to its source.".into(),
        }),
        None => Ok(manifest.components.iter().collect()),
    }
}

/// Assemble one component into an artifact.
fn pack_one(
    decl: &ComponentDecl,
    manifest: &torvyn_config::ComponentManifest,
    project_dir: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackedArtifact, CliError> {
    // `execute` checks this for every selected component up front; this guard
    // keeps `pack_one` correct on its own, since it is what the unit tests call.
    let wasm_path = artifact_path(project_dir, &decl.name);
    if !wasm_path.is_file() {
        return Err(CliError::Packaging {
            detail: format!(
                "Component '{}' has not been built (expected {})",
                decl.name,
                wasm_path.display()
            ),
            suggestion: "Run `torvyn build` to compile the project's components, then \
                         `torvyn pack` again."
                .into(),
        });
    }

    let wit_dir = locate_wit_dir(project_dir, decl).ok_or_else(|| CliError::Config {
        detail: format!("Component '{}' has no WIT contracts to package", decl.name),
        file: None,
        suggestion: format!(
            "Expected a wit/ directory under {} or at the project root. An artifact carries the \
             contracts its component implements, so a consumer can check compatibility without \
             running it.",
            project_dir.join(&decl.path).display()
        ),
    })?;

    let artifact_manifest = build_artifact_manifest(decl, manifest, version);
    let manifest_toml = artifact_manifest
        .to_toml_string()
        .map_err(|e| CliError::Runtime {
            detail: format!(
                "Cannot serialize the artifact manifest for '{}': {e}",
                decl.name
            ),
            context: None,
        })?;

    // The provenance subject is the component binary, identified by its own
    // digest — so the record attests to the exact bytes that were packed.
    let wasm_digest =
        torvyn_packaging::ContentDigest::of_file(&wasm_path).map_err(|e| CliError::Io {
            detail: format!("Cannot digest {}: {e}", wasm_path.display()),
            path: Some(wasm_path.display().to_string()),
        })?;
    let provenance = ProvenanceRecord::builder(&decl.name, &wasm_digest.to_string())
        .builder_id("https://torvyn.dev/torvyn-cli")
        .build();

    let input = PackInput {
        wasm_path: wasm_path.clone(),
        // Recorded for diagnostics only: the manifest text is supplied
        // directly, since an artifact manifest is derived rather than read.
        manifest_path: project_dir.join("Torvyn.toml"),
        wit_dir,
        provenance,
    };

    let output =
        pack_with_manifest(&input, &manifest_toml, output_dir).map_err(|e| CliError::Io {
            detail: format!("Cannot assemble the artifact for '{}': {e}", decl.name),
            path: Some(output_dir.display().to_string()),
        })?;

    Ok(PackedArtifact {
        name: decl.name.clone(),
        version: version.to_owned(),
        artifact_path: output.artifact_path,
        artifact_size_bytes: output.size_bytes,
        digest: output.digest.to_string(),
        layers: output
            .layer_digests
            .into_iter()
            .map(|(name, digest)| PackLayer {
                name,
                digest: digest.to_string(),
            })
            .collect(),
    })
}

/// Derive an artifact manifest for one component from the project manifest.
///
/// Capabilities come from the flow-node grants that name this component: an
/// artifact should declare what its component needs so a consumer can decide
/// whether to grant it, and the project's own grants are the only statement of
/// that intent the project makes.
fn build_artifact_manifest(
    decl: &ComponentDecl,
    manifest: &torvyn_config::ComponentManifest,
    version: &str,
) -> ArtifactManifest {
    let mut capabilities = CapabilitiesSpec::default();
    for capability in required_capabilities(decl, manifest) {
        capabilities.required.insert(capability, true);
    }

    let build_info = BuildInfoSpec {
        tool: build_tool_for(&decl.language).to_owned(),
        tool_version: String::new(),
        ..BuildInfoSpec::default()
    };

    ArtifactManifest::new(decl.name.clone(), version.to_owned())
        .with_description(manifest.torvyn.description.clone())
        .with_contracts(vec![format!(
            "torvyn:streaming@{}",
            manifest.torvyn.contract_version
        )])
        .with_build_info(build_info)
        .with_capabilities(capabilities)
}

/// Capability strings granted to any flow node that runs this component.
fn required_capabilities(
    decl: &ComponentDecl,
    manifest: &torvyn_config::ComponentManifest,
) -> Vec<String> {
    let mut capabilities: Vec<String> = manifest
        .security
        .grants
        .iter()
        .filter(|(node, _)| node_runs_component(manifest, node, &decl.name))
        .flat_map(|(_, grant)| grant.capabilities.iter().cloned())
        .collect();
    capabilities.sort_unstable();
    capabilities.dedup();
    capabilities
}

/// Whether a flow node named `node` runs the component named `component`.
///
/// Flow tables are stored as raw TOML in the project manifest, so this reads
/// the node's `component` value directly. A node that cannot be resolved is
/// treated as unrelated: over-declaring a capability on an artifact would ask
/// a consumer to grant more than the component needs.
fn node_runs_component(
    manifest: &torvyn_config::ComponentManifest,
    node: &str,
    component: &str,
) -> bool {
    manifest.flow.values().any(|flow| {
        flow.get("nodes")
            .and_then(|nodes| nodes.get(node))
            .and_then(|n| n.get("component"))
            .and_then(|c| c.as_str())
            == Some(component)
    })
}

/// The tool that builds a component of the given language, for provenance.
fn build_tool_for(language: &str) -> &'static str {
    match language {
        "rust" => "cargo-component",
        "go" => "tinygo",
        "python" => "componentize-py",
        _ => "custom",
    }
}

/// Find the WIT directory whose contracts belong to a component.
///
/// A multi-component project keeps them beside each component; a
/// single-component project keeps one at the root.
fn locate_wit_dir(project_dir: &Path, decl: &ComponentDecl) -> Option<PathBuf> {
    let beside_component = project_dir.join(&decl.path).join("wit");
    if beside_component.is_dir() {
        return Some(beside_component);
    }
    let at_root = project_dir.join("wit");
    at_root.is_dir().then_some(at_root)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn project_manifest(toml: &str) -> torvyn_config::ComponentManifest {
        torvyn_config::ComponentManifest::from_toml_str(toml, "Torvyn.toml")
            .expect("test manifest must parse")
    }

    const TWO_COMPONENTS: &str = r#"
[torvyn]
name = "demo"
version = "1.2.3"
contract_version = "0.1.0"
description = "a demo project"

[[component]]
name = "greeter"
path = "components/greeter"

[[component]]
name = "printer"
path = "components/printer"
language = "go"

[flow.main.nodes.source]
component = "greeter"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "printer"
interface = "torvyn:streaming/sink"

[security.grants.sink]
capabilities = ["stdio:stdout"]
"#;

    #[test]
    fn packs_every_component_by_default() {
        let manifest = project_manifest(TWO_COMPONENTS);
        let selected = select_components(&manifest, None, Path::new("Torvyn.toml")).unwrap();
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn a_named_component_packs_alone() {
        let manifest = project_manifest(TWO_COMPONENTS);
        let selected =
            select_components(&manifest, Some("printer"), Path::new("Torvyn.toml")).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "printer");
    }

    #[test]
    fn an_unknown_component_lists_what_is_declared() {
        let manifest = project_manifest(TWO_COMPONENTS);
        let err = select_components(&manifest, Some("absent"), Path::new("Torvyn.toml"))
            .expect_err("not declared");
        let message = format!("{err:?}");
        assert!(message.contains("greeter"), "{message}");
        assert!(message.contains("printer"), "{message}");
    }

    #[test]
    fn a_project_with_no_components_says_so() {
        let manifest = project_manifest(
            "[torvyn]\nname = \"x\"\nversion = \"0.1.0\"\ncontract_version = \"0.1.0\"\n",
        );
        assert!(select_components(&manifest, None, Path::new("Torvyn.toml")).is_err());
    }

    #[test]
    fn capabilities_follow_the_node_that_runs_the_component() {
        let manifest = project_manifest(TWO_COMPONENTS);
        let printer = manifest
            .components
            .iter()
            .find(|d| d.name == "printer")
            .unwrap();
        let greeter = manifest
            .components
            .iter()
            .find(|d| d.name == "greeter")
            .unwrap();

        // The `sink` node runs `printer` and is granted stdout.
        assert_eq!(
            required_capabilities(printer, &manifest),
            vec!["stdio:stdout".to_owned()]
        );
        // The `source` node runs `greeter` and is granted nothing. An artifact
        // must not ask a consumer for a capability its component never uses.
        assert!(required_capabilities(greeter, &manifest).is_empty());
    }

    #[test]
    fn the_artifact_manifest_carries_identity_contracts_and_capabilities() {
        let manifest = project_manifest(TWO_COMPONENTS);
        let printer = manifest
            .components
            .iter()
            .find(|d| d.name == "printer")
            .unwrap();

        let artifact = build_artifact_manifest(printer, &manifest, "1.2.3");
        assert_eq!(artifact.name(), "printer");
        assert_eq!(artifact.version(), "1.2.3");
        assert_eq!(artifact.description(), "a demo project");
        assert_eq!(
            artifact.contract_package_strings(),
            ["torvyn:streaming@0.1.0"]
        );
        assert!(artifact.capabilities.required.contains_key("stdio:stdout"));
        assert_eq!(artifact.build_info.tool, "tinygo");

        // It must round-trip: `pack` parses this text back before embedding it,
        // and `inspect` parses it out of the archive.
        let toml = artifact.to_toml_string().expect("serializes");
        let reparsed = ArtifactManifest::from_toml_str(&toml).expect("round-trips");
        assert_eq!(reparsed.name(), "printer");
        assert_eq!(reparsed.version(), "1.2.3");
    }

    #[test]
    fn a_tag_overrides_the_project_version() {
        let manifest = project_manifest(TWO_COMPONENTS);
        let greeter = &manifest.components[0];
        let artifact = build_artifact_manifest(greeter, &manifest, "nightly");
        assert_eq!(artifact.version(), "nightly");
    }

    #[test]
    fn build_tool_is_named_per_language() {
        assert_eq!(build_tool_for("rust"), "cargo-component");
        assert_eq!(build_tool_for("go"), "tinygo");
        assert_eq!(build_tool_for("python"), "componentize-py");
        assert_eq!(build_tool_for("zig"), "custom");
    }

    #[test]
    fn wit_is_found_beside_the_component_then_at_the_root() {
        let dir = std::env::temp_dir().join(format!("torvyn-pack-wit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let decl = ComponentDecl {
            name: "c".into(),
            path: "components/c".into(),
            ..ComponentDecl::default()
        };

        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(locate_wit_dir(&dir, &decl), None, "neither location exists");

        std::fs::create_dir_all(dir.join("wit")).unwrap();
        assert_eq!(locate_wit_dir(&dir, &decl), Some(dir.join("wit")));

        let beside = dir.join("components/c/wit");
        std::fs::create_dir_all(&beside).unwrap();
        assert_eq!(
            locate_wit_dir(&dir, &decl),
            Some(beside),
            "a component's own contracts win over the project's"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
