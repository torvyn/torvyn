//! `torvyn link` — verify component composition compatibility.
//!
//! Delegates to `torvyn-linker` for topology validation and interface
//! compatibility checking.
//!
//! # One schema, one definition
//!
//! Flow definitions are read through `torvyn_config`, the same types the host
//! uses when it starts a flow. This command previously declared its own
//! `FlowDef`, `NodeDef`, and `EdgeDef` privately, with an edge written as the
//! string `"node:port"`. The canonical schema writes an edge as a table —
//! `from = { node = "source", port = "output" }` — which is what every
//! manifest in this repository uses, so `link` failed on all of them with
//! "invalid type: map, expected a string". Two definitions of one schema will
//! drift; this file no longer keeps a second.

use crate::cli::LinkArgs;
use crate::errors::CliError;
use crate::output::terminal;
use crate::output::{CommandResult, HumanRenderable, OutputContext};
use serde::Serialize;
use std::path::{Path, PathBuf};
use torvyn_config::{FlowDef, NodeDef};
use torvyn_pipeline::ComponentIndex;

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
        return Err(crate::commands::no_flow_defined(manifest_path, &manifest));
    }

    let project_dir = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    // Resolve node component names the same way the host does, so `link`
    // checks the artifacts a run would actually load.
    let components = ComponentIndex::new(project_dir, &manifest.components);

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

        // Deserialize into the canonical flow definition.
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
            topo.add_node(torvyn_linker::TopologyNode {
                name: node_name.clone(),
                role: role_of(node_def),
                // A node names its component; the index turns that into the
                // artifact a run would load. A component that is declared but
                // not yet built has no artifact, and linking is a static check
                // that should still run, so the declared name is kept as the
                // path in that case rather than failing.
                artifact_path: components.resolve(&node_def.component).map_or_else(
                    |_| PathBuf::from(&node_def.component),
                    |uri| PathBuf::from(uri.strip_prefix("file://").unwrap_or(&uri)),
                ),
                config: node_def.config.clone(),
                capability_grants: grants_for(&manifest, node_name),
            });
        }

        // Add edges from the flow definition
        for edge_def in &flow_def.edges {
            topo.add_edge(torvyn_linker::TopologyEdge {
                from_node: edge_def.from.node.clone(),
                from_port: edge_def.from.port.clone(),
                to_node: edge_def.to.node.clone(),
                to_port: edge_def.to.port.clone(),
                queue_depth: edge_def
                    .queue_depth
                    .and_then(|d| u32::try_from(d).ok())
                    .unwrap_or_else(|| {
                        u32::try_from(torvyn_types::DEFAULT_QUEUE_DEPTH).unwrap_or(64)
                    }),
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
        failure: None,
        command: "link".into(),
        data: result,
        warnings: vec![],
    })
}

/// The role a node plays, inferred from the interface it declares.
///
/// Matching on the interface *path* rather than a bare substring keeps
/// `torvyn:streaming/processor` from being read as a source just because the
/// package name contains "s". Nodes that declare no interface are processors,
/// which is the only role that both consumes and produces.
fn role_of(node: &NodeDef) -> torvyn_types::ComponentRole {
    Some(node.interface.as_str())
        .filter(|iface| !iface.is_empty())
        .and_then(|iface| match iface.rsplit('/').next() {
            Some("source") => Some(torvyn_types::ComponentRole::Source),
            Some("sink") => Some(torvyn_types::ComponentRole::Sink),
            Some("filter") => Some(torvyn_types::ComponentRole::Filter),
            Some("router") => Some(torvyn_types::ComponentRole::Router),
            Some("processor") => Some(torvyn_types::ComponentRole::Processor),
            _ => None,
        })
        .unwrap_or(torvyn_types::ComponentRole::Processor)
}

/// Capability grants the manifest gives a flow node.
///
/// The linker checks a node's declared grants against what its component
/// requires, so dropping them — as this command used to, passing an empty
/// vector — meant the capability half of "verify interface compatibility and
/// capability grants" never ran.
fn grants_for(
    manifest: &torvyn_config::ComponentManifest,
    node_name: &str,
) -> Vec<torvyn_linker::CapabilityGrant> {
    manifest
        .security
        .grants
        .get(node_name)
        .map(|grant| {
            grant
                .capabilities
                .iter()
                .map(|capability| {
                    // Canonical form is `<domain>:<action>[:<scope>]`; the
                    // linker models a grant as a name and a detail, so the
                    // scope becomes the detail where one is present.
                    let (name, detail) = match capability.split_once(':') {
                        Some((domain, rest)) => match rest.split_once(':') {
                            Some((action, scope)) => {
                                (format!("{domain}:{action}"), scope.to_owned())
                            }
                            None => (capability.clone(), String::new()),
                        },
                        None => (capability.clone(), String::new()),
                    };
                    torvyn_linker::CapabilityGrant { name, detail }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::ComponentRole;

    fn node(interface: &str) -> NodeDef {
        NodeDef {
            interface: interface.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn infers_a_role_from_the_interface_path() {
        assert_eq!(
            role_of(&node("torvyn:streaming/source")),
            ComponentRole::Source
        );
        assert_eq!(role_of(&node("torvyn:streaming/sink")), ComponentRole::Sink);
        assert_eq!(
            role_of(&node("torvyn:streaming/processor")),
            ComponentRole::Processor
        );
        assert_eq!(
            role_of(&node("torvyn:streaming/filter")),
            ComponentRole::Filter
        );
        assert_eq!(
            role_of(&node("torvyn:streaming/router")),
            ComponentRole::Router
        );
    }

    /// A versioned interface is what a real manifest carries once contracts are
    /// pinned, and the last path segment is what identifies the role.
    #[test]
    fn matches_on_the_path_segment_not_a_substring() {
        // `torvyn:streaming` contains "s"; naive substring matching read every
        // node as a source.
        assert_eq!(
            role_of(&node("torvyn:streaming/processor")),
            ComponentRole::Processor
        );
        // An unrecognised interface, and a node that declares none, both fall
        // back to the role that consumes and produces.
        assert_eq!(
            role_of(&node("acme:custom/widget")),
            ComponentRole::Processor
        );
        assert_eq!(role_of(&node("")), ComponentRole::Processor);
    }

    fn manifest_granting(
        node_name: &str,
        capabilities: &[&str],
    ) -> torvyn_config::ComponentManifest {
        let mut manifest = torvyn_config::ComponentManifest::default();
        manifest.security.grants.insert(
            node_name.to_owned(),
            torvyn_config::CapabilityGrant {
                capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
            },
        );
        manifest
    }

    #[test]
    fn reads_a_two_part_grant_as_a_name_with_no_detail() {
        let manifest = manifest_granting("sink", &["stdio:stdout"]);
        let grants = grants_for(&manifest, "sink");
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].name, "stdio:stdout");
        assert_eq!(grants[0].detail, "");
    }

    #[test]
    fn splits_a_scoped_grant_into_name_and_detail() {
        let manifest = manifest_granting("reader", &["fs:read:/var/data"]);
        let grants = grants_for(&manifest, "reader");
        assert_eq!(grants[0].name, "fs:read");
        assert_eq!(grants[0].detail, "/var/data");
    }

    /// The linker checks a node's grants against what its component requires.
    /// This command used to pass an empty vector for every node, so the
    /// capability half of the check never ran; a node with no grant entry must
    /// still yield an empty list rather than panicking.
    #[test]
    fn a_node_with_no_grants_gets_an_empty_list() {
        let manifest = manifest_granting("sink", &["stdio:stdout"]);
        assert!(grants_for(&manifest, "source").is_empty());
        assert!(grants_for(&torvyn_config::ComponentManifest::default(), "sink").is_empty());
    }

    #[test]
    fn keeps_a_grant_with_no_separator_intact() {
        let manifest = manifest_granting("odd", &["clock"]);
        let grants = grants_for(&manifest, "odd");
        assert_eq!(grants[0].name, "clock");
        assert_eq!(grants[0].detail, "");
    }
}
