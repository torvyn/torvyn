//! Flow topology definition and validation.
//!
//! A [`FlowTopology`] is a directed acyclic graph (DAG) of stages connected
//! by streams. Validation ensures no cycles, all stages are connected,
//! sources have no inputs, and sinks have no outputs.

use std::collections::VecDeque;
use std::fmt;

use torvyn_types::{ComponentId, ComponentRole};

use crate::config::StreamConfig;
use crate::error::FlowCreationError;

// ---------------------------------------------------------------------------
// StageDefinition
// ---------------------------------------------------------------------------

/// A single stage in a flow pipeline.
///
/// Each stage is a component instance with a defined role. The `component_id`
/// refers to an already-instantiated component provided by the host runtime.
#[derive(Clone, Debug)]
pub struct StageDefinition {
    /// The component instance for this stage.
    pub component_id: ComponentId,
    /// The role this component fulfills in the pipeline.
    /// Per C04-4: uses `ComponentRole` (not `StageRole`).
    pub role: ComponentRole,
    /// Fuel budget for this component, per C04-3.
    /// `None` means use the global default.
    pub fuel_budget: Option<u64>,
    /// Per-component configuration string passed to `lifecycle.init()`.
    pub config: String,
}

// ---------------------------------------------------------------------------
// StreamConnection
// ---------------------------------------------------------------------------

/// A directed connection between two stages.
///
/// Defines which stage produces to which stage, with optional per-stream
/// configuration overrides.
///
/// # Invariants
/// - `from_stage` and `to_stage` are valid indices into `FlowTopology::stages`.
/// - `from_stage != to_stage` (no self-loops).
#[derive(Clone, Debug)]
pub struct StreamConnection {
    /// Index of the producing stage in `FlowTopology::stages`.
    pub from_stage: usize,
    /// Index of the consuming stage in `FlowTopology::stages`.
    pub to_stage: usize,
    /// Per-stream configuration overrides.
    pub config: StreamConfig,
}

// ---------------------------------------------------------------------------
// FlowTopology
// ---------------------------------------------------------------------------

/// The directed acyclic graph defining a flow's pipeline structure.
///
/// # Invariants
/// - Must be a valid DAG (no cycles).
/// - Every stage must be reachable from at least one source.
/// - Sources have no incoming connections.
/// - Sinks have no outgoing connections.
/// - At least one source and one sink must exist.
#[derive(Clone, Debug)]
pub struct FlowTopology {
    /// Ordered list of stages (components) in the pipeline.
    pub stages: Vec<StageDefinition>,
    /// Directed connections between stages.
    pub connections: Vec<StreamConnection>,
}

impl FlowTopology {
    /// Create an empty topology (for testing defaults).
    ///
    /// # COLD PATH
    pub fn empty() -> Self {
        Self {
            stages: Vec::new(),
            connections: Vec::new(),
        }
    }

    /// Validate the topology, returning an error if any invariant is violated.
    ///
    /// Checks performed:
    /// 1. At least one source and one sink.
    /// 2. All connection indices are within bounds.
    /// 3. No self-loops.
    /// 4. Sources have no incoming connections.
    /// 5. Sinks have no outgoing connections.
    /// 6. No cycles (topological sort).
    /// 7. All stages are reachable from a source.
    ///
    /// # COLD PATH — called once during flow creation.
    ///
    /// # Errors
    /// Returns [`FlowCreationError::InvalidTopology`] with a descriptive message.
    pub fn validate(&self) -> Result<(), FlowCreationError> {
        let n = self.stages.len();
        if n == 0 {
            return Err(FlowCreationError::InvalidTopology(
                "topology has no stages".into(),
            ));
        }

        // Check at least one source and one sink.
        let has_source = self.stages.iter().any(|s| s.role == ComponentRole::Source);
        let has_sink = self.stages.iter().any(|s| s.role == ComponentRole::Sink);
        if !has_source {
            return Err(FlowCreationError::InvalidTopology(
                "topology has no Source stage".into(),
            ));
        }
        if !has_sink {
            return Err(FlowCreationError::InvalidTopology(
                "topology has no Sink stage".into(),
            ));
        }

        // Build adjacency and in-degree for cycle detection.
        let mut in_degree = vec![0u32; n];
        let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_edges: Vec<Vec<usize>> = vec![Vec::new(); n];

        for (i, conn) in self.connections.iter().enumerate() {
            if conn.from_stage >= n || conn.to_stage >= n {
                return Err(FlowCreationError::InvalidTopology(format!(
                    "connection {i} references out-of-bounds stage index \
                     (from={}, to={}, stages={})",
                    conn.from_stage, conn.to_stage, n
                )));
            }
            if conn.from_stage == conn.to_stage {
                return Err(FlowCreationError::InvalidTopology(format!(
                    "connection {i} is a self-loop on stage {}",
                    conn.from_stage
                )));
            }
            out_edges[conn.from_stage].push(conn.to_stage);
            in_edges[conn.to_stage].push(conn.from_stage);
            in_degree[conn.to_stage] += 1;
        }

        // Validate source/sink constraints.
        for (idx, stage) in self.stages.iter().enumerate() {
            if stage.role == ComponentRole::Source && !in_edges[idx].is_empty() {
                return Err(FlowCreationError::InvalidTopology(format!(
                    "Source stage {idx} (component {}) has incoming connections",
                    stage.component_id
                )));
            }
            if stage.role == ComponentRole::Sink && !out_edges[idx].is_empty() {
                return Err(FlowCreationError::InvalidTopology(format!(
                    "Sink stage {idx} (component {}) has outgoing connections",
                    stage.component_id
                )));
            }
        }

        // Kahn's algorithm for topological sort (cycle detection).
        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut in_deg = in_degree.clone();
        for (idx, &deg) in in_deg.iter().enumerate() {
            if deg == 0 {
                queue.push_back(idx);
            }
        }
        let mut visited_count = 0usize;
        while let Some(node) = queue.pop_front() {
            visited_count += 1;
            for &neighbor in &out_edges[node] {
                in_deg[neighbor] -= 1;
                if in_deg[neighbor] == 0 {
                    queue.push_back(neighbor);
                }
            }
        }
        if visited_count != n {
            return Err(FlowCreationError::InvalidTopology(
                "topology contains a cycle".into(),
            ));
        }

        // BFS from sources to check reachability.
        let mut reachable = vec![false; n];
        let mut bfs_queue: VecDeque<usize> = VecDeque::new();
        for (idx, stage) in self.stages.iter().enumerate() {
            if stage.role == ComponentRole::Source {
                reachable[idx] = true;
                bfs_queue.push_back(idx);
            }
        }
        while let Some(node) = bfs_queue.pop_front() {
            for &neighbor in &out_edges[node] {
                if !reachable[neighbor] {
                    reachable[neighbor] = true;
                    bfs_queue.push_back(neighbor);
                }
            }
        }
        for (idx, &is_reachable) in reachable.iter().enumerate() {
            if !is_reachable {
                return Err(FlowCreationError::InvalidTopology(format!(
                    "stage {idx} (component {}) is not reachable from any source",
                    self.stages[idx].component_id
                )));
            }
        }

        Ok(())
    }

    /// Returns the topological order of stages (sources first, sinks last).
    ///
    /// # Preconditions
    /// The topology must have passed `validate()`.
    ///
    /// # COLD PATH — called once during flow setup.
    pub fn topological_order(&self) -> Vec<usize> {
        let n = self.stages.len();
        let mut in_degree = vec![0u32; n];
        let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        for conn in &self.connections {
            out_edges[conn.from_stage].push(conn.to_stage);
            in_degree[conn.to_stage] += 1;
        }

        let mut queue: VecDeque<usize> = VecDeque::new();
        for (idx, &deg) in in_degree.iter().enumerate() {
            if deg == 0 {
                queue.push_back(idx);
            }
        }
        let mut order = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            for &neighbor in &out_edges[node] {
                in_degree[neighbor] -= 1;
                if in_degree[neighbor] == 0 {
                    queue.push_back(neighbor);
                }
            }
        }
        order
    }

    /// Returns the number of stages.
    #[inline]
    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    /// Returns the number of connections (streams).
    #[inline]
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }
}

impl fmt::Display for FlowTopology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FlowTopology({} stages, {} connections)",
            self.stages.len(),
            self.connections.len()
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::ComponentRole;

    fn source_stage(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Source,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn processor_stage(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Processor,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn sink_stage(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Sink,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn conn(from: usize, to: usize) -> StreamConnection {
        StreamConnection {
            from_stage: from,
            to_stage: to,
            config: StreamConfig::default(),
        }
    }

    #[test]
    fn test_valid_source_sink_topology() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        assert!(topo.validate().is_ok());
    }

    #[test]
    fn test_valid_three_stage_topology() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), processor_stage(2), sink_stage(3)],
            connections: vec![conn(0, 1), conn(1, 2)],
        };
        assert!(topo.validate().is_ok());
    }

    #[test]
    fn test_empty_topology_rejected() {
        let topo = FlowTopology::empty();
        assert!(matches!(
            topo.validate(),
            Err(FlowCreationError::InvalidTopology(_))
        ));
    }

    #[test]
    fn test_no_source_rejected() {
        let topo = FlowTopology {
            stages: vec![processor_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        let err = topo.validate().unwrap_err();
        assert!(format!("{err}").contains("no Source"));
    }

    #[test]
    fn test_no_sink_rejected() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), processor_stage(2)],
            connections: vec![conn(0, 1)],
        };
        let err = topo.validate().unwrap_err();
        assert!(format!("{err}").contains("no Sink"));
    }

    #[test]
    fn test_self_loop_rejected() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 0)],
        };
        let err = topo.validate().unwrap_err();
        assert!(format!("{err}").contains("self-loop"));
    }

    #[test]
    fn test_cycle_rejected() {
        let topo = FlowTopology {
            stages: vec![
                source_stage(1),
                processor_stage(2),
                processor_stage(3),
                sink_stage(4),
            ],
            connections: vec![conn(0, 1), conn(1, 2), conn(2, 1), conn(2, 3)],
        };
        let err = topo.validate().unwrap_err();
        assert!(format!("{err}").contains("cycle"));
    }

    #[test]
    fn test_unreachable_stage_rejected() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2), processor_stage(3)],
            connections: vec![conn(0, 1)],
        };
        let err = topo.validate().unwrap_err();
        assert!(format!("{err}").contains("not reachable"));
    }

    #[test]
    fn test_source_with_incoming_rejected() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), source_stage(2), sink_stage(3)],
            connections: vec![conn(0, 1), conn(1, 2)],
        };
        let err = topo.validate().unwrap_err();
        assert!(format!("{err}").contains("Source stage 1"));
    }

    #[test]
    fn test_out_of_bounds_rejected() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 5)],
        };
        let err = topo.validate().unwrap_err();
        assert!(format!("{err}").contains("out-of-bounds"));
    }

    #[test]
    fn test_topological_order_linear() {
        let topo = FlowTopology {
            stages: vec![source_stage(1), processor_stage(2), sink_stage(3)],
            connections: vec![conn(0, 1), conn(1, 2)],
        };
        let order = topo.topological_order();
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn test_topological_order_fan_out() {
        // Source → Sink1, Source → Sink2
        let topo = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2), sink_stage(3)],
            connections: vec![conn(0, 1), conn(0, 2)],
        };
        let order = topo.topological_order();
        assert_eq!(order[0], 0); // Source first
        assert!(order.contains(&1));
        assert!(order.contains(&2));
    }
}
