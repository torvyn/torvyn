//! Topology validation: acyclicity, connectedness, role consistency, fan limits.
//!
//! Per Doc 02, Section 5.3: before execution, the topology is validated for:
//! 1. **Acyclicity** — topological sort; report cycle if sort fails.
//! 2. **Connectedness** — every node reachable from at least one source.
//! 3. **Role consistency** — sources have no incoming; sinks have no outgoing.
//! 4. **Fan limits** — configurable max fan-out/fan-in per port.

use std::collections::{HashMap, VecDeque};

use torvyn_types::ComponentRole;

use crate::error::{PipelineError, ValidationReport};
use crate::topology::{TopologyEdge, TopologyNode};

/// Default maximum fan-out per output port.
///
/// Per Doc 02, Section 5.3: configurable, default 16.
pub const DEFAULT_MAX_FAN_OUT: usize = 16;

/// Default maximum fan-in per input port.
pub const DEFAULT_MAX_FAN_IN: usize = 16;

/// Validate a topology, populating the report with any errors found.
///
/// Returns the topological execution order if validation succeeds,
/// or an empty vec if it fails.
///
/// # COLD PATH — called once per topology during pipeline construction.
///
/// # Preconditions
/// - `nodes` and `edges` have been constructed (no unknown node references,
///   no self-loops — the builder checks these).
///
/// # Postconditions
/// - If `report.is_ok()`, the returned `Vec<usize>` is a valid topological
///   order covering all nodes.
/// - If `!report.is_ok()`, the returned vec is empty.
pub fn validate_topology(
    flow_name: &str,
    nodes: &[TopologyNode],
    edges: &[TopologyEdge],
    report: &mut ValidationReport,
) -> Vec<usize> {
    if nodes.is_empty() {
        report.push(PipelineError::EmptyTopology {
            flow_name: flow_name.to_owned(),
        });
        return Vec::new();
    }

    // 1. Check that at least one source and one sink exist
    check_source_and_sink(flow_name, nodes, report);

    // 2. Check role consistency (sources have no incoming, sinks have no outgoing)
    check_role_consistency(flow_name, nodes, edges, report);

    // 3. Check fan-out/fan-in limits
    check_fan_limits(flow_name, nodes, edges, report);

    // 4. Topological sort (detects cycles)
    let topo_order = topological_sort(flow_name, nodes, edges, report);

    // 5. Check connectedness (every node reachable from a source)
    if !topo_order.is_empty() {
        check_connectedness(flow_name, nodes, edges, report);
    }

    topo_order
}

/// Check that at least one source and one sink exist.
///
/// # COLD PATH
fn check_source_and_sink(flow_name: &str, nodes: &[TopologyNode], report: &mut ValidationReport) {
    let has_source = nodes.iter().any(|n| n.role() == ComponentRole::Source);
    let has_sink = nodes.iter().any(|n| n.role() == ComponentRole::Sink);

    if !has_source {
        report.push(PipelineError::NoSourceNodes {
            flow_name: flow_name.to_owned(),
        });
    }
    if !has_sink {
        report.push(PipelineError::NoSinkNodes {
            flow_name: flow_name.to_owned(),
        });
    }
}

/// Check role consistency: sources have no incoming, sinks have no outgoing,
/// processors/filters/routers have both.
///
/// # COLD PATH
fn check_role_consistency(
    flow_name: &str,
    nodes: &[TopologyNode],
    edges: &[TopologyEdge],
    report: &mut ValidationReport,
) {
    for (idx, node) in nodes.iter().enumerate() {
        let has_incoming = edges.iter().any(|e| e.to_node() == idx);
        let has_outgoing = edges.iter().any(|e| e.from_node() == idx);

        match node.role() {
            ComponentRole::Source => {
                if has_incoming {
                    report.push(PipelineError::SourceHasIncoming {
                        flow_name: flow_name.to_owned(),
                        node_name: node.name().to_owned(),
                    });
                }
            }
            ComponentRole::Sink => {
                if has_outgoing {
                    report.push(PipelineError::SinkHasOutgoing {
                        flow_name: flow_name.to_owned(),
                        node_name: node.name().to_owned(),
                    });
                }
            }
            ComponentRole::Processor | ComponentRole::Filter | ComponentRole::Router => {
                if !has_incoming || !has_outgoing {
                    report.push(PipelineError::ProcessorMissingEdges {
                        flow_name: flow_name.to_owned(),
                        node_name: node.name().to_owned(),
                        role: format!("{}", node.role()),
                        has_incoming,
                        has_outgoing,
                    });
                }
            }
        }
    }
}

/// Check fan-out and fan-in limits per port.
///
/// # COLD PATH
fn check_fan_limits(
    flow_name: &str,
    nodes: &[TopologyNode],
    edges: &[TopologyEdge],
    report: &mut ValidationReport,
) {
    // Count outgoing edges per (node, port)
    let mut out_counts: HashMap<(usize, &str), usize> = HashMap::new();
    for edge in edges {
        *out_counts
            .entry((edge.from_node(), edge.from_port()))
            .or_insert(0) += 1;
    }

    for ((node_idx, port), count) in &out_counts {
        if *count > DEFAULT_MAX_FAN_OUT {
            report.push(PipelineError::FanLimitExceeded {
                flow_name: flow_name.to_owned(),
                node_name: nodes[*node_idx].name().to_owned(),
                port: (*port).to_string(),
                count: *count,
                limit: DEFAULT_MAX_FAN_OUT,
                direction: "fan-out",
            });
        }
    }

    // Count incoming edges per (node, port)
    let mut in_counts: HashMap<(usize, &str), usize> = HashMap::new();
    for edge in edges {
        *in_counts
            .entry((edge.to_node(), edge.to_port()))
            .or_insert(0) += 1;
    }

    for ((node_idx, port), count) in &in_counts {
        if *count > DEFAULT_MAX_FAN_IN {
            report.push(PipelineError::FanLimitExceeded {
                flow_name: flow_name.to_owned(),
                node_name: nodes[*node_idx].name().to_owned(),
                port: (*port).to_string(),
                count: *count,
                limit: DEFAULT_MAX_FAN_IN,
                direction: "fan-in",
            });
        }
    }
}

/// Kahn's algorithm for topological sort.
///
/// Returns the topological order if the graph is a DAG, or reports a cycle.
///
/// # COLD PATH
fn topological_sort(
    flow_name: &str,
    nodes: &[TopologyNode],
    edges: &[TopologyEdge],
    report: &mut ValidationReport,
) -> Vec<usize> {
    let n = nodes.len();

    // Build adjacency list and in-degree array
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree: Vec<usize> = vec![0; n];

    for edge in edges {
        adj[edge.from_node()].push(edge.to_node());
        in_degree[edge.to_node()] += 1;
    }

    // Seed queue with zero-in-degree nodes
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    let mut order: Vec<usize> = Vec::with_capacity(n);

    while let Some(node) = queue.pop_front() {
        order.push(node);
        for &neighbor in &adj[node] {
            in_degree[neighbor] -= 1;
            if in_degree[neighbor] == 0 {
                queue.push_back(neighbor);
            }
        }
    }

    if order.len() != n {
        // Cycle detected — find nodes still with nonzero in-degree
        let cycle_nodes: Vec<String> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, &deg)| deg > 0)
            .map(|(i, _)| nodes[i].name().to_owned())
            .collect();

        // Build a more informative cycle representation
        let mut cycle = find_cycle(nodes, edges, &in_degree);
        if cycle.is_empty() {
            cycle = cycle_nodes;
        }

        report.push(PipelineError::CycleDetected {
            flow_name: flow_name.to_owned(),
            cycle,
        });
        return Vec::new();
    }

    order
}

/// Find one cycle in the graph for error reporting.
///
/// Uses DFS with color marking to find a back edge and trace the cycle.
///
/// # COLD PATH — only called when a cycle is detected.
fn find_cycle(nodes: &[TopologyNode], edges: &[TopologyEdge], in_degree: &[usize]) -> Vec<String> {
    let n = nodes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in edges {
        adj[edge.from_node()].push(edge.to_node());
    }

    // Deduplicate adjacency lists for cycle detection
    for list in &mut adj {
        list.sort_unstable();
        list.dedup();
    }

    // 0 = white (unvisited), 1 = gray (in stack), 2 = black (finished)
    let mut color = vec![0u8; n];
    let mut parent = vec![usize::MAX; n];

    for start in 0..n {
        if in_degree[start] == 0 || color[start] != 0 {
            continue;
        }
        // DFS from this node
        let mut stack = vec![(start, 0usize)]; // (node, adj_index)
        color[start] = 1;

        while let Some((node, adj_idx)) = stack.last_mut() {
            if *adj_idx >= adj[*node].len() {
                color[*node] = 2;
                stack.pop();
                continue;
            }
            let next = adj[*node][*adj_idx];
            *adj_idx += 1;

            if color[next] == 1 {
                // Back edge found — trace cycle
                let mut cycle = vec![nodes[next].name().to_owned()];
                let mut cur = *node;
                while cur != next {
                    cycle.push(nodes[cur].name().to_owned());
                    // Walk parent chain
                    cur = parent[cur];
                    if cur == usize::MAX {
                        break;
                    }
                }
                cycle.push(nodes[next].name().to_owned());
                cycle.reverse();
                return cycle;
            } else if color[next] == 0 {
                color[next] = 1;
                parent[next] = *node;
                stack.push((next, 0));
            }
        }
    }

    Vec::new()
}

/// Check that every node is reachable from at least one source.
///
/// Uses BFS from all source nodes.
///
/// # COLD PATH
fn check_connectedness(
    flow_name: &str,
    nodes: &[TopologyNode],
    edges: &[TopologyEdge],
    report: &mut ValidationReport,
) {
    let n = nodes.len();

    // Build adjacency list
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in edges {
        adj[edge.from_node()].push(edge.to_node());
    }

    let mut visited = vec![false; n];
    let mut queue: VecDeque<usize> = VecDeque::new();

    // Seed with all source nodes
    for (i, node) in nodes.iter().enumerate() {
        if node.role() == ComponentRole::Source {
            visited[i] = true;
            queue.push_back(i);
        }
    }

    // BFS
    while let Some(node) = queue.pop_front() {
        for &neighbor in &adj[node] {
            if !visited[neighbor] {
                visited[neighbor] = true;
                queue.push_back(neighbor);
            }
        }
    }

    // Report any unvisited nodes
    for (i, node) in nodes.iter().enumerate() {
        if !visited[i] {
            report.push(PipelineError::DisconnectedNode {
                flow_name: flow_name.to_owned(),
                node_name: node.name().to_owned(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    fn make_node(name: &str, role: ComponentRole) -> TopologyNode {
        TopologyNode::new(
            name.to_owned(),
            format!("file://{name}.wasm"),
            "torvyn:streaming/processor".to_owned(),
            role,
        )
    }

    #[test]
    fn test_valid_linear_pipeline() {
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("p", ComponentRole::Processor),
            make_node("k", ComponentRole::Sink),
        ];
        let edges = vec![
            TopologyEdge::new(0, "out".into(), 1, "in".into()),
            TopologyEdge::new(1, "out".into(), 2, "in".into()),
        ];
        let mut report = ValidationReport::new("test");
        let order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(report.is_ok(), "errors: {:?}", report.errors());
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn test_valid_fan_out() {
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("k1", ComponentRole::Sink),
            make_node("k2", ComponentRole::Sink),
        ];
        let edges = vec![
            TopologyEdge::new(0, "out".into(), 1, "in".into()),
            TopologyEdge::new(0, "out".into(), 2, "in".into()),
        ];
        let mut report = ValidationReport::new("test");
        let order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(report.is_ok());
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], 0); // source first
    }

    #[test]
    fn test_valid_fan_in() {
        let nodes = vec![
            make_node("s1", ComponentRole::Source),
            make_node("s2", ComponentRole::Source),
            make_node("k", ComponentRole::Sink),
        ];
        let edges = vec![
            TopologyEdge::new(0, "out".into(), 2, "in".into()),
            TopologyEdge::new(1, "out".into(), 2, "in".into()),
        ];
        let mut report = ValidationReport::new("test");
        let order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(report.is_ok());
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn test_detect_cycle() {
        let nodes = vec![
            make_node("a", ComponentRole::Processor),
            make_node("b", ComponentRole::Processor),
            make_node("s", ComponentRole::Source),
            make_node("k", ComponentRole::Sink),
        ];
        let edges = vec![
            TopologyEdge::new(2, "out".into(), 0, "in".into()),
            TopologyEdge::new(0, "out".into(), 1, "in".into()),
            TopologyEdge::new(1, "out".into(), 0, "in2".into()), // cycle: a -> b -> a
            TopologyEdge::new(1, "out2".into(), 3, "in".into()),
        ];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report
            .errors()
            .iter()
            .any(|e| matches!(e, PipelineError::CycleDetected { .. })));
    }

    #[test]
    fn test_detect_disconnected_node() {
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("k", ComponentRole::Sink),
            make_node("orphan", ComponentRole::Processor),
        ];
        let edges = vec![TopologyEdge::new(0, "out".into(), 1, "in".into())];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report.errors().iter().any(|e| matches!(
            e,
            PipelineError::DisconnectedNode { node_name, .. } if node_name == "orphan"
        )));
    }

    #[test]
    fn test_source_with_incoming_edge_rejected() {
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("k", ComponentRole::Sink),
        ];
        let edges = vec![
            TopologyEdge::new(0, "out".into(), 1, "in".into()),
            TopologyEdge::new(1, "out".into(), 0, "in".into()), // incoming to source
        ];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report
            .errors()
            .iter()
            .any(|e| matches!(e, PipelineError::SourceHasIncoming { .. })));
    }

    #[test]
    fn test_sink_with_outgoing_edge_rejected() {
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("k", ComponentRole::Sink),
        ];
        let edges = vec![
            TopologyEdge::new(0, "out".into(), 1, "in".into()),
            TopologyEdge::new(1, "out".into(), 0, "in".into()), // outgoing from sink
        ];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report
            .errors()
            .iter()
            .any(|e| matches!(e, PipelineError::SinkHasOutgoing { .. })));
    }

    #[test]
    fn test_processor_missing_incoming() {
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("p", ComponentRole::Processor),
            make_node("k", ComponentRole::Sink),
        ];
        // p has no incoming edge
        let edges = vec![
            TopologyEdge::new(0, "out".into(), 2, "in".into()),
            TopologyEdge::new(1, "out".into(), 2, "in2".into()),
        ];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report.errors().iter().any(|e| matches!(
            e,
            PipelineError::ProcessorMissingEdges { node_name, has_incoming: false, .. } if node_name == "p"
        )));
    }

    #[test]
    fn test_no_sources_detected() {
        let nodes = vec![
            make_node("p", ComponentRole::Processor),
            make_node("k", ComponentRole::Sink),
        ];
        let edges = vec![TopologyEdge::new(0, "out".into(), 1, "in".into())];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report
            .errors()
            .iter()
            .any(|e| matches!(e, PipelineError::NoSourceNodes { .. })));
    }

    #[test]
    fn test_no_sinks_detected() {
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("p", ComponentRole::Processor),
        ];
        let edges = vec![TopologyEdge::new(0, "out".into(), 1, "in".into())];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report
            .errors()
            .iter()
            .any(|e| matches!(e, PipelineError::NoSinkNodes { .. })));
    }

    #[test]
    fn test_empty_topology_rejected() {
        let nodes: Vec<TopologyNode> = vec![];
        let edges: Vec<TopologyEdge> = vec![];
        let mut report = ValidationReport::new("test");
        let _order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(!report.is_ok());
        assert!(report
            .errors()
            .iter()
            .any(|e| matches!(e, PipelineError::EmptyTopology { .. })));
    }

    #[test]
    fn test_diamond_topology() {
        // s → p1 → k
        //   ↘ p2 ↗
        let nodes = vec![
            make_node("s", ComponentRole::Source),
            make_node("p1", ComponentRole::Processor),
            make_node("p2", ComponentRole::Processor),
            make_node("k", ComponentRole::Sink),
        ];
        let edges = vec![
            TopologyEdge::new(0, "out".into(), 1, "in".into()),
            TopologyEdge::new(0, "out".into(), 2, "in".into()),
            TopologyEdge::new(1, "out".into(), 3, "in".into()),
            TopologyEdge::new(2, "out".into(), 3, "in".into()),
        ];
        let mut report = ValidationReport::new("test");
        let order = validate_topology("test", &nodes, &edges, &mut report);

        assert!(report.is_ok(), "errors: {:?}", report.errors());
        assert_eq!(order.len(), 4);
        assert_eq!(order[0], 0); // source first
        assert_eq!(order[3], 3); // sink last
    }
}
