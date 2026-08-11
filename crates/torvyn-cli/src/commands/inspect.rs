//! `torvyn inspect` — inspect a component or artifact.
//!
//! Reports what a component binary declares, read from its Component Model
//! type section, alongside the metadata a packaged artifact carries.
//!
//! The interface lists used to be empty for everything — a contract-first
//! runtime reporting that a component has no interfaces. They are now read
//! from the binary itself, so they describe what the component genuinely
//! imports and exports rather than what a manifest says about it.

use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::path::Path;

/// Result of `torvyn inspect`.
#[derive(Debug, Serialize)]
pub struct InspectResult {
    /// Component name.
    pub name: String,
    /// Component version.
    pub version: String,
    /// Wasm binary size.
    pub size_wasm_bytes: u64,
    /// Packaged size (if inspecting an artifact).
    pub size_packaged_bytes: Option<u64>,
    /// Exported interfaces.
    pub exports: Vec<String>,
    /// Imported interfaces.
    pub imports: Vec<String>,
    /// Required capabilities.
    pub capabilities_required: Vec<String>,
    /// Contract version.
    pub contract_version: Option<String>,
    /// Build tool info.
    pub build_info: Option<String>,
}

impl HumanRenderable for InspectResult {
    fn render_human(&self, ctx: &OutputContext) {
        terminal::print_kv(ctx, "Component", &self.name);
        terminal::print_kv(ctx, "Version", &self.version);
        terminal::print_kv(
            ctx,
            "Size",
            &format!(
                "{} (Wasm){}",
                terminal::format_bytes(self.size_wasm_bytes),
                self.size_packaged_bytes
                    .map(|s| format!(", {} (packaged)", terminal::format_bytes(s)))
                    .unwrap_or_default()
            ),
        );

        if !self.exports.is_empty() {
            eprintln!();
            eprintln!("  Exports:");
            for exp in &self.exports {
                eprintln!("    {exp}");
            }
        }

        if !self.imports.is_empty() {
            eprintln!();
            eprintln!("  Imports:");
            for imp in &self.imports {
                eprintln!("    {imp}");
            }
        }

        eprintln!();
        if self.capabilities_required.is_empty() {
            eprintln!("  Capabilities required: (none)");
        } else {
            eprintln!("  Capabilities required:");
            for cap in &self.capabilities_required {
                eprintln!("    {cap}");
            }
        }

        if let Some(cv) = &self.contract_version {
            terminal::print_kv(ctx, "Contract version", cv);
        }
        if let Some(bi) = &self.build_info {
            terminal::print_kv(ctx, "Built with", bi);
        }
    }
}

/// Execute the `torvyn inspect` command.
///
/// COLD PATH.
pub async fn execute(
    args: &crate::cli::InspectArgs,
    _ctx: &OutputContext,
) -> Result<CommandResult<InspectResult>, CliError> {
    let target = &args.target;
    let target_path = Path::new(target);

    if !target_path.exists() {
        return Err(CliError::Config {
            detail: format!("Target not found: {target}"),
            file: Some(target.clone()),
            suggestion: "Provide a path to a .wasm file or packaged artifact.".into(),
        });
    }

    let file_size = std::fs::metadata(target_path).map(|m| m.len()).unwrap_or(0);

    let name = target_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Check if this is a packaged artifact (.torvyn extension)
    let is_artifact = target_path
        .extension()
        .map(|ext| ext == "torvyn" || ext == "tar")
        .unwrap_or(false);

    if is_artifact {
        // Delegate to torvyn-packaging for artifact inspection
        match torvyn_packaging::inspect(target_path) {
            Ok(inspection) => {
                // Read the packaged component's own interface surface.
                let interfaces = torvyn_packaging::unpack(target_path)
                    .ok()
                    .map(|contents| describe_wasm(&contents.wasm_bytes))
                    .unwrap_or_default();

                let result = InspectResult {
                    name: inspection.name,
                    version: inspection.version,
                    size_wasm_bytes: inspection.wasm_size_bytes as u64,
                    size_packaged_bytes: Some(file_size),
                    exports: interfaces.exports,
                    imports: interfaces.imports,
                    capabilities_required: inspection.capabilities_required,
                    contract_version: if inspection.min_torvyn_version.is_empty() {
                        None
                    } else {
                        Some(inspection.min_torvyn_version)
                    },
                    build_info: if inspection.build_tool.is_empty() {
                        None
                    } else {
                        Some(inspection.build_tool)
                    },
                };

                return Ok(CommandResult {
                    success: true,
                    failure: None,
                    command: "inspect".into(),
                    data: result,
                    warnings: vec![],
                });
            }
            Err(_e) => {
                // Fall through to basic file inspection
            }
        }
    }

    // A bare `.wasm` carries no manifest, so identity comes from the file name
    // and everything else that can be known comes from the binary itself.
    let interfaces = std::fs::read(target_path)
        .map(|bytes| describe_wasm(&bytes))
        .unwrap_or_default();

    let result = InspectResult {
        name,
        version: "unknown".into(),
        size_wasm_bytes: file_size,
        size_packaged_bytes: None,
        exports: interfaces.exports,
        imports: interfaces.imports,
        capabilities_required: vec![],
        contract_version: None,
        build_info: None,
    };

    Ok(CommandResult {
        success: true,
        failure: None,
        command: "inspect".into(),
        data: result,
        warnings: vec![],
    })
}

/// Read a component binary's declared imports and exports.
///
/// Compiling to inspect is heavier than parsing the type section alone, but it
/// reuses the very engine that will run the component — so what is reported is
/// exactly what the runtime sees, and a binary the runtime would reject cannot
/// be reported as valid. Returns empty lists for anything that is not a
/// Component Model binary.
fn describe_wasm(bytes: &[u8]) -> torvyn_engine::ComponentInterfaces {
    torvyn_engine::WasmtimeEngine::new(torvyn_engine::WasmtimeEngineConfig::default())
        .ok()
        .and_then(|engine| engine.describe_bytes(bytes).ok())
        .unwrap_or_default()
}
