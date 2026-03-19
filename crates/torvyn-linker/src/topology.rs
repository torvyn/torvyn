//! Pipeline topology definition and validation.
//!
//! A `PipelineTopology` is a directed acyclic graph (DAG) of component nodes
//! connected by typed stream edges. The linker validates the topology and
//! resolves all imports before producing a `LinkedPipeline`.
//!
//! Per Doc 02 Section 5: pipelines must be DAGs. Cycles are rejected.
//! Fan-out and fan-in are supported with configurable limits.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use torvyn_types::{BackpressurePolicy, ComponentRole};

use crate::error::{LinkDiagnostic, LinkDiagnosticCategory, LinkReport};

/// Maximum fan-out per output port (configurable, default 16).
/// Per Doc 02 Section 5.3.
pub const DEFAULT_MAX_FAN_OUT: usize = 16;

/// Maximum fan-in per input port (configurable, default 16).
pub const DEFAULT_MAX_FAN_IN: usize = 16;

/// A pipeline topology: the graph structure of a flow.
///
/// # Invariants
/// - `name` is non-empty.
/// - Node names are unique within the topology.
/// - Edges reference only nodes that exist in `nodes`.
///
/// # Examples
/// ```
/// use torvyn_linker::topology::{PipelineTopology, TopologyNode, TopologyEdge};
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
/// let report = topo.validate();
/// assert!(report.is_ok());
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PipelineTopology {
    /// Pipeline name. Non-empty.
    pub name: String,

    /// Component nodes in the pipeline, keyed by node name.
    pub nodes: HashMap<String, TopologyNode>,

    /// Stream edges connecting nodes.
    pub edges: Vec<TopologyEdge>,

    /// Maximum fan-out per output port.
    pub max_fan_out: usize,

    /// Maximum fan-in per input port.
    pub max_fan_in: usize,
}

/// A single node (component instance) in the pipeline topology.
///
/// # Invariants
/// - `name` is non-empty and unique within the topology.
/// - `role` matches the component's exported WIT interface.
/// - `artifact_path` points to a valid Wasm component binary.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TopologyNode {
    /// Unique name of this node within the pipeline.
    pub name: String,

    /// The component's role (Source, Processor, Sink, Filter, Router).
    pub role: ComponentRole,

    /// Path to the component artifact (.wasm file or OCI reference).
    pub artifact_path: PathBuf,

    /// Per-component configuration string passed to `lifecycle.init()`.
    pub config: Option<String>,

    /// Capabilities granted to this component.
    pub capability_grants: Vec<CapabilityGrant>,
}

/// A capability granted to a component in the pipeline configuration.
///
/// # Examples
/// ```
/// use torvyn_linker::topology::CapabilityGrant;
///
/// let grant = CapabilityGrant {
///     name: "wasi-filesystem-read".into(),
///     detail: "/data/*".into(),
/// };
/// assert_eq!(grant.name, "wasi-filesystem-read");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CapabilityGrant {
    /// Capability name (e.g., "wasi-filesystem-read", "wasi-network-egress").
    pub name: String,
    /// Grant detail (e.g., allowed path pattern for filesystem, allowed hosts for network).
    pub detail: String,
}

/// A stream edge connecting an output port to an input port.
///
/// # Invariants
/// - `from_node` and `to_node` reference existing nodes.
/// - `queue_depth` > 0.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TopologyEdge {
    /// Source node name.
    pub from_node: String,
    /// Source output port name.
    pub from_port: String,
    /// Destination node name.
    pub to_node: String,
    /// Destination input port name.
    pub to_port: String,
    /// Queue depth for this stream connection. Per C02-2: default is 64.
    pub queue_depth: u32,
    /// Backpressure policy for this stream connection.
    pub backpressure_policy: BackpressurePolicy,
}

impl PipelineTopology {
    /// Create a new empty topology with the given pipeline name.
    ///
    /// # Preconditions
    /// - `name` must be non-empty.
    ///
    /// # COLD PATH
    pub fn new(name: String) -> Self {
        Self {
            name,
            nodes: HashMap::new(),
            edges: Vec::new(),
            max_fan_out: DEFAULT_MAX_FAN_OUT,
            max_fan_in: DEFAULT_MAX_FAN_IN,
        }
    }

    /// Add a node to the topology.
    ///
    /// # COLD PATH
    pub fn add_node(&mut self, node: TopologyNode) {
        self.nodes.insert(node.name.clone(), node);
    }

    /// Add an edge to the topology.
    ///
    /// # COLD PATH
    pub fn add_edge(&mut self, edge: TopologyEdge) {
        self.edges.push(edge);
    }

    /// Validate the topology structure.
    ///
    /// Checks:
    /// 1. Pipeline name is non-empty.
    /// 2. At least one node exists.
    /// 3. All edge endpoints reference existing nodes.
    /// 4. No orphan nodes (every node has at least one connection).
    /// 5. Source nodes have no incoming edges.
    /// 6. Sink nodes have no outgoing edges.
    /// 7. No cycles (topological sort succeeds).
    /// 8. Fan-out and fan-in limits are respected.
    ///
    /// Reports ALL errors, not just the first one.
    ///
    /// # Postconditions
    /// - Returns a `LinkReport` with all structural diagnostics.
    ///
    /// # COLD PATH
    pub fn validate(&self) -> LinkReport {
        let mut report = LinkReport::new();

        // 1. Pipeline name
        if self.name.is_empty() {
            report.push_error(LinkDiagnostic {
                category: LinkDiagnosticCategory::TopologyError,
                message: "Pipeline name must be non-empty.".into(),
                component: None,
                related_component: None,
                interface_name: None,
            });
        }

        // 2. At least one node
        if self.nodes.is_empty() {
            report.push_error(LinkDiagnostic {
                category: LinkDiagnosticCategory::TopologyError,
                message: "Pipeline must contain at least one component node.".into(),
                component: None,
                related_component: None,
                interface_name: None,
            });
            return report;
        }

        // 3. Edge endpoints reference existing nodes
        for edge in &self.edges {
            if !self.nodes.contains_key(&edge.from_node) {
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::TopologyError,
                    message: format!(
                        "Edge references unknown source node '{}'. \
                         Check that all edge 'from' values match a defined component name.",
                        edge.from_node
                    ),
                    component: Some(edge.from_node.clone()),
                    related_component: Some(edge.to_node.clone()),
                    interface_name: None,
                });
            }
            if !self.nodes.contains_key(&edge.to_node) {
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::TopologyError,
                    message: format!(
                        "Edge references unknown destination node '{}'. \
                         Check that all edge 'to' values match a defined component name.",
                        edge.to_node
                    ),
                    component: Some(edge.to_node.clone()),
                    related_component: Some(edge.from_node.clone()),
                    interface_name: None,
                });
            }
        }

        // 4. Connectivity: every node participates in at least one edge
        let connected: HashSet<&str> = self
            .edges
            .iter()
            .flat_map(|e| [e.from_node.as_str(), e.to_node.as_str()])
            .collect();

        for name in self.nodes.keys() {
            if !connected.contains(name.as_str()) {
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::TopologyError,
                    message: format!(
                        "Component '{}' is disconnected (no edges). \
                         Connect it to the pipeline or remove it.",
                        name
                    ),
                    component: Some(name.clone()),
                    related_component: None,
                    interface_name: None,
                });
            }
        }

        // 5 & 6. Role constraints
        let incoming = self.compute_incoming_counts();
        let outgoing = self.compute_outgoing_counts();

        for (name, node) in &self.nodes {
            let in_count = incoming.get(name.as_str()).copied().unwrap_or(0);
            let out_count = outgoing.get(name.as_str()).copied().unwrap_or(0);

            if node.role == ComponentRole::Source && in_count > 0 {
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::RoleViolation,
                    message: format!(
                        "Source component '{}' has {} incoming edge(s). \
                         Sources produce data and must not have incoming connections.",
                        name, in_count
                    ),
                    component: Some(name.clone()),
                    related_component: None,
                    interface_name: None,
                });
            }

            if node.role == ComponentRole::Sink && out_count > 0 {
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::RoleViolation,
                    message: format!(
                        "Sink component '{}' has {} outgoing edge(s). \
                         Sinks consume data and must not have outgoing connections.",
                        name, out_count
                    ),
                    component: Some(name.clone()),
                    related_component: None,
                    interface_name: None,
                });
            }
        }

        // 7. Cycle detection
        if let Some(cycle) = self.detect_cycle() {
            report.push_error(LinkDiagnostic {
                category: LinkDiagnosticCategory::CyclicDependency,
                message: format!(
                    "Pipeline topology contains a cycle: {}. \
                     Torvyn pipelines must be directed acyclic graphs (DAGs). \
                     Remove the cycle by restructuring the pipeline.",
                    cycle.join(" → ")
                ),
                component: cycle.first().cloned(),
                related_component: None,
                interface_name: None,
            });
        }

        // 8. Fan-out / fan-in limits
        self.check_fan_limits(&outgoing, &incoming, &mut report);

        report
    }

    /// Compute a topological ordering of node names. Returns `None` if a cycle exists.
    ///
    /// Uses Kahn's algorithm.
    ///
    /// # COLD PATH
    pub fn topological_order(&self) -> Option<Vec<String>> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        for name in self.nodes.keys() {
            in_degree.insert(name.as_str(), 0);
        }
        for edge in &self.edges {
            if let Some(count) = in_degree.get_mut(edge.to_node.as_str()) {
                *count += 1;
            }
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &count)| count == 0)
            .map(|(&name, _)| name)
            .collect();

        let mut order = Vec::with_capacity(self.nodes.len());

        while let Some(node) = queue.pop_front() {
            order.push(node.to_string());

            for edge in &self.edges {
                if edge.from_node == node {
                    if let Some(count) = in_degree.get_mut(edge.to_node.as_str()) {
                        *count -= 1;
                        if *count == 0 {
                            queue.push_back(edge.to_node.as_str());
                        }
                    }
                }
            }
        }

        if order.len() == self.nodes.len() {
            Some(order)
        } else {
            None
        }
    }

    /// Detect a cycle using DFS. Returns the cycle path if found, or `None`.
    ///
    /// # COLD PATH
    fn detect_cycle(&self) -> Option<Vec<String>> {
        let adj = self.adjacency_list();
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();
        let mut path = Vec::new();

        for name in self.nodes.keys() {
            if !visited.contains(name.as_str()) {
                if let Some(cycle) =
                    Self::dfs_cycle(name.as_str(), &adj, &mut visited, &mut in_stack, &mut path)
                {
                    return Some(cycle);
                }
            }
        }
        None
    }

    /// DFS helper for cycle detection.
    fn dfs_cycle<'a>(
        node: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        in_stack: &mut HashSet<&'a str>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        visited.insert(node);
        in_stack.insert(node);
        path.push(node);

        if let Some(neighbors) = adj.get(node) {
            for &neighbor in neighbors {
                if !visited.contains(neighbor) {
                    if let Some(cycle) = Self::dfs_cycle(neighbor, adj, visited, in_stack, path) {
                        return Some(cycle);
                    }
                } else if in_stack.contains(neighbor) {
                    // Found cycle — extract cycle from path
                    let start = path.iter().position(|&n| n == neighbor).unwrap_or(0);
                    let mut cycle: Vec<String> =
                        path[start..].iter().map(|s| s.to_string()).collect();
                    cycle.push(neighbor.to_string());
                    return Some(cycle);
                }
            }
        }

        in_stack.remove(node);
        path.pop();
        None
    }

    /// Build an adjacency list from edges.
    fn adjacency_list(&self) -> HashMap<&str, Vec<&str>> {
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for edge in &self.edges {
            adj.entry(edge.from_node.as_str())
                .or_default()
                .push(edge.to_node.as_str());
        }
        adj
    }

    /// Compute incoming edge counts per node.
    fn compute_incoming_counts(&self) -> HashMap<&str, usize> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for edge in &self.edges {
            *counts.entry(edge.to_node.as_str()).or_insert(0) += 1;
        }
        counts
    }

    /// Compute outgoing edge counts per node.
    fn compute_outgoing_counts(&self) -> HashMap<&str, usize> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for edge in &self.edges {
            *counts.entry(edge.from_node.as_str()).or_insert(0) += 1;
        }
        counts
    }

    /// Check fan-out and fan-in limits.
    fn check_fan_limits(
        &self,
        _outgoing: &HashMap<&str, usize>,
        _incoming: &HashMap<&str, usize>,
        report: &mut LinkReport,
    ) {
        // Per-port fan-out
        let mut port_fan_out: HashMap<(&str, &str), usize> = HashMap::new();
        for edge in &self.edges {
            *port_fan_out
                .entry((edge.from_node.as_str(), edge.from_port.as_str()))
                .or_insert(0) += 1;
        }
        for ((node, port), count) in &port_fan_out {
            if *count > self.max_fan_out {
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::TopologyError,
                    message: format!(
                        "Output port '{port}' of component '{node}' has fan-out of {count}, \
                         which exceeds the limit of {}. \
                         Reduce the number of downstream connections or increase max_fan_out.",
                        self.max_fan_out
                    ),
                    component: Some(node.to_string()),
                    related_component: None,
                    interface_name: Some(port.to_string()),
                });
            }
        }

        // Per-port fan-in
        let mut port_fan_in: HashMap<(&str, &str), usize> = HashMap::new();
        for edge in &self.edges {
            *port_fan_in
                .entry((edge.to_node.as_str(), edge.to_port.as_str()))
                .or_insert(0) += 1;
        }
        for ((node, port), count) in &port_fan_in {
            if *count > self.max_fan_in {
                report.push_error(LinkDiagnostic {
                    category: LinkDiagnosticCategory::TopologyError,
                    message: format!(
                        "Input port '{port}' of component '{node}' has fan-in of {count}, \
                         which exceeds the limit of {}. \
                         Reduce the number of upstream connections or increase max_fan_in.",
                        self.max_fan_in
                    ),
                    component: Some(node.to_string()),
                    related_component: None,
                    interface_name: Some(port.to_string()),
                });
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

    #[test]
    fn test_valid_source_sink_topology() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "snk"));

        let report = topo.validate();
        assert!(
            report.is_ok(),
            "Expected valid topology, got: {}",
            report.format_all()
        );
    }

    #[test]
    fn test_valid_three_stage_topology() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(processor_node("proc"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "proc"));
        topo.add_edge(edge("proc", "snk"));

        let report = topo.validate();
        assert!(report.is_ok(), "{}", report.format_all());
    }

    #[test]
    fn test_valid_fan_out_topology() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk1"));
        topo.add_node(sink_node("snk2"));
        topo.add_edge(edge("src", "snk1"));
        topo.add_edge(edge("src", "snk2"));

        let report = topo.validate();
        assert!(report.is_ok(), "{}", report.format_all());
    }

    #[test]
    fn test_empty_pipeline_rejected() {
        let topo = PipelineTopology::new("test".into());
        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("at least one")));
    }

    #[test]
    fn test_empty_name_rejected() {
        let mut topo = PipelineTopology::new(String::new());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "snk"));

        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| e.message.contains("name")));
    }

    #[test]
    fn test_unknown_edge_endpoint_rejected() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_edge(edge("src", "nonexistent"));

        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("nonexistent")));
    }

    #[test]
    fn test_disconnected_node_rejected() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("snk"));
        topo.add_node(processor_node("orphan"));
        topo.add_edge(edge("src", "snk"));

        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| e.message.contains("orphan")));
    }

    #[test]
    fn test_source_with_incoming_rejected() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(processor_node("proc"));
        // proc → src is invalid: source has incoming
        topo.add_edge(edge("proc", "src"));

        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| {
            e.category == LinkDiagnosticCategory::RoleViolation && e.message.contains("Source")
        }));
    }

    #[test]
    fn test_sink_with_outgoing_rejected() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(sink_node("snk"));
        topo.add_node(processor_node("proc"));
        // snk → proc is invalid: sink has outgoing
        topo.add_edge(edge("snk", "proc"));

        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| {
            e.category == LinkDiagnosticCategory::RoleViolation && e.message.contains("Sink")
        }));
    }

    #[test]
    fn test_cycle_detected() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(processor_node("a"));
        topo.add_node(processor_node("b"));
        topo.add_edge(edge("a", "b"));
        topo.add_edge(edge("b", "a"));

        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| { e.category == LinkDiagnosticCategory::CyclicDependency }));
    }

    #[test]
    fn test_topological_order_success() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(source_node("src"));
        topo.add_node(processor_node("proc"));
        topo.add_node(sink_node("snk"));
        topo.add_edge(edge("src", "proc"));
        topo.add_edge(edge("proc", "snk"));

        let order = topo.topological_order().unwrap();
        assert_eq!(order.len(), 3);
        let src_pos = order.iter().position(|n| n == "src").unwrap();
        let proc_pos = order.iter().position(|n| n == "proc").unwrap();
        let snk_pos = order.iter().position(|n| n == "snk").unwrap();
        assert!(src_pos < proc_pos);
        assert!(proc_pos < snk_pos);
    }

    #[test]
    fn test_topological_order_cycle_returns_none() {
        let mut topo = PipelineTopology::new("test".into());
        topo.add_node(processor_node("a"));
        topo.add_node(processor_node("b"));
        topo.add_edge(edge("a", "b"));
        topo.add_edge(edge("b", "a"));

        assert!(topo.topological_order().is_none());
    }

    #[test]
    fn test_fan_out_limit_exceeded() {
        let mut topo = PipelineTopology::new("test".into());
        topo.max_fan_out = 2;
        topo.add_node(source_node("src"));
        topo.add_node(sink_node("s1"));
        topo.add_node(sink_node("s2"));
        topo.add_node(sink_node("s3"));
        topo.add_edge(edge("src", "s1"));
        topo.add_edge(edge("src", "s2"));
        topo.add_edge(edge("src", "s3")); // exceeds limit of 2

        let report = topo.validate();
        assert!(!report.is_ok());
        assert!(report.errors.iter().any(|e| e.message.contains("fan-out")));
    }
}
