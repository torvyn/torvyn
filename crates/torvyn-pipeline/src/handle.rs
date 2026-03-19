//! Runtime handle for a running pipeline.
//!
//! [`PipelineHandle`] is the primary interface for managing a running
//! flow's lifecycle after instantiation.

use torvyn_types::FlowId;

use crate::topology::PipelineTopology;

// ---------------------------------------------------------------------------
// PipelineHandle
// ---------------------------------------------------------------------------

/// Handle to a running pipeline.
///
/// Returned by [`crate::instantiate::instantiate_pipeline`] on success.
/// Provides access to flow identity, topology, and lifecycle operations.
///
/// # Invariants
/// - `flow_id` is valid and registered with the reactor.
/// - `topology` is the validated topology that was used to create the flow.
///
/// # Examples
/// ```
/// use torvyn_pipeline::PipelineHandle;
/// use torvyn_types::FlowId;
/// use torvyn_pipeline::{PipelineTopologyBuilder, NodeConfig};
/// use torvyn_types::ComponentRole;
///
/// let topo = PipelineTopologyBuilder::new("example")
///     .add_node("s", ComponentRole::Source, "file://s.wasm", NodeConfig::default())
///     .add_node("k", ComponentRole::Sink, "file://k.wasm", NodeConfig::default())
///     .add_edge("s", "output", "k", "input")
///     .build()
///     .unwrap();
///
/// let handle = PipelineHandle::new(FlowId::new(1), "example".into(), topo);
/// assert_eq!(handle.flow_id(), FlowId::new(1));
/// assert_eq!(handle.name(), "example");
/// ```
#[derive(Debug, Clone)]
pub struct PipelineHandle {
    /// Unique flow identifier from the reactor.
    flow_id: FlowId,

    /// Human-readable pipeline name.
    name: String,

    /// The validated topology used to create this flow.
    topology: PipelineTopology,
}

impl PipelineHandle {
    /// Create a new pipeline handle.
    ///
    /// # COLD PATH — called once after successful instantiation.
    pub fn new(flow_id: FlowId, name: String, topology: PipelineTopology) -> Self {
        Self {
            flow_id,
            name,
            topology,
        }
    }

    /// Returns the flow identifier.
    #[inline]
    pub fn flow_id(&self) -> FlowId {
        self.flow_id
    }

    /// Returns the pipeline name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the topology.
    #[inline]
    pub fn topology(&self) -> &PipelineTopology {
        &self.topology
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::PipelineTopologyBuilder;
    use crate::topology::NodeConfig;
    use torvyn_types::ComponentRole;

    #[test]
    fn test_pipeline_handle_accessors() {
        let topo = PipelineTopologyBuilder::new("my-flow")
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

        let handle = PipelineHandle::new(FlowId::new(42), "my-flow".into(), topo);

        assert_eq!(handle.flow_id(), FlowId::new(42));
        assert_eq!(handle.name(), "my-flow");
        assert_eq!(handle.topology().node_count(), 2);
    }
}
