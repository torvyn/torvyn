//! Config-to-topology conversion.
//!
//! Converts a [`torvyn_config::FlowDef`] (parsed from TOML) into a
//! [`PipelineTopology`] (the validated in-memory topology model).

use torvyn_config::{parse_memory_size, EdgeDef, FlowDef, NodeDef, SecurityConfig};
use torvyn_security::WasiConfiguration;
use torvyn_types::ComponentRole;

use crate::builder::PipelineTopologyBuilder;
use crate::error::PipelineError;
use crate::resolve::ComponentIndex;
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
    security: &SecurityConfig,
    components: &ComponentIndex,
) -> Result<PipelineTopology, Vec<PipelineError>> {
    let mut builder = PipelineTopologyBuilder::new(flow_name).description(&flow_def.description);

    // Add nodes
    for (node_name, node_def) in &flow_def.nodes {
        let role =
            infer_role_from_interface(&node_def.interface).unwrap_or(ComponentRole::Processor);

        let mut config = node_def_to_config(node_def).map_err(|reason| {
            vec![PipelineError::Subsystem {
                subsystem: "config",
                reason: format!("flow '{flow_name}', node '{node_name}': {reason}"),
            }]
        })?;

        // Resolve this component's capability grants into its WASI sandbox.
        // A component with no grants stays deny-all (fully sandboxed).
        if let Some(grant) = security.grants.get(node_name) {
            config.wasi =
                WasiConfiguration::from_grant_strings(&grant.capabilities).map_err(|e| {
                    vec![PipelineError::SandboxConfigFailed {
                        flow_name: flow_name.to_owned(),
                        node_name: node_name.clone(),
                        reason: e.to_string(),
                    }]
                })?;
        }

        // Join the node's `component` value to the manifest's `[[component]]`
        // declarations. A bare name becomes the artifact `torvyn build`
        // produced; a `file://` or `mock://` reference passes through.
        let component_ref = components.resolve(&node_def.component).map_err(|e| {
            vec![PipelineError::Subsystem {
                subsystem: "config",
                reason: format!("flow '{flow_name}', node '{node_name}': {e}"),
            }]
        })?;

        builder = builder.add_node_with_interface(
            node_name,
            role,
            &component_ref,
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
fn node_def_to_config(node_def: &NodeDef) -> Result<NodeConfig, String> {
    // A memory cap the runtime cannot read is a limit the operator believes is
    // in force. Report it rather than dropping it — `torvyn check` validates
    // the same field with the same parser, so this is the second line of
    // defence, not the first.
    let memory_limit = match &node_def.max_memory {
        Some(raw) => {
            let bytes = parse_memory_size(raw).map_err(|reason| format!("max_memory: {reason}"))?;
            // A cap wider than the address space cannot be applied, and
            // truncating it would silently install a *smaller* one.
            Some(usize::try_from(bytes).map_err(|_| {
                format!("max_memory: {bytes} bytes does not fit in this platform's address space")
            })?)
        }
        None => None,
    };

    Ok(NodeConfig {
        fuel_budget: node_def.fuel_budget,
        memory_limit,
        timeout: None, // Not exposed in NodeDef at config level (flow-level default)
        priority: node_def.priority.map(|p| p.min(10) as u8),
        error_policy: None, // Not exposed in NodeDef at config level
        init_config: node_def.config.clone(),
        // Deny-all by default; `flow_def_to_topology` overrides this from the
        // security configuration's per-component grants.
        wasi: WasiConfiguration::deny_all(),
    })
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
        let topo = flow_def_to_topology(
            "test",
            &flow,
            &SecurityConfig::default(),
            &ComponentIndex::empty(),
        )
        .unwrap();

        assert_eq!(topo.node_count(), 2);
        assert_eq!(topo.edge_count(), 1);
        // No grants declared -> every node is deny-all.
        for node in topo.nodes() {
            assert_eq!(node.config().wasi, WasiConfiguration::deny_all());
        }
    }

    #[test]
    fn test_flow_def_to_topology_resolves_capability_grants() {
        use std::collections::BTreeMap;
        use torvyn_config::CapabilityGrant;

        let flow = make_flow_def();
        // `make_flow_def` defines nodes "source" and "sink"; grant the source
        // filesystem read and environment access.
        let mut grants = BTreeMap::new();
        grants.insert(
            "source".to_owned(),
            CapabilityGrant {
                capabilities: vec![
                    "filesystem:read:/data".to_owned(),
                    "environment:read".to_owned(),
                ],
            },
        );
        let security = SecurityConfig {
            grants,
            ..SecurityConfig::default()
        };

        let topo =
            flow_def_to_topology("test", &flow, &security, &ComponentIndex::empty()).unwrap();
        let source = topo
            .nodes()
            .iter()
            .find(|n| n.name() == "source")
            .expect("source node exists");
        assert!(source.config().wasi.allow_environment);
        assert!(source
            .config()
            .wasi
            .preopened_dirs
            .iter()
            .any(|d| d.host_path == "/data" && d.read));

        // The sink declared no grants -> deny-all.
        let sink = topo
            .nodes()
            .iter()
            .find(|n| n.name() == "sink")
            .expect("sink node exists");
        assert_eq!(sink.config().wasi, WasiConfiguration::deny_all());
    }

    #[test]
    fn test_flow_def_to_topology_rejects_invalid_grant() {
        use std::collections::BTreeMap;
        use torvyn_config::CapabilityGrant;

        let flow = make_flow_def();
        let mut grants = BTreeMap::new();
        grants.insert(
            "source".to_owned(),
            CapabilityGrant {
                capabilities: vec!["network:egress:*".to_owned()],
            },
        );
        let security = SecurityConfig {
            grants,
            ..SecurityConfig::default()
        };

        let errors =
            flow_def_to_topology("test", &flow, &security, &ComponentIndex::empty()).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, PipelineError::SandboxConfigFailed { .. })));
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
    fn a_nodes_memory_cap_is_read_in_every_documented_unit() {
        for (written, expected) in [
            ("16MiB", 16 * 1024 * 1024),
            ("1GiB", 1024 * 1024 * 1024),
            ("512KiB", 512 * 1024),
            // The `B` suffix is documented and the pipeline's own parser, now
            // removed, could not read it — so `max_memory = "1024B"` was
            // silently dropped.
            ("1024B", 1024),
            ("65536", 65536),
        ] {
            let node = NodeDef {
                max_memory: Some(written.to_owned()),
                ..Default::default()
            };
            let config = node_def_to_config(&node).expect("a documented size must parse");
            assert_eq!(
                config.memory_limit,
                Some(expected),
                "max_memory = {written:?}"
            );
        }
    }

    /// A cap the runtime cannot read is a limit the operator believes is in
    /// force. It must be reported, not dropped.
    #[test]
    fn an_unreadable_memory_cap_is_reported() {
        let node = NodeDef {
            max_memory: Some("banana".to_owned()),
            ..Default::default()
        };
        let err = node_def_to_config(&node).expect_err("an unreadable size must be rejected");
        assert!(err.contains("max_memory"), "{err}");
    }

    /// A node that sets no limits inherits the engine's, which is what `None`
    /// means downstream.
    #[test]
    fn a_node_without_limits_inherits_them() {
        let config = node_def_to_config(&NodeDef::default()).expect("defaults are valid");
        assert!(config.memory_limit.is_none());
        assert!(config.fuel_budget.is_none());
    }

    /// A node's fuel budget must survive the conversion; it used to reach the
    /// reactor and be read by nothing.
    #[test]
    fn a_nodes_fuel_budget_is_carried_through() {
        let node = NodeDef {
            fuel_budget: Some(5_000_000),
            ..Default::default()
        };
        let config = node_def_to_config(&node).expect("valid");
        assert_eq!(config.fuel_budget, Some(5_000_000));
    }
}
