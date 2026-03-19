//! Fluent builder for constructing [`PipelineTopology`] instances.
//!
//! Per Doc 02, Section 5.2: for embedding Torvyn as a library, a programmatic
//! API is available alongside TOML configuration.
//!
//! # Examples
//! ```
//! use torvyn_pipeline::{PipelineTopologyBuilder, NodeConfig};
//! use torvyn_types::ComponentRole;
//!
//! let topology = PipelineTopologyBuilder::new("my-pipeline")
//!     .description("A simple three-stage pipeline")
//!     .add_node("source-1", ComponentRole::Source, "file://source.wasm", NodeConfig::default())
//!     .add_node("transform-1", ComponentRole::Processor, "file://transform.wasm", NodeConfig::default())
//!     .add_node("sink-1", ComponentRole::Sink, "file://sink.wasm", NodeConfig::default())
//!     .add_edge("source-1", "output", "transform-1", "input")
//!     .add_edge("transform-1", "output", "sink-1", "input")
//!     .build()
//!     .unwrap();
//!
//! assert_eq!(topology.node_count(), 3);
//! assert_eq!(topology.edge_count(), 2);
//! ```

use std::collections::HashMap;

use torvyn_types::ComponentRole;

use crate::error::{PipelineError, ValidationReport};
use crate::topology::{EdgeConfig, NodeConfig, PipelineTopology, TopologyEdge, TopologyNode};
use crate::validate;

// ---------------------------------------------------------------------------
// PipelineTopologyBuilder
// ---------------------------------------------------------------------------

/// Fluent builder for constructing a [`PipelineTopology`].
///
/// The builder collects nodes and edges, then validates the topology on
/// `build()`. If validation fails, all errors are returned.
///
/// # COLD PATH — used during pipeline construction.
pub struct PipelineTopologyBuilder {
    name: String,
    description: String,
    nodes: Vec<TopologyNode>,
    node_index: HashMap<String, usize>,
    edges: Vec<PendingEdge>,
}

/// An edge defined by node names (not yet resolved to indices).
struct PendingEdge {
    from_node: String,
    from_port: String,
    to_node: String,
    to_port: String,
    config: EdgeConfig,
}

impl PipelineTopologyBuilder {
    /// Create a new builder for a pipeline with the given name.
    ///
    /// # COLD PATH
    ///
    /// # Examples
    /// ```
    /// use torvyn_pipeline::PipelineTopologyBuilder;
    ///
    /// let builder = PipelineTopologyBuilder::new("my-pipeline");
    /// ```
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            description: String::new(),
            nodes: Vec::new(),
            node_index: HashMap::new(),
            edges: Vec::new(),
        }
    }

    /// Set the pipeline description.
    ///
    /// # COLD PATH
    pub fn description(mut self, description: &str) -> Self {
        description.clone_into(&mut self.description);
        self
    }

    /// Add a node to the topology.
    ///
    /// # COLD PATH
    ///
    /// # Panics
    /// Does not panic. Duplicate names are detected during `build()`.
    pub fn add_node(
        mut self,
        name: &str,
        role: ComponentRole,
        component_ref: &str,
        config: NodeConfig,
    ) -> Self {
        let index = self.nodes.len();
        self.nodes.push(TopologyNode::with_config(
            name.to_owned(),
            component_ref.to_owned(),
            role_to_interface(role).to_owned(),
            role,
            config,
        ));
        self.node_index.insert(name.to_owned(), index);
        self
    }

    /// Add a node with an explicit WIT interface string.
    ///
    /// # COLD PATH
    pub fn add_node_with_interface(
        mut self,
        name: &str,
        role: ComponentRole,
        component_ref: &str,
        interface: &str,
        config: NodeConfig,
    ) -> Self {
        let index = self.nodes.len();
        self.nodes.push(TopologyNode::with_config(
            name.to_owned(),
            component_ref.to_owned(),
            interface.to_owned(),
            role,
            config,
        ));
        self.node_index.insert(name.to_owned(), index);
        self
    }

    /// Add an edge connecting two nodes.
    ///
    /// # COLD PATH
    pub fn add_edge(
        mut self,
        from_node: &str,
        from_port: &str,
        to_node: &str,
        to_port: &str,
    ) -> Self {
        self.edges.push(PendingEdge {
            from_node: from_node.to_owned(),
            from_port: from_port.to_owned(),
            to_node: to_node.to_owned(),
            to_port: to_port.to_owned(),
            config: EdgeConfig::default(),
        });
        self
    }

    /// Add an edge with per-edge configuration.
    ///
    /// # COLD PATH
    pub fn add_edge_with_config(
        mut self,
        from_node: &str,
        from_port: &str,
        to_node: &str,
        to_port: &str,
        config: EdgeConfig,
    ) -> Self {
        self.edges.push(PendingEdge {
            from_node: from_node.to_owned(),
            from_port: from_port.to_owned(),
            to_node: to_node.to_owned(),
            to_port: to_port.to_owned(),
            config,
        });
        self
    }

    /// Build the topology, validating all constraints.
    ///
    /// # COLD PATH
    ///
    /// # Errors
    /// Returns `Err(Vec<PipelineError>)` if validation fails.
    /// All errors are collected — not fail-fast.
    ///
    /// # Postconditions
    /// On success, the returned `PipelineTopology` satisfies all invariants:
    /// acyclic, connected, role-consistent, fan limits respected.
    pub fn build(self) -> Result<PipelineTopology, Vec<PipelineError>> {
        let mut report = ValidationReport::new(&self.name);

        // Check for duplicate node names
        {
            let mut seen = std::collections::HashSet::new();
            for node in &self.nodes {
                if !seen.insert(node.name()) {
                    report.push(PipelineError::DuplicateNode {
                        flow_name: self.name.clone(),
                        node_name: node.name().to_owned(),
                    });
                }
            }
        }

        // Resolve pending edges to index-based edges
        let mut resolved_edges = Vec::with_capacity(self.edges.len());
        for (i, pending) in self.edges.iter().enumerate() {
            let Some(&from_idx) = self.node_index.get(&pending.from_node) else {
                report.push(PipelineError::EdgeReferencesUnknownNode {
                    flow_name: self.name.clone(),
                    edge_index: i,
                    node_name: pending.from_node.clone(),
                    endpoint: "from",
                });
                continue;
            };

            let Some(&to_idx) = self.node_index.get(&pending.to_node) else {
                report.push(PipelineError::EdgeReferencesUnknownNode {
                    flow_name: self.name.clone(),
                    edge_index: i,
                    node_name: pending.to_node.clone(),
                    endpoint: "to",
                });
                continue;
            };

            if from_idx == to_idx {
                report.push(PipelineError::SelfLoop {
                    flow_name: self.name.clone(),
                    node_name: pending.from_node.clone(),
                });
                continue;
            }

            resolved_edges.push(TopologyEdge::with_config(
                from_idx,
                pending.from_port.clone(),
                to_idx,
                pending.to_port.clone(),
                pending.config.clone(),
            ));
        }

        // If we already have construction errors, return early
        if !report.is_ok() {
            return Err(report.into_result().unwrap_err());
        }

        // Run full topology validation to compute execution_order
        let execution_order =
            validate::validate_topology(&self.name, &self.nodes, &resolved_edges, &mut report);

        if !report.is_ok() {
            return Err(report.into_result().unwrap_err());
        }

        Ok(PipelineTopology::new_unchecked(
            self.name,
            self.description,
            self.nodes,
            self.node_index,
            resolved_edges,
            execution_order,
        ))
    }
}

/// Map a `ComponentRole` to the canonical Torvyn WIT interface name.
///
/// # COLD PATH
fn role_to_interface(role: ComponentRole) -> &'static str {
    match role {
        ComponentRole::Source => "torvyn:streaming/source",
        ComponentRole::Processor => "torvyn:streaming/processor",
        ComponentRole::Sink => "torvyn:streaming/sink",
        ComponentRole::Filter => "torvyn:streaming/filter",
        ComponentRole::Router => "torvyn:streaming/router",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_simple_linear_pipeline() {
        let topo = PipelineTopologyBuilder::new("linear")
            .add_node(
                "src",
                ComponentRole::Source,
                "file://s.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "proc",
                ComponentRole::Processor,
                "file://p.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "snk",
                ComponentRole::Sink,
                "file://k.wasm",
                NodeConfig::default(),
            )
            .add_edge("src", "output", "proc", "input")
            .add_edge("proc", "output", "snk", "input")
            .build()
            .unwrap();

        assert_eq!(topo.node_count(), 3);
        assert_eq!(topo.edge_count(), 2);
        assert_eq!(topo.execution_order().len(), 3);
        assert_eq!(topo.name(), "linear");
    }

    #[test]
    fn test_build_source_to_sink() {
        let topo = PipelineTopologyBuilder::new("minimal")
            .add_node(
                "src",
                ComponentRole::Source,
                "file://s.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "snk",
                ComponentRole::Sink,
                "file://k.wasm",
                NodeConfig::default(),
            )
            .add_edge("src", "output", "snk", "input")
            .build()
            .unwrap();

        assert_eq!(topo.node_count(), 2);
        assert_eq!(topo.edge_count(), 1);
    }

    #[test]
    fn test_build_fan_out() {
        let topo = PipelineTopologyBuilder::new("fanout")
            .add_node(
                "src",
                ComponentRole::Source,
                "file://s.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "snk1",
                ComponentRole::Sink,
                "file://k1.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "snk2",
                ComponentRole::Sink,
                "file://k2.wasm",
                NodeConfig::default(),
            )
            .add_edge("src", "output", "snk1", "input")
            .add_edge("src", "output", "snk2", "input")
            .build()
            .unwrap();

        assert_eq!(topo.node_count(), 3);
        assert_eq!(topo.edge_count(), 2);
    }

    #[test]
    fn test_build_fan_in() {
        let topo = PipelineTopologyBuilder::new("fanin")
            .add_node(
                "src1",
                ComponentRole::Source,
                "file://s1.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "src2",
                ComponentRole::Source,
                "file://s2.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "snk",
                ComponentRole::Sink,
                "file://k.wasm",
                NodeConfig::default(),
            )
            .add_edge("src1", "output", "snk", "input")
            .add_edge("src2", "output", "snk", "input")
            .build()
            .unwrap();

        assert_eq!(topo.node_count(), 3);
        assert_eq!(topo.edge_count(), 2);
    }

    #[test]
    fn test_build_fails_duplicate_node_name() {
        let result = PipelineTopologyBuilder::new("dup")
            .add_node(
                "x",
                ComponentRole::Source,
                "file://a.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "x",
                ComponentRole::Sink,
                "file://b.wasm",
                NodeConfig::default(),
            )
            .add_edge("x", "output", "x", "input")
            .build();

        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, PipelineError::DuplicateNode { .. })));
    }

    #[test]
    fn test_build_fails_unknown_node_in_edge() {
        let result = PipelineTopologyBuilder::new("bad-edge")
            .add_node(
                "src",
                ComponentRole::Source,
                "file://s.wasm",
                NodeConfig::default(),
            )
            .add_edge("src", "output", "nonexistent", "input")
            .build();

        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            PipelineError::EdgeReferencesUnknownNode { node_name, .. } if node_name == "nonexistent"
        )));
    }

    #[test]
    fn test_build_fails_self_loop() {
        let result = PipelineTopologyBuilder::new("loop")
            .add_node(
                "a",
                ComponentRole::Processor,
                "file://a.wasm",
                NodeConfig::default(),
            )
            .add_edge("a", "output", "a", "input")
            .build();

        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, PipelineError::SelfLoop { .. })));
    }

    #[test]
    fn test_node_index_by_name() {
        let topo = PipelineTopologyBuilder::new("idx")
            .add_node(
                "alpha",
                ComponentRole::Source,
                "file://a.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "beta",
                ComponentRole::Sink,
                "file://b.wasm",
                NodeConfig::default(),
            )
            .add_edge("alpha", "output", "beta", "input")
            .build()
            .unwrap();

        assert_eq!(topo.node_index_by_name("alpha"), Some(0));
        assert_eq!(topo.node_index_by_name("beta"), Some(1));
        assert_eq!(topo.node_index_by_name("gamma"), None);
    }

    #[test]
    fn test_source_and_sink_nodes() {
        let topo = PipelineTopologyBuilder::new("roles")
            .add_node(
                "s",
                ComponentRole::Source,
                "file://s.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "p",
                ComponentRole::Processor,
                "file://p.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "k",
                ComponentRole::Sink,
                "file://k.wasm",
                NodeConfig::default(),
            )
            .add_edge("s", "output", "p", "input")
            .add_edge("p", "output", "k", "input")
            .build()
            .unwrap();

        let sources = topo.source_nodes();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].1.name(), "s");

        let sinks = topo.sink_nodes();
        assert_eq!(sinks.len(), 1);
        assert_eq!(sinks[0].1.name(), "k");
    }

    #[test]
    fn test_outgoing_and_incoming_edges() {
        let topo = PipelineTopologyBuilder::new("edges")
            .add_node(
                "s",
                ComponentRole::Source,
                "file://s.wasm",
                NodeConfig::default(),
            )
            .add_node(
                "k",
                ComponentRole::Sink,
                "file://k.wasm",
                NodeConfig::default(),
            )
            .add_edge("s", "output", "k", "input")
            .build()
            .unwrap();

        let outgoing = topo.outgoing_edges(0);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].1.to_node(), 1);

        let incoming = topo.incoming_edges(1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].1.from_node(), 0);
    }

    #[test]
    fn test_role_to_interface() {
        assert_eq!(
            role_to_interface(ComponentRole::Source),
            "torvyn:streaming/source"
        );
        assert_eq!(
            role_to_interface(ComponentRole::Processor),
            "torvyn:streaming/processor"
        );
        assert_eq!(
            role_to_interface(ComponentRole::Sink),
            "torvyn:streaming/sink"
        );
        assert_eq!(
            role_to_interface(ComponentRole::Filter),
            "torvyn:streaming/filter"
        );
        assert_eq!(
            role_to_interface(ComponentRole::Router),
            "torvyn:streaming/router"
        );
    }
}
