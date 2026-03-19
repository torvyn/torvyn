//! `torvyn link` — verify component composition compatibility.
//!
//! Delegates to `torvyn-linker` for topology validation and
//! interface compatibility checking.

use crate::cli::LinkArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Result of `torvyn link`.
#[derive(Debug, Serialize)]
pub struct LinkResult {
    /// Whether all flows link successfully.
    pub all_linked: bool,
    /// Per-flow results.
    pub flows: Vec<FlowLinkResult>,
}

/// Link result for a single flow.
#[derive(Debug, Serialize)]
pub struct FlowLinkResult {
    /// Flow name.
    pub name: String,
    /// Whether this flow links.
    pub linked: bool,
    /// Number of components in the flow.
    pub component_count: usize,
    /// Number of edges in the flow.
    pub edge_count: usize,
    /// Diagnostics for this flow.
    pub diagnostics: Vec<String>,
}

impl HumanRenderable for LinkResult {
    fn render_human(&self, ctx: &OutputContext) {
        for flow in &self.flows {
            if flow.linked {
                terminal::print_success(
                    ctx,
                    &format!(
                        "Flow \"{}\" links successfully ({} components, {} edges, 0 errors)",
                        flow.name, flow.component_count, flow.edge_count
                    ),
                );
            } else {
                terminal::print_failure(ctx, &format!("Flow \"{}\" has linking errors", flow.name));
                for d in &flow.diagnostics {
                    eprintln!("  {d}");
                }
            }
        }
    }
}

/// Execute the `torvyn link` command.
///
/// COLD PATH.
pub async fn execute(
    args: &LinkArgs,
    ctx: &OutputContext,
) -> Result<CommandResult<LinkResult>, CliError> {
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

    if !manifest.has_flows() {
        return Err(CliError::Config {
            detail: "No flows defined in manifest".into(),
            file: Some(manifest_path.display().to_string()),
            suggestion: "Add a [flow.*] section to your Torvyn.toml.".into(),
        });
    }

    let _project_dir = manifest_path.parent().unwrap_or(Path::new("."));
    let mut flow_results = Vec::new();
    let mut all_linked = true;

    // Build topologies from the manifest's flow definitions.
    // `manifest.flow` is `HashMap<String, toml::Value>`, so we deserialize
    // each flow value into our local FlowDef struct.
    for (flow_name, flow_value) in &manifest.flow {
        // Skip flows not matching --flow filter
        if let Some(ref filter) = args.flow {
            if flow_name != filter {
                continue;
            }
        }

        ctx.print_debug(&format!("Linking flow: {flow_name}"));

        // Deserialize the toml::Value into our local FlowDef
        let flow_def: FlowDef = flow_value
            .clone()
            .try_into()
            .map_err(|e: toml::de::Error| CliError::Config {
                detail: format!("Invalid flow definition for '{flow_name}': {e}"),
                file: Some(manifest_path.display().to_string()),
                suggestion: "Check the [flow] section in your Torvyn.toml.".into(),
            })?;

        // Build the PipelineTopology from the config flow definition
        let mut topo = torvyn_linker::PipelineTopology::new(flow_name.clone());

        // Add nodes from the flow definition
        for (node_name, node_def) in &flow_def.nodes {
            let role = match node_def.interface.as_deref() {
                Some(iface) if iface.contains("source") => torvyn_types::ComponentRole::Source,
                Some(iface) if iface.contains("sink") => torvyn_types::ComponentRole::Sink,
                Some(iface) if iface.contains("filter") => torvyn_types::ComponentRole::Filter,
                Some(iface) if iface.contains("router") => torvyn_types::ComponentRole::Router,
                _ => torvyn_types::ComponentRole::Processor,
            };

            topo.add_node(torvyn_linker::TopologyNode {
                name: node_name.clone(),
                role,
                artifact_path: PathBuf::from(&node_def.component),
                config: node_def.config.clone(),
                capability_grants: vec![],
            });
        }

        // Add edges from the flow definition
        for edge_def in &flow_def.edges {
            let (from_node, from_port) = parse_port_ref(&edge_def.from);
            let (to_node, to_port) = parse_port_ref(&edge_def.to);

            topo.add_edge(torvyn_linker::TopologyEdge {
                from_node,
                from_port,
                to_node,
                to_port,
                queue_depth: 64,
                backpressure_policy: Default::default(),
            });
        }

        // Validate the topology
        let node_count = topo.nodes.len();
        let edge_count = topo.edges.len();

        let mut linker = torvyn_linker::PipelineLinker::new();
        let link_result = linker.link_topology_only(&topo);

        let (linked, diags) = match link_result {
            Ok(_) => (true, vec![]),
            Err(e) => {
                let diag_strs = match &e {
                    torvyn_linker::LinkerError::LinkFailed(report) => report
                        .errors
                        .iter()
                        .map(|d| d.message.clone())
                        .collect::<Vec<_>>(),
                    other => vec![other.to_string()],
                };
                (false, diag_strs)
            }
        };

        if !linked {
            all_linked = false;
        }

        flow_results.push(FlowLinkResult {
            name: flow_name.clone(),
            linked,
            component_count: node_count,
            edge_count,
            diagnostics: diags,
        });
    }

    let result = LinkResult {
        all_linked,
        flows: flow_results,
    };

    if !all_linked {
        let err_msgs: Vec<String> = result
            .flows
            .iter()
            .filter(|f| !f.linked)
            .flat_map(|f| f.diagnostics.clone())
            .collect();
        return Err(CliError::Link {
            detail: "One or more flows failed to link".into(),
            diagnostics: err_msgs,
        });
    }

    Ok(CommandResult {
        success: true,
        command: "link".into(),
        data: result,
        warnings: vec![],
    })
}

/// Local flow definition, deserialized from `toml::Value`.
#[derive(Debug, Deserialize)]
struct FlowDef {
    /// Nodes keyed by name.
    #[serde(default)]
    nodes: HashMap<String, NodeDef>,
    /// Edges connecting nodes.
    #[serde(default)]
    edges: Vec<EdgeDef>,
}

/// A single node in a flow definition.
#[derive(Debug, Deserialize)]
struct NodeDef {
    /// Path to the component artifact.
    component: String,
    /// Interface type hint (e.g. "torvyn:streaming/source").
    #[serde(default)]
    interface: Option<String>,
    /// TOML config string for the component.
    #[serde(default)]
    config: Option<String>,
}

/// A single edge in a flow definition.
#[derive(Debug, Deserialize)]
struct EdgeDef {
    /// Source in "node:port" format.
    from: String,
    /// Destination in "node:port" format.
    to: String,
}

/// Parse a "node:port" reference into (node, port) parts.
fn parse_port_ref(s: &str) -> (String, String) {
    match s.split_once(':') {
        Some((node, port)) => (node.to_string(), port.to_string()),
        None => (s.to_string(), "default".to_string()),
    }
}
