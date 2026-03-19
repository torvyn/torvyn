//! Top-level pipeline linking orchestrator.
//!
//! `PipelineLinker` is the main entry point for the `torvyn-linker` crate.
//! It takes a `PipelineTopology` and produces a `LinkedPipeline` after
//! validating the topology, resolving all imports, and checking capabilities.
//!
//! Per Doc 02 Section 4.1: linking is a static, ahead-of-execution operation.
//! All errors are reported before any stream data flows.

use std::collections::HashMap;

use torvyn_types::StreamId;

use crate::error::{LinkReport, LinkerError};
use crate::linked_pipeline::{LinkedComponent, LinkedConnection, LinkedPipeline};
use crate::resolver::{check_capability_grants, resolve_imports};
use crate::topology::PipelineTopology;

/// The pipeline linker: orchestrates topology validation, import resolution,
/// capability checking, and linked pipeline construction.
///
/// # Usage
///
/// ```
/// use torvyn_linker::{PipelineLinker, PipelineTopology, TopologyNode, TopologyEdge};
/// use torvyn_types::ComponentRole;
///
/// let mut topo = PipelineTopology::new("my-pipeline".into());
/// topo.add_node(TopologyNode {
///     name: "src".into(),
///     role: ComponentRole::Source,
///     artifact_path: "src.wasm".into(),
///     config: None,
///     capability_grants: vec![],
/// });
/// topo.add_node(TopologyNode {
///     name: "snk".into(),
///     role: ComponentRole::Sink,
///     artifact_path: "snk.wasm".into(),
///     config: None,
///     capability_grants: vec![],
/// });
/// topo.add_edge(TopologyEdge {
///     from_node: "src".into(),
///     from_port: "output".into(),
///     to_node: "snk".into(),
///     to_port: "input".into(),
///     queue_depth: 64,
///     backpressure_policy: Default::default(),
/// });
///
/// let mut linker = PipelineLinker::new();
///
/// // In Phase 0: component_imports and component_exports are extracted
/// // from the compiled components by the host. For now, we pass empty maps
/// // for a topology-only link.
/// let result = linker.link_topology_only(&topo);
/// assert!(result.is_ok());
/// ```
///
/// # COLD PATH — run once at pipeline startup.
pub struct PipelineLinker {
    /// Stream ID counter.
    next_stream_id: u64,
}

impl PipelineLinker {
    /// Create a new `PipelineLinker`.
    ///
    /// # COLD PATH
    pub fn new() -> Self {
        Self { next_stream_id: 0 }
    }

    /// Allocate the next stream ID.
    ///
    /// # COLD PATH
    fn alloc_stream_id(&mut self) -> StreamId {
        let id = StreamId::new(self.next_stream_id);
        self.next_stream_id += 1;
        id
    }

    /// Link a pipeline with full import/export and capability resolution.
    ///
    /// # Arguments
    /// - `topology` — the pipeline topology to link.
    /// - `component_imports` — maps each component name to its WIT import names
    ///   (extracted from compiled components by the host/engine).
    /// - `component_exports` — maps each component name to its WIT export names.
    ///
    /// # Returns
    /// - `Ok(LinkedPipeline)` — fully linked and validated pipeline.
    /// - `Err(LinkerError::LinkFailed(report))` — linking failed; `report` contains
    ///   all diagnostics.
    ///
    /// # Preconditions
    /// - All component artifacts referenced by `topology` have been compiled.
    /// - `component_imports` and `component_exports` contain entries for every
    ///   node in the topology.
    ///
    /// # Postconditions
    /// - On success, the `LinkedPipeline` satisfies all invariants documented
    ///   on `LinkedPipeline`.
    ///
    /// # COLD PATH
    pub fn link(
        &mut self,
        topology: &PipelineTopology,
        component_imports: &HashMap<String, Vec<String>>,
        component_exports: &HashMap<String, Vec<String>>,
    ) -> Result<LinkedPipeline, LinkerError> {
        // Phase 1: Validate topology structure
        let topo_report = topology.validate();
        if !topo_report.is_ok() {
            return Err(LinkerError::LinkFailed(topo_report));
        }

        // Phase 2: Compute topological order
        let topo_order = topology.topological_order().ok_or_else(|| {
            let mut report = LinkReport::new();
            report.push_error(crate::error::LinkDiagnostic {
                category: crate::error::LinkDiagnosticCategory::CyclicDependency,
                message: "Failed to compute topological order (cycle exists).".into(),
                component: None,
                related_component: None,
                interface_name: None,
            });
            LinkerError::LinkFailed(report)
        })?;

        // Phase 3: Resolve imports
        let mut report = LinkReport::new();
        let resolution = resolve_imports(
            topology,
            &topo_order,
            component_imports,
            component_exports,
            &mut report,
        );

        // Phase 4: Check capability grants
        check_capability_grants(topology, component_imports, &mut report);

        // If any errors, fail
        if !report.is_ok() {
            return Err(LinkerError::LinkFailed(report));
        }

        // Phase 5: Construct LinkedPipeline
        let linked = self.build_linked_pipeline(topology, &topo_order, &resolution);

        Ok(linked)
    }

    /// Link a pipeline with topology validation only (no import/export maps).
    ///
    /// This is used in Phase 0 minimal linking where component WIT metadata
    /// is not yet available. It validates the topology and produces a
    /// `LinkedPipeline` with empty import resolutions.
    ///
    /// # COLD PATH
    pub fn link_topology_only(
        &mut self,
        topology: &PipelineTopology,
    ) -> Result<LinkedPipeline, LinkerError> {
        let empty_imports = HashMap::new();
        let empty_exports = HashMap::new();

        // Validate topology
        let topo_report = topology.validate();
        if !topo_report.is_ok() {
            return Err(LinkerError::LinkFailed(topo_report));
        }

        let topo_order = topology.topological_order().ok_or_else(|| {
            let mut report = LinkReport::new();
            report.push_error(crate::error::LinkDiagnostic {
                category: crate::error::LinkDiagnosticCategory::CyclicDependency,
                message: "Failed to compute topological order.".into(),
                component: None,
                related_component: None,
                interface_name: None,
            });
            LinkerError::LinkFailed(report)
        })?;

        let mut report = LinkReport::new();
        let resolution = resolve_imports(
            topology,
            &topo_order,
            &empty_imports,
            &empty_exports,
            &mut report,
        );

        // For topology-only, we don't fail on unresolved imports
        let linked = self.build_linked_pipeline(topology, &topo_order, &resolution);

        Ok(linked)
    }

    /// Build the `LinkedPipeline` from validated and resolved data.
    ///
    /// # COLD PATH
    fn build_linked_pipeline(
        &mut self,
        topology: &PipelineTopology,
        topo_order: &[String],
        resolution: &crate::resolver::PipelineResolution,
    ) -> LinkedPipeline {
        // Build components in topological order
        let components: Vec<LinkedComponent> = topo_order
            .iter()
            .map(|name| {
                let node = &topology.nodes[name];
                let comp_resolution = resolution
                    .components
                    .iter()
                    .find(|c| c.component_name == *name);

                LinkedComponent {
                    name: name.clone(),
                    role: node.role,
                    artifact_path: node.artifact_path.clone(),
                    resolved_imports: comp_resolution
                        .map(|r| r.resolved_imports.clone())
                        .unwrap_or_default(),
                    capability_grants: node.capability_grants.clone(),
                    config: node.config.clone(),
                }
            })
            .collect();

        // Build connections with stream IDs
        let connections: Vec<LinkedConnection> = topology
            .edges
            .iter()
            .map(|edge| LinkedConnection {
                stream_id: self.alloc_stream_id(),
                from_component: edge.from_node.clone(),
                from_port: edge.from_port.clone(),
                to_component: edge.to_node.clone(),
                to_port: edge.to_port.clone(),
                queue_depth: edge.queue_depth,
                backpressure_policy: edge.backpressure_policy,
            })
            .collect();

        LinkedPipeline {
            name: topology.name.clone(),
            components,
            connections,
            topological_order: topo_order.to_vec(),
        }
    }
}

impl Default for PipelineLinker {
    fn default() -> Self {
        Self::new()
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

    fn source_node(name: &str) -> TopologyNode {
        TopologyNode {
            name: name.into(),
            role: ComponentRole::Source,
            artifact_path: format!("{name}.wasm").into(),
            config: None,
            capability_grants: vec![],
        }
    }

    fn processor_node(name: &str) -> TopologyNode {
        TopologyNode {
            name: name.into(),
            role: ComponentRole::Processor,
            artifact_path: format!("{name}.wasm").into(),
            config: None,
            capability_grants: vec![],
        }
    }

    fn sink_node(name: &str) -> TopologyNode {
        TopologyNode {
            name: name.into(),
            role: ComponentRole::Sink,
            artifact_path: format!("{name}.wasm").into(),
            config: None,
            capability_grants: vec![],
        }
    }

    fn edge(from: &str, to: &str) -> TopologyEdge {
        TopologyEdge {
            from_node: from.into(),
            from_port: "output".into(),
            to_node: to.into(),
            to_port: "input".into(),
            queue_depth: 64,
            backpressure_policy: BackpressurePolicy::default(),
        }
    }

    // --- basic_link tests ---

    #[test]
    fn basic_link_two_components() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "snk"));

        let mut linker = PipelineLinker::new();
        let result = linker.link_topology_only(&topo);
        assert!(result.is_ok(), "Expected link success");

        let linked = result.unwrap();
        assert_eq!(linked.component_count(), 2);
        assert_eq!(linked.connection_count(), 1);
        assert_eq!(linked.topological_order, vec!["src", "snk"]);
    }

    #[test]
    fn basic_link_three_components() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(processor_node("proc"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "proc"));
        topo.add_edge(edge("proc", "snk"));

        let mut linker = PipelineLinker::new();
        let linked = linker.link_topology_only(&topo).unwrap();

        assert_eq!(linked.component_count(), 3);
        assert_eq!(linked.connection_count(), 2);

        let src_pos = linked
            .topological_order
            .iter()
            .position(|n| n == "src")
            .unwrap();
        let proc_pos = linked
            .topological_order
            .iter()
            .position(|n| n == "proc")
            .unwrap();
        let snk_pos = linked
            .topological_order
            .iter()
            .position(|n| n == "snk")
            .unwrap();
        assert!(src_pos < proc_pos);
        assert!(proc_pos < snk_pos);
    }

    #[test]
    fn basic_link_fan_out() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk1"));
        topo.add_node(sink_node("snk2"));
        topo.add_edge(edge("src", "snk1"));
        topo.add_edge(edge("src", "snk2"));

        let mut linker = PipelineLinker::new();
        let linked = linker.link_topology_only(&topo).unwrap();
        assert_eq!(linked.component_count(), 3);
        assert_eq!(linked.connection_count(), 2);
    }

    #[test]
    fn basic_link_stream_ids_are_unique() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(processor_node("p1"));
        topo.add_node(processor_node("p2"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "p1"));
        topo.add_edge(edge("src", "p2"));
        topo.add_edge(edge("p1", "snk"));
        topo.add_edge(edge("p2", "snk"));

        let mut linker = PipelineLinker::new();
        let linked = linker.link_topology_only(&topo).unwrap();

        let ids: Vec<u64> = linked
            .connections
            .iter()
            .map(|c| c.stream_id.as_u64())
            .collect();
        let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "all stream IDs must be unique");
    }

    // --- link_errors tests ---

    #[test]
    fn link_errors_empty_topology() {
        let topo = PipelineTopology::new("test".into());
        let mut linker = PipelineLinker::new();
        let result = linker.link_topology_only(&topo);
        assert!(result.is_err());
    }

    #[test]
    fn link_errors_cycle() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(processor_node("a"));
        topo.add_node(processor_node("b"));
        topo.add_edge(edge("a", "b"));
        topo.add_edge(edge("b", "a"));

        let mut linker = PipelineLinker::new();
        let result = linker.link_topology_only(&topo);
        assert!(result.is_err());
    }

    #[test]
    fn link_errors_type_mismatch() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "snk"));

        let mut imports = HashMap::new();
        imports.insert("snk".into(), vec!["my-custom/interface".into()]);
        let mut exports = HashMap::new();
        exports.insert("src".into(), vec![]); // Does NOT export what snk needs

        let mut linker = PipelineLinker::new();
        let result = linker.link(&topo, &imports, &exports);
        assert!(result.is_err());

        if let Err(LinkerError::LinkFailed(report)) = result {
            assert!(report
                .errors
                .iter()
                .any(|e| { e.category == crate::error::LinkDiagnosticCategory::UnresolvedImport }));
        }
    }

    #[test]
    fn link_errors_missing_capability() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(TopologyNode {
            name: "src".into(),
            role: ComponentRole::Source,
            artifact_path: "src.wasm".into(),
            config: None,
            capability_grants: vec![], // No grants
        });
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "snk"));

        let mut imports = HashMap::new();
        imports.insert("src".into(), vec!["wasi:filesystem/preopens".into()]);
        let exports = HashMap::new();

        let mut linker = PipelineLinker::new();
        let result = linker.link(&topo, &imports, &exports);
        assert!(result.is_err());

        if let Err(LinkerError::LinkFailed(report)) = result {
            assert!(report
                .errors
                .iter()
                .any(|e| { e.category == crate::error::LinkDiagnosticCategory::CapabilityDenied }));
        }
    }

    #[test]
    fn link_full_success_with_wasi_imports() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "snk"));

        let mut imports = HashMap::new();
        imports.insert(
            "src".into(),
            vec![
                "wasi:io/streams".into(),
                "torvyn:resources/buffer-ops".into(),
            ],
        );
        imports.insert("snk".into(), vec!["wasi:io/streams".into()]);

        let mut exports = HashMap::new();
        exports.insert("src".into(), vec![]);
        exports.insert("snk".into(), vec![]);

        let mut linker = PipelineLinker::new();
        let result = linker.link(&topo, &imports, &exports);
        assert!(result.is_ok(), "Expected link success: {:?}", result.err());

        let linked = result.unwrap();
        assert_eq!(linked.component_count(), 2);

        let src_comp = linked.get_component("src").unwrap();
        assert!(src_comp.resolved_imports.contains_key("wasi:io/streams"));
        assert!(src_comp
            .resolved_imports
            .contains_key("torvyn:resources/buffer-ops"));
    }

    #[test]
    fn link_reports_multiple_errors() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(TopologyNode {
            name: "src".into(),
            role: ComponentRole::Source,
            artifact_path: "src.wasm".into(),
            config: None,
            capability_grants: vec![], // Missing capability
        });
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "snk"));

        let mut imports = HashMap::new();
        imports.insert(
            "src".into(),
            vec![
                "wasi:filesystem/preopens".into(), // Needs capability
                "some:unknown/interface".into(),   // Unresolved
            ],
        );
        let exports = HashMap::new();

        let mut linker = PipelineLinker::new();
        let result = linker.link(&topo, &imports, &exports);
        assert!(result.is_err());

        if let Err(LinkerError::LinkFailed(report)) = result {
            // Should have at least 2 errors: unresolved + capability denied
            assert!(
                report.error_count() >= 2,
                "Expected at least 2 errors, got {}: {}",
                report.error_count(),
                report.format_all()
            );
        }
    }
}
