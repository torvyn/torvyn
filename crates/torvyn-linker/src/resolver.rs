//! Import/export resolution for component linking.
//!
//! Per Doc 02 Section 4.2: the linker resolves imports in topological order,
//! classifying each as a stream connection, WASI interface, Torvyn built-in,
//! or unresolved.
//!
//! Diamond dependency handling per Doc 02 Section 4.3:
//! - Host-provided interfaces are canonical (no ambiguity).
//! - Component-provided interfaces require explicit provider naming.

use std::collections::{HashMap, HashSet};

use crate::error::{LinkDiagnostic, LinkDiagnosticCategory, LinkReport};
use crate::topology::PipelineTopology;

/// Known WASI interfaces provided by the host.
///
/// Per Doc 02 Section 4.4: these are always resolved to the host's
/// WASI implementation, potentially wrapped by torvyn-security.
const HOST_PROVIDED_WASI: &[&str] = &[
    "wasi:io/streams",
    "wasi:io/poll",
    "wasi:clocks/monotonic-clock",
    "wasi:clocks/wall-clock",
    "wasi:random/random",
    "wasi:random/insecure",
    "wasi:random/insecure-seed",
    "wasi:filesystem/types",
    "wasi:filesystem/preopens",
    "wasi:sockets/tcp",
    "wasi:sockets/udp",
    "wasi:sockets/ip-name-lookup",
    "wasi:cli/stdin",
    "wasi:cli/stdout",
    "wasi:cli/stderr",
    "wasi:cli/environment",
];

/// Known Torvyn built-in interfaces provided by the host.
const HOST_PROVIDED_TORVYN: &[&str] = &[
    "torvyn:resources/buffer-ops",
    "torvyn:runtime/flow-context",
    "torvyn:runtime/backpressure",
    "torvyn:capabilities/capability-query",
];

/// Classification of a resolved import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportResolution {
    /// Resolved to a stream connection from an upstream component's export.
    StreamConnection {
        /// Name of the upstream component providing the export.
        provider: String,
        /// The interface name on the provider.
        export_name: String,
    },
    /// Resolved to a host-provided WASI implementation.
    HostWasi {
        /// The WASI interface name.
        interface_name: String,
        /// Whether this WASI interface is capability-gated.
        capability_gated: bool,
    },
    /// Resolved to a Torvyn built-in host interface.
    HostTorvyn {
        /// The Torvyn interface name.
        interface_name: String,
    },
}

/// The complete import resolution for a single component.
#[derive(Debug, Clone)]
pub struct ComponentResolution {
    /// Component name.
    pub component_name: String,
    /// Map from import name to its resolution.
    pub resolved_imports: HashMap<String, ImportResolution>,
    /// Imports that could not be resolved (errors).
    pub unresolved: Vec<String>,
}

/// The complete import resolution for the entire pipeline.
#[derive(Debug, Clone)]
pub struct PipelineResolution {
    /// Per-component resolutions, in topological order.
    pub components: Vec<ComponentResolution>,
}

/// Capability-gated WASI interfaces.
///
/// Per Doc 02 Section 4.4: these interfaces return permission-denied
/// if the component lacks the corresponding capability grant.
const CAPABILITY_GATED_WASI: &[&str] = &[
    "wasi:filesystem/types",
    "wasi:filesystem/preopens",
    "wasi:sockets/tcp",
    "wasi:sockets/udp",
    "wasi:sockets/ip-name-lookup",
    "wasi:random/random",
    "wasi:random/insecure",
    "wasi:random/insecure-seed",
];

/// Resolve all imports for a pipeline.
///
/// Per Doc 02 Section 4.2 algorithm:
/// 1. For each component in topological order:
///    a. For each import:
///    - Stream connection → resolve to upstream export.
///    - WASI interface → resolve to host WASI impl.
///    - Torvyn built-in → resolve to host Torvyn impl.
///    - Otherwise → unresolved import error.
///
/// # Preconditions
/// - `topology` has been validated (no cycles, connected, role-consistent).
/// - `topo_order` is a valid topological ordering of node names.
/// - `component_imports` maps each component name to its set of WIT import names.
/// - `component_exports` maps each component name to its set of WIT export names.
///
/// # Postconditions
/// - Returns `PipelineResolution` with all resolved imports.
/// - Adds diagnostics to `report` for unresolved and ambiguous imports.
///
/// # COLD PATH
pub fn resolve_imports(
    topology: &PipelineTopology,
    topo_order: &[String],
    component_imports: &HashMap<String, Vec<String>>,
    component_exports: &HashMap<String, Vec<String>>,
    report: &mut LinkReport,
) -> PipelineResolution {
    // Build edge map: downstream → list of upstream nodes
    let mut upstream_map: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &topology.edges {
        upstream_map
            .entry(edge.to_node.as_str())
            .or_default()
            .push(edge.from_node.as_str());
    }

    let mut resolution = PipelineResolution {
        components: Vec::with_capacity(topo_order.len()),
    };

    for node_name in topo_order {
        let imports = match component_imports.get(node_name) {
            Some(imports) => imports,
            None => {
                resolution.components.push(ComponentResolution {
                    component_name: node_name.clone(),
                    resolved_imports: HashMap::new(),
                    unresolved: Vec::new(),
                });
                continue;
            }
        };

        let mut resolved = HashMap::new();
        let mut unresolved = Vec::new();

        for import_name in imports {
            if let Some(res) = resolve_single_import(
                import_name,
                node_name,
                &upstream_map,
                component_exports,
                report,
            ) {
                resolved.insert(import_name.clone(), res);
            } else {
                unresolved.push(import_name.clone());
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::UnresolvedImport,
                    message: format!(
                        "Component '{}' requires import '{}' but no provider was found. \
                         Ensure a connected upstream component exports this interface, \
                         or verify it is a known WASI/Torvyn host interface.",
                        node_name, import_name
                    ),
                    component: Some(node_name.clone()),
                    related_component: None,
                    interface_name: Some(import_name.clone()),
                });
            }
        }

        resolution.components.push(ComponentResolution {
            component_name: node_name.clone(),
            resolved_imports: resolved,
            unresolved,
        });
    }

    resolution
}

/// Resolve a single import for a component.
///
/// Per Doc 02 Section 4.2:
/// 1. Check if it's a known WASI interface → `HostWasi`
/// 2. Check if it's a known Torvyn interface → `HostTorvyn`
/// 3. Check if an upstream component exports it → `StreamConnection`
/// 4. Otherwise → None (unresolved)
///
/// # COLD PATH
fn resolve_single_import(
    import_name: &str,
    node_name: &str,
    upstream_map: &HashMap<&str, Vec<&str>>,
    component_exports: &HashMap<String, Vec<String>>,
    report: &mut LinkReport,
) -> Option<ImportResolution> {
    // 1. Check host-provided WASI
    for &wasi_iface in HOST_PROVIDED_WASI {
        if import_name.contains(wasi_iface) || import_name == wasi_iface {
            let capability_gated = CAPABILITY_GATED_WASI
                .iter()
                .any(|&cg| import_name.contains(cg) || import_name == cg);
            return Some(ImportResolution::HostWasi {
                interface_name: import_name.to_string(),
                capability_gated,
            });
        }
    }

    // 2. Check host-provided Torvyn built-in
    for &torvyn_iface in HOST_PROVIDED_TORVYN {
        if import_name.contains(torvyn_iface) || import_name == torvyn_iface {
            return Some(ImportResolution::HostTorvyn {
                interface_name: import_name.to_string(),
            });
        }
    }

    // 3. Check upstream component exports
    let upstreams = upstream_map.get(node_name).cloned().unwrap_or_default();
    let mut providers: Vec<&str> = Vec::new();

    for &upstream in &upstreams {
        if let Some(exports) = component_exports.get(upstream) {
            if exports.iter().any(|e| e == import_name) {
                providers.push(upstream);
            }
        }
    }

    match providers.len() {
        0 => None, // Unresolved
        1 => Some(ImportResolution::StreamConnection {
            provider: providers[0].to_string(),
            export_name: import_name.to_string(),
        }),
        _ => {
            // Diamond dependency: ambiguous provider
            // Per Doc 02 Section 4.3: emit an ambiguity error
            report.push_error(LinkDiagnostic {
                category: LinkDiagnosticCategory::AmbiguousProvider,
                message: format!(
                    "Component '{}' imports '{}' but multiple upstream components export it: [{}]. \
                     Disambiguate by specifying the provider in the pipeline configuration.",
                    node_name,
                    import_name,
                    providers.join(", ")
                ),
                component: Some(node_name.to_string()),
                related_component: None,
                interface_name: Some(import_name.to_string()),
            });
            // Return the first provider as a best-effort resolution
            Some(ImportResolution::StreamConnection {
                provider: providers[0].to_string(),
                export_name: import_name.to_string(),
            })
        }
    }
}

/// Check capability grants for all components.
///
/// For each component that imports a capability-gated WASI interface,
/// verify that the required capability is granted in the topology.
///
/// # COLD PATH
pub fn check_capability_grants(
    topology: &PipelineTopology,
    component_imports: &HashMap<String, Vec<String>>,
    report: &mut LinkReport,
) {
    /// Mapping from WASI interface to required capability name.
    /// Mirrors the mapping in torvyn-contracts validator.
    const WASI_TO_CAPABILITY: &[(&str, &str)] = &[
        ("wasi:filesystem/preopens", "wasi-filesystem-read"),
        ("wasi:filesystem/types", "wasi-filesystem-read"),
        ("wasi:sockets/tcp", "wasi-network-egress"),
        ("wasi:sockets/udp", "wasi-network-egress"),
        ("wasi:sockets/ip-name-lookup", "wasi-network-egress"),
        ("wasi:clocks/wall-clock", "wasi-clocks"),
        ("wasi:clocks/monotonic-clock", "wasi-clocks"),
        ("wasi:random/random", "wasi-random"),
        ("wasi:random/insecure", "wasi-random"),
        ("wasi:random/insecure-seed", "wasi-random"),
    ];

    for (node_name, node) in &topology.nodes {
        let grants: HashSet<&str> = node
            .capability_grants
            .iter()
            .map(|g| g.name.as_str())
            .collect();

        let imports = match component_imports.get(node_name) {
            Some(i) => i,
            None => continue,
        };

        for import_name in imports {
            for &(wasi_iface, capability_name) in WASI_TO_CAPABILITY {
                if import_name.contains(wasi_iface) && !grants.contains(capability_name) {
                    report.push_error(LinkDiagnostic {
                        category: LinkDiagnosticCategory::CapabilityDenied,
                        message: format!(
                            "Component '{}' imports '{}' which requires capability '{}', \
                             but this capability is not granted. \
                             Add '{}' to the component's capability grants in Torvyn.toml.",
                            node_name, import_name, capability_name, capability_name
                        ),
                        component: Some(node_name.clone()),
                        related_component: None,
                        interface_name: Some(capability_name.to_string()),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::*;
    use torvyn_types::{BackpressurePolicy, ComponentRole};

    type CapabilityMap = HashMap<String, Vec<String>>;

    fn make_topo_and_imports() -> (PipelineTopology, Vec<String>, CapabilityMap, CapabilityMap) {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(TopologyNode {
            name: "src".into(),
            role: ComponentRole::Source,
            artifact_path: "src.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_node(TopologyNode {
            name: "snk".into(),
            role: ComponentRole::Sink,
            artifact_path: "snk.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_edge(TopologyEdge {
            from_node: "src".into(),
            from_port: "output".into(),
            to_node: "snk".into(),
            to_port: "input".into(),
            queue_depth: 64,
            backpressure_policy: BackpressurePolicy::default(),
        });

        let topo_order = vec!["src".to_string(), "snk".to_string()];

        let mut imports = HashMap::new();
        imports.insert(
            "src".into(),
            vec![
                "wasi:io/streams".into(),
                "torvyn:resources/buffer-ops".into(),
            ],
        );
        imports.insert(
            "snk".into(),
            vec![
                "wasi:io/streams".into(),
                "torvyn:streaming/processor".into(),
            ],
        );

        let mut exports = HashMap::new();
        exports.insert("src".into(), vec!["torvyn:streaming/processor".into()]);
        exports.insert("snk".into(), vec![]);

        (topo, topo_order, imports, exports)
    }

    #[test]
    fn test_resolve_wasi_import() {
        let (topo, topo_order, imports, exports) = make_topo_and_imports();
        let mut report = LinkReport::new();

        let resolution = resolve_imports(&topo, &topo_order, &imports, &exports, &mut report);
        assert!(report.is_ok(), "{}", report.format_all());

        let src_res = &resolution.components[0];
        assert!(src_res.resolved_imports.contains_key("wasi:io/streams"));
        assert!(matches!(
            src_res.resolved_imports["wasi:io/streams"],
            ImportResolution::HostWasi { .. }
        ));
    }

    #[test]
    fn test_resolve_torvyn_builtin_import() {
        let (topo, topo_order, imports, exports) = make_topo_and_imports();
        let mut report = LinkReport::new();

        let resolution = resolve_imports(&topo, &topo_order, &imports, &exports, &mut report);

        let src_res = &resolution.components[0];
        assert!(src_res
            .resolved_imports
            .contains_key("torvyn:resources/buffer-ops"));
        assert!(matches!(
            src_res.resolved_imports["torvyn:resources/buffer-ops"],
            ImportResolution::HostTorvyn { .. }
        ));
    }

    #[test]
    fn test_resolve_stream_connection() {
        let (topo, topo_order, imports, exports) = make_topo_and_imports();
        let mut report = LinkReport::new();

        let resolution = resolve_imports(&topo, &topo_order, &imports, &exports, &mut report);

        let snk_res = &resolution.components[1];
        assert!(snk_res
            .resolved_imports
            .contains_key("torvyn:streaming/processor"));
        match &snk_res.resolved_imports["torvyn:streaming/processor"] {
            ImportResolution::StreamConnection { provider, .. } => {
                assert_eq!(provider, "src");
            }
            other => panic!("expected StreamConnection, got: {:?}", other),
        }
    }

    #[test]
    fn test_unresolved_import_reported() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(TopologyNode {
            name: "src".into(),
            role: ComponentRole::Source,
            artifact_path: "src.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_node(TopologyNode {
            name: "snk".into(),
            role: ComponentRole::Sink,
            artifact_path: "snk.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_edge(TopologyEdge {
            from_node: "src".into(),
            from_port: "output".into(),
            to_node: "snk".into(),
            to_port: "input".into(),
            queue_depth: 64,
            backpressure_policy: BackpressurePolicy::default(),
        });

        let topo_order = vec!["src".to_string(), "snk".to_string()];
        let mut imports = HashMap::new();
        imports.insert("snk".into(), vec!["some:unknown/interface".into()]);
        let exports = HashMap::new();

        let mut report = LinkReport::new();
        resolve_imports(&topo, &topo_order, &imports, &exports, &mut report);

        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| {
            e.category == LinkDiagnosticCategory::UnresolvedImport
                && e.message.contains("some:unknown/interface")
        }));
    }

    #[test]
    fn test_ambiguous_provider_reported() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(TopologyNode {
            name: "a".into(),
            role: ComponentRole::Processor,
            artifact_path: "a.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_node(TopologyNode {
            name: "b".into(),
            role: ComponentRole::Processor,
            artifact_path: "b.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_node(TopologyNode {
            name: "c".into(),
            role: ComponentRole::Sink,
            artifact_path: "c.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_edge(TopologyEdge {
            from_node: "a".into(),
            from_port: "output".into(),
            to_node: "c".into(),
            to_port: "input".into(),
            queue_depth: 64,
            backpressure_policy: Default::default(),
        });
        topo.add_edge(TopologyEdge {
            from_node: "b".into(),
            from_port: "output".into(),
            to_node: "c".into(),
            to_port: "input2".into(),
            queue_depth: 64,
            backpressure_policy: Default::default(),
        });

        let topo_order = vec!["a".into(), "b".into(), "c".into()];
        let mut imports = HashMap::new();
        imports.insert("c".into(), vec!["shared-interface".into()]);
        let mut exports = HashMap::new();
        exports.insert("a".into(), vec!["shared-interface".into()]);
        exports.insert("b".into(), vec!["shared-interface".into()]);

        let mut report = LinkReport::new();
        resolve_imports(&topo, &topo_order, &imports, &exports, &mut report);

        assert!(report
            .errors
            .iter()
            .any(|e| { e.category == LinkDiagnosticCategory::AmbiguousProvider }));
    }

    #[test]
    fn test_capability_grant_check_missing_capability() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(TopologyNode {
            name: "reader".into(),
            role: ComponentRole::Source,
            artifact_path: "reader.wasm".into(),
            config: None,
            capability_grants: vec![], // No grants!
        });
        topo.add_node(TopologyNode {
            name: "snk".into(),
            role: ComponentRole::Sink,
            artifact_path: "snk.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_edge(TopologyEdge {
            from_node: "reader".into(),
            from_port: "output".into(),
            to_node: "snk".into(),
            to_port: "input".into(),
            queue_depth: 64,
            backpressure_policy: Default::default(),
        });

        let mut imports = HashMap::new();
        imports.insert("reader".into(), vec!["wasi:filesystem/preopens".into()]);

        let mut report = LinkReport::new();
        check_capability_grants(&topo, &imports, &mut report);

        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| {
            e.category == LinkDiagnosticCategory::CapabilityDenied
                && e.message.contains("wasi-filesystem-read")
        }));
    }

    #[test]
    fn test_capability_grant_check_granted() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(TopologyNode {
            name: "reader".into(),
            role: ComponentRole::Source,
            artifact_path: "reader.wasm".into(),
            config: None,
            capability_grants: vec![CapabilityGrant {
                name: "wasi-filesystem-read".into(),
                detail: "/data/*".into(),
            }],
        });
        topo.add_node(TopologyNode {
            name: "snk".into(),
            role: ComponentRole::Sink,
            artifact_path: "snk.wasm".into(),
            config: None,
            capability_grants: vec![],
        });
        topo.add_edge(TopologyEdge {
            from_node: "reader".into(),
            from_port: "output".into(),
            to_node: "snk".into(),
            to_port: "input".into(),
            queue_depth: 64,
            backpressure_policy: Default::default(),
        });

        let mut imports = HashMap::new();
        imports.insert("reader".into(), vec!["wasi:filesystem/preopens".into()]);

        let mut report = LinkReport::new();
        check_capability_grants(&topo, &imports, &mut report);

        assert!(report.is_ok(), "{}", report.format_all());
    }
}
