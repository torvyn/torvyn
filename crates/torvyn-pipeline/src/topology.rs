//! Core pipeline topology data model.
//!
//! [`PipelineTopology`] is a validated directed acyclic graph (DAG) of
//! component instances ([`TopologyNode`]) connected by typed stream edges
//! ([`TopologyEdge`]).
//!
//! Per Doc 02, Section 5.1: a pipeline (flow) is a DAG where:
//! - Nodes are component instances with a role and configuration.
//! - Edges are typed streams between output and input ports.
//! - Sources have no incoming edges; sinks have no outgoing edges.

use std::collections::HashMap;
use std::time::Duration;

use torvyn_types::{BackpressurePolicy, ComponentRole};

// ---------------------------------------------------------------------------
// PipelineTopology
// ---------------------------------------------------------------------------

/// A validated pipeline topology, ready for instantiation.
///
/// This is the output of the builder or config-to-topology conversion,
/// after passing through [`crate::validate::validate_topology`].
///
/// # Invariants
/// - `nodes` is non-empty.
/// - `edges` forms a DAG (no cycles).
/// - Every node is reachable from at least one source.
/// - Source nodes have no incoming edges; sink nodes have no outgoing edges.
/// - `execution_order` contains all node indices in valid topological order.
/// - `node_index` maps every node name to its index in `nodes`.
///
/// # Examples
/// ```
/// use torvyn_pipeline::{PipelineTopologyBuilder, NodeConfig};
/// use torvyn_types::ComponentRole;
///
/// let topology = PipelineTopologyBuilder::new("example")
///     .add_node("src", ComponentRole::Source, "file://source.wasm", NodeConfig::default())
///     .add_node("sink", ComponentRole::Sink, "file://sink.wasm", NodeConfig::default())
///     .add_edge("src", "output", "sink", "input")
///     .build()
///     .unwrap();
///
/// assert_eq!(topology.name(), "example");
/// assert_eq!(topology.nodes().len(), 2);
/// assert_eq!(topology.edges().len(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct PipelineTopology {
    /// Human-readable name of this pipeline.
    name: String,

    /// Description of this pipeline.
    description: String,

    /// Ordered list of nodes (component instances).
    nodes: Vec<TopologyNode>,

    /// Node name → index lookup. Populated during construction.
    node_index: HashMap<String, usize>,

    /// Stream edges connecting nodes.
    edges: Vec<TopologyEdge>,

    /// Topological execution order (indices into `nodes`).
    /// Computed during validation. Sources come first, sinks last.
    execution_order: Vec<usize>,
}

impl PipelineTopology {
    /// Returns the pipeline name.
    ///
    /// # COLD PATH
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the pipeline description.
    ///
    /// # COLD PATH
    #[inline]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns all nodes in this topology.
    ///
    /// # COLD PATH
    #[inline]
    pub fn nodes(&self) -> &[TopologyNode] {
        &self.nodes
    }

    /// Returns all edges in this topology.
    ///
    /// # COLD PATH
    #[inline]
    pub fn edges(&self) -> &[TopologyEdge] {
        &self.edges
    }

    /// Returns the topological execution order as indices into `nodes()`.
    ///
    /// Sources appear first, sinks appear last. Within a level, order is
    /// stable (insertion order).
    ///
    /// # COLD PATH
    #[inline]
    pub fn execution_order(&self) -> &[usize] {
        &self.execution_order
    }

    /// Look up a node index by name.
    ///
    /// # COLD PATH
    #[inline]
    pub fn node_index_by_name(&self, name: &str) -> Option<usize> {
        self.node_index.get(name).copied()
    }

    /// Returns the node at the given index.
    ///
    /// # COLD PATH
    #[inline]
    pub fn node(&self, index: usize) -> Option<&TopologyNode> {
        self.nodes.get(index)
    }

    /// Returns all source nodes (nodes with role Source).
    ///
    /// # COLD PATH
    pub fn source_nodes(&self) -> Vec<(usize, &TopologyNode)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.role == ComponentRole::Source)
            .collect()
    }

    /// Returns all sink nodes (nodes with role Sink).
    ///
    /// # COLD PATH
    pub fn sink_nodes(&self) -> Vec<(usize, &TopologyNode)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.role == ComponentRole::Sink)
            .collect()
    }

    /// Returns the count of nodes.
    ///
    /// # COLD PATH
    #[inline]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns the count of edges.
    ///
    /// # COLD PATH
    #[inline]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Returns edges outgoing from the given node index.
    ///
    /// # COLD PATH
    pub fn outgoing_edges(&self, node_idx: usize) -> Vec<(usize, &TopologyEdge)> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from_node == node_idx)
            .collect()
    }

    /// Returns edges incoming to the given node index.
    ///
    /// # COLD PATH
    pub fn incoming_edges(&self, node_idx: usize) -> Vec<(usize, &TopologyEdge)> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.to_node == node_idx)
            .collect()
    }

    /// Package-private constructor. Only the builder and validator produce
    /// this type. The validation module sets the `execution_order`.
    pub(crate) fn new_unchecked(
        name: String,
        description: String,
        nodes: Vec<TopologyNode>,
        node_index: HashMap<String, usize>,
        edges: Vec<TopologyEdge>,
        execution_order: Vec<usize>,
    ) -> Self {
        Self {
            name,
            description,
            nodes,
            node_index,
            edges,
            execution_order,
        }
    }
}

// ---------------------------------------------------------------------------
// TopologyNode
// ---------------------------------------------------------------------------

/// A component instance within the pipeline topology.
///
/// # Invariants
/// - `name` is unique within the pipeline.
/// - `component_ref` is a valid reference (file path, OCI URI, or name).
/// - `role` matches the component's exported WIT interface.
///
/// # Examples
/// ```
/// use torvyn_pipeline::TopologyNode;
/// use torvyn_types::ComponentRole;
///
/// let node = TopologyNode::new(
///     "my-source".into(),
///     "file://./source.wasm".into(),
///     "torvyn:streaming/source".into(),
///     ComponentRole::Source,
/// );
/// assert_eq!(node.name(), "my-source");
/// assert_eq!(node.role(), ComponentRole::Source);
/// ```
#[derive(Debug, Clone)]
pub struct TopologyNode {
    /// Unique name within the pipeline.
    name: String,

    /// Reference to the component artifact.
    component_ref: String,

    /// WIT interface this node implements.
    interface: String,

    /// The role of this component in the pipeline.
    role: ComponentRole,

    /// Per-node configuration overrides.
    config: NodeConfig,
}

impl TopologyNode {
    /// Create a new topology node.
    ///
    /// # COLD PATH
    pub fn new(
        name: String,
        component_ref: String,
        interface: String,
        role: ComponentRole,
    ) -> Self {
        Self {
            name,
            component_ref,
            interface,
            role,
            config: NodeConfig::default(),
        }
    }

    /// Create a new topology node with configuration.
    ///
    /// # COLD PATH
    pub fn with_config(
        name: String,
        component_ref: String,
        interface: String,
        role: ComponentRole,
        config: NodeConfig,
    ) -> Self {
        Self {
            name,
            component_ref,
            interface,
            role,
            config,
        }
    }

    /// Returns the node name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the component reference.
    #[inline]
    pub fn component_ref(&self) -> &str {
        &self.component_ref
    }

    /// Returns the WIT interface.
    #[inline]
    pub fn interface(&self) -> &str {
        &self.interface
    }

    /// Returns the component role.
    #[inline]
    pub fn role(&self) -> ComponentRole {
        self.role
    }

    /// Returns the per-node configuration.
    #[inline]
    pub fn config(&self) -> &NodeConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// TopologyEdge
// ---------------------------------------------------------------------------

/// A typed stream connection between two nodes in the topology.
///
/// # Invariants
/// - `from_node` and `to_node` are valid indices into the topology's node list.
/// - `from_node != to_node` (no self-loops).
/// - `from_port` and `to_port` are non-empty strings.
///
/// # Examples
/// ```
/// use torvyn_pipeline::TopologyEdge;
///
/// let edge = TopologyEdge::new(0, "output".into(), 1, "input".into());
/// assert_eq!(edge.from_node(), 0);
/// assert_eq!(edge.to_node(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct TopologyEdge {
    /// Index of the source node in the topology's node list.
    from_node: usize,

    /// Name of the output port on the source node.
    from_port: String,

    /// Index of the destination node in the topology's node list.
    to_node: usize,

    /// Name of the input port on the destination node.
    to_port: String,

    /// Per-edge configuration.
    edge_config: EdgeConfig,
}

impl TopologyEdge {
    /// Create a new topology edge with default configuration.
    ///
    /// # COLD PATH
    pub fn new(from_node: usize, from_port: String, to_node: usize, to_port: String) -> Self {
        Self {
            from_node,
            from_port,
            to_node,
            to_port,
            edge_config: EdgeConfig::default(),
        }
    }

    /// Create a new topology edge with configuration.
    ///
    /// # COLD PATH
    pub fn with_config(
        from_node: usize,
        from_port: String,
        to_node: usize,
        to_port: String,
        edge_config: EdgeConfig,
    ) -> Self {
        Self {
            from_node,
            from_port,
            to_node,
            to_port,
            edge_config,
        }
    }

    /// Returns the source node index.
    #[inline]
    pub fn from_node(&self) -> usize {
        self.from_node
    }

    /// Returns the output port name.
    #[inline]
    pub fn from_port(&self) -> &str {
        &self.from_port
    }

    /// Returns the destination node index.
    #[inline]
    pub fn to_node(&self) -> usize {
        self.to_node
    }

    /// Returns the input port name.
    #[inline]
    pub fn to_port(&self) -> &str {
        &self.to_port
    }

    /// Returns the per-edge configuration.
    #[inline]
    pub fn edge_config(&self) -> &EdgeConfig {
        &self.edge_config
    }
}

// ---------------------------------------------------------------------------
// NodeConfig
// ---------------------------------------------------------------------------

/// Per-node configuration overrides.
///
/// All fields are optional; `None` means "use the flow-level or global default."
///
/// Per Doc 02, Section 10.2: fuel budget, memory limit, timeout, priority,
/// and error policy are configurable per-node.
///
/// # Examples
/// ```
/// use torvyn_pipeline::NodeConfig;
///
/// let config = NodeConfig::default();
/// assert!(config.fuel_budget.is_none());
/// ```
#[derive(Debug, Clone, Default)]
pub struct NodeConfig {
    /// Fuel budget per invocation. Overrides flow/global default.
    pub fuel_budget: Option<u64>,

    /// Maximum linear memory in bytes. Overrides flow/global default.
    pub memory_limit: Option<usize>,

    /// Maximum wall-clock time per invocation.
    pub timeout: Option<Duration>,

    /// Scheduling priority (1–10, where 10 is highest).
    pub priority: Option<u8>,

    /// Error handling policy for this component.
    pub error_policy: Option<ErrorPolicy>,

    /// Per-component init configuration string (JSON).
    /// Passed to `lifecycle.init()`.
    pub init_config: Option<String>,
}

// ---------------------------------------------------------------------------
// EdgeConfig
// ---------------------------------------------------------------------------

/// Per-edge configuration for the stream connection between two nodes.
///
/// # Examples
/// ```
/// use torvyn_pipeline::EdgeConfig;
///
/// let config = EdgeConfig::default();
/// assert_eq!(config.queue_depth, None);
/// ```
#[derive(Debug, Clone, Default)]
pub struct EdgeConfig {
    /// Queue depth override for this stream.
    /// Default: uses flow-level or global default (64 per C02-2).
    pub queue_depth: Option<usize>,

    /// Backpressure policy override for this stream.
    pub backpressure_policy: Option<BackpressurePolicy>,
}

// ---------------------------------------------------------------------------
// ErrorPolicy
// ---------------------------------------------------------------------------

/// Per-component error handling policy.
///
/// Per Doc 02, Section 7.3.
///
/// # Examples
/// ```
/// use torvyn_pipeline::ErrorPolicy;
///
/// let policy = ErrorPolicy::FailFast;
/// assert!(matches!(policy, ErrorPolicy::FailFast));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ErrorPolicy {
    /// Terminate the flow on any component error (default).
    #[default]
    FailFast,
    /// Skip the faulting element and continue processing.
    SkipAndContinue,
    /// Retry the element up to `max_attempts` times.
    Retry {
        /// Maximum number of retry attempts.
        max_attempts: u32,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topology_node_accessors() {
        let node = TopologyNode::new(
            "src".into(),
            "file://source.wasm".into(),
            "torvyn:streaming/source".into(),
            ComponentRole::Source,
        );
        assert_eq!(node.name(), "src");
        assert_eq!(node.component_ref(), "file://source.wasm");
        assert_eq!(node.interface(), "torvyn:streaming/source");
        assert_eq!(node.role(), ComponentRole::Source);
    }

    #[test]
    fn test_topology_edge_accessors() {
        let edge = TopologyEdge::new(0, "output".into(), 1, "input".into());
        assert_eq!(edge.from_node(), 0);
        assert_eq!(edge.from_port(), "output");
        assert_eq!(edge.to_node(), 1);
        assert_eq!(edge.to_port(), "input");
    }

    #[test]
    fn test_node_config_default() {
        let config = NodeConfig::default();
        assert!(config.fuel_budget.is_none());
        assert!(config.memory_limit.is_none());
        assert!(config.timeout.is_none());
        assert!(config.priority.is_none());
        assert!(config.error_policy.is_none());
        assert!(config.init_config.is_none());
    }

    #[test]
    fn test_edge_config_default() {
        let config = EdgeConfig::default();
        assert!(config.queue_depth.is_none());
        assert!(config.backpressure_policy.is_none());
    }

    #[test]
    fn test_error_policy_default() {
        assert_eq!(ErrorPolicy::default(), ErrorPolicy::FailFast);
    }

    #[test]
    fn test_error_policy_retry() {
        let policy = ErrorPolicy::Retry { max_attempts: 3 };
        assert!(matches!(policy, ErrorPolicy::Retry { max_attempts: 3 }));
    }
}
