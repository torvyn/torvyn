//! Config-to-topology conversion.
//!
//! Converts a [`torvyn_config::FlowDef`] (parsed from TOML) into a
//! [`PipelineTopology`] (the validated in-memory topology model).

use torvyn_config::{EdgeDef, FlowDef, NodeDef};
use torvyn_types::ComponentRole;

use crate::builder::PipelineTopologyBuilder;
use crate::error::PipelineError;
use crate::topology::{EdgeConfig, NodeConfig, PipelineTopology};

/// Convert a `FlowDef` (from config) into a `PipelineTopology`.
///
/// # COLD PATH — called once per flow during pipeline loading.
///
/// # Errors
/// Returns `Err(Vec<PipelineError>)` if the flow definition is invalid.
///
/// # Preconditions
/// - `flow_def` has been parsed and syntactically validated by `torvyn-config`.
///
/// # Postconditions
/// - On success, the returned `PipelineTopology` satisfies all topology invariants.
pub fn flow_def_to_topology(
    flow_name: &str,
    flow_def: &FlowDef,
) -> Result<PipelineTopology, Vec<PipelineError>> {
    let mut builder = PipelineTopologyBuilder::new(flow_name).description(&flow_def.description);

    // Add nodes
    for (node_name, node_def) in &flow_def.nodes {
        let role =
            infer_role_from_interface(&node_def.interface).unwrap_or(ComponentRole::Processor);

        let config = node_def_to_config(node_def);

        builder = builder.add_node_with_interface(
            node_name,
            role,
            &node_def.component,
            &node_def.interface,
            config,
        );
    }

    // Add edges
    for edge_def in &flow_def.edges {
        let edge_config = edge_def_to_config(edge_def);
        builder = builder.add_edge_with_config(
            &edge_def.from.node,
            &edge_def.from.port,
            &edge_def.to.node,
            &edge_def.to.port,
            edge_config,
        );
    }

    builder.build()
}

/// Infer a `ComponentRole` from a WIT interface string.
///
/// # COLD PATH
fn infer_role_from_interface(interface: &str) -> Option<ComponentRole> {
    if interface.contains("/source") {
        Some(ComponentRole::Source)
    } else if interface.contains("/processor") {
        Some(ComponentRole::Processor)
    } else if interface.contains("/sink") {
        Some(ComponentRole::Sink)
    } else if interface.contains("/filter") {
        Some(ComponentRole::Filter)
    } else if interface.contains("/router") {
        Some(ComponentRole::Router)
    } else {
        None
    }
}

/// Convert a `NodeDef` to a `NodeConfig`.
///
/// # COLD PATH
fn node_def_to_config(node_def: &NodeDef) -> NodeConfig {
    NodeConfig {
        fuel_budget: node_def.fuel_budget,
        memory_limit: node_def
            .max_memory
            .as_ref()
            .and_then(|s| parse_memory_size(s)),
        timeout: None, // Not exposed in NodeDef at config level (flow-level default)
        priority: node_def.priority.map(|p| p.min(10) as u8),
        error_policy: None, // Not exposed in NodeDef at config level
        init_config: node_def.config.clone(),
    }
}

/// Convert an `EdgeDef` to an `EdgeConfig`.
///
/// # COLD PATH
fn edge_def_to_config(edge_def: &EdgeDef) -> EdgeConfig {
    EdgeConfig {
        queue_depth: edge_def.queue_depth,
        backpressure_policy: edge_def.backpressure.as_ref().map(|bp| {
            // LLI DEVIATION: BackpressureConfig.backpressure_policy is a String,
            // not Option<String>. Match against the string value directly.
            match bp.backpressure_policy.as_str() {
                "drop-oldest" => torvyn_types::BackpressurePolicy::DropOldest,
                "drop-newest" => torvyn_types::BackpressurePolicy::DropNewest,
                "error" => torvyn_types::BackpressurePolicy::Error,
                // "block-producer" and any unknown value default to BlockProducer
                _ => torvyn_types::BackpressurePolicy::BlockProducer,
            }
        }),
    }
}

/// Parse a memory size string like "16MiB" or "1GiB" into bytes.
///
/// # COLD PATH
fn parse_memory_size(s: &str) -> Option<usize> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix("GiB") {
        rest.trim()
            .parse::<usize>()
            .ok()
            .map(|v| v * 1024 * 1024 * 1024)
    } else if let Some(rest) = s.strip_suffix("MiB") {
        rest.trim().parse::<usize>().ok().map(|v| v * 1024 * 1024)
    } else if let Some(rest) = s.strip_suffix("KiB") {
        rest.trim().parse::<usize>().ok().map(|v| v * 1024)
    } else {
        s.parse::<usize>().ok()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use torvyn_config::EdgeEndpoint;

    fn make_flow_def() -> FlowDef {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "source".to_owned(),
            NodeDef {
                component: "file://source.wasm".into(),
                interface: "torvyn:streaming/source".into(),
                ..Default::default()
            },
        );
        nodes.insert(
            "sink".to_owned(),
            NodeDef {
                component: "file://sink.wasm".into(),
                interface: "torvyn:streaming/sink".into(),
                ..Default::default()
            },
        );

        FlowDef {
            description: "test flow".into(),
            nodes,
            edges: vec![EdgeDef {
                from: EdgeEndpoint {
                    node: "source".into(),
                    port: "output".into(),
                },
                to: EdgeEndpoint {
                    node: "sink".into(),
                    port: "input".into(),
                },
                queue_depth: None,
                backpressure: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_flow_def_to_topology_basic() {
        let flow = make_flow_def();
        let topo = flow_def_to_topology("test", &flow).unwrap();

        assert_eq!(topo.node_count(), 2);
        assert_eq!(topo.edge_count(), 1);
    }

    #[test]
    fn test_infer_role_from_interface() {
        assert_eq!(
            infer_role_from_interface("torvyn:streaming/source"),
            Some(ComponentRole::Source)
        );
        assert_eq!(
            infer_role_from_interface("torvyn:streaming/processor"),
            Some(ComponentRole::Processor)
        );
        assert_eq!(
            infer_role_from_interface("torvyn:streaming/sink"),
            Some(ComponentRole::Sink)
        );
        assert_eq!(
            infer_role_from_interface("torvyn:streaming/filter"),
            Some(ComponentRole::Filter)
        );
        assert_eq!(
            infer_role_from_interface("torvyn:streaming/router"),
            Some(ComponentRole::Router)
        );
        assert_eq!(infer_role_from_interface("unknown:thing/whatever"), None);
    }

    #[test]
    fn test_parse_memory_size() {
        assert_eq!(parse_memory_size("16MiB"), Some(16 * 1024 * 1024));
        assert_eq!(parse_memory_size("1GiB"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_size("512KiB"), Some(512 * 1024));
        assert_eq!(parse_memory_size("65536"), Some(65536));
        assert_eq!(parse_memory_size("invalid"), None);
    }
}
