//! Linked pipeline representation — the output of the linking process.
//!
//! A `LinkedPipeline` contains all information needed to instantiate and
//! run a pipeline: verified topology, resolved imports, capability grants,
//! queue configurations, and per-component metadata.

use std::collections::HashMap;
use std::path::PathBuf;

use torvyn_types::{BackpressurePolicy, ComponentRole, StreamId};

use crate::resolver::ImportResolution;
use crate::topology::CapabilityGrant;

/// A fully linked pipeline, ready for instantiation.
///
/// Produced by [`PipelineLinker::link`](crate::PipelineLinker::link).
/// All imports are resolved, capabilities are verified, and the topology
/// has been validated as a DAG.
///
/// # Invariants
/// - `components` is non-empty.
/// - `components` is in topological order.
/// - All imports in every component are resolved.
/// - All required capabilities are granted.
/// - The topology is a valid DAG.
/// - `connections` maps each stream to its queue configuration.
///
/// # Examples
/// ```no_run
/// use torvyn_linker::LinkedPipeline;
///
/// // LinkedPipeline is produced by PipelineLinker::link()
/// // and consumed by the host runtime for instantiation.
/// ```
#[derive(Debug, Clone)]
pub struct LinkedPipeline {
    /// Pipeline name.
    pub name: String,

    /// Linked components in topological order.
    pub components: Vec<LinkedComponent>,

    /// Stream connections with queue configuration.
    pub connections: Vec<LinkedConnection>,

    /// Topological order of component names.
    pub topological_order: Vec<String>,
}

/// A single linked component with all resolved metadata.
///
/// # Invariants
/// - `resolved_imports` is complete: every import has a resolution.
/// - `capability_grants` satisfies all required capabilities.
#[derive(Debug, Clone)]
pub struct LinkedComponent {
    /// Component name (unique within the pipeline).
    pub name: String,

    /// Component role.
    pub role: ComponentRole,

    /// Path to the component artifact.
    pub artifact_path: PathBuf,

    /// Resolved imports: import name → resolution.
    pub resolved_imports: HashMap<String, ImportResolution>,

    /// Capability grants for this component.
    pub capability_grants: Vec<CapabilityGrant>,

    /// Per-component configuration string for `lifecycle.init()`.
    pub config: Option<String>,
}

/// A stream connection between two linked components.
///
/// Carries queue configuration for the reactor to use during
/// flow construction.
#[derive(Debug, Clone)]
pub struct LinkedConnection {
    /// Stream identifier (assigned during linking).
    pub stream_id: StreamId,

    /// Source component name.
    pub from_component: String,
    /// Source output port name.
    pub from_port: String,

    /// Destination component name.
    pub to_component: String,
    /// Destination input port name.
    pub to_port: String,

    /// Queue depth for this stream.
    pub queue_depth: u32,

    /// Backpressure policy for this stream.
    pub backpressure_policy: BackpressurePolicy,
}

impl LinkedPipeline {
    /// Returns the number of components in the pipeline.
    ///
    /// # COLD PATH
    pub fn component_count(&self) -> usize {
        self.components.len()
    }

    /// Returns the number of stream connections.
    ///
    /// # COLD PATH
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Look up a linked component by name.
    ///
    /// # COLD PATH
    pub fn get_component(&self, name: &str) -> Option<&LinkedComponent> {
        self.components.iter().find(|c| c.name == name)
    }

    /// Returns an iterator over components in topological order.
    ///
    /// # COLD PATH
    pub fn iter_components(&self) -> impl Iterator<Item = &LinkedComponent> {
        self.components.iter()
    }

    /// Returns all connections originating from the given component.
    ///
    /// # COLD PATH
    pub fn outgoing_connections(&self, component_name: &str) -> Vec<&LinkedConnection> {
        self.connections
            .iter()
            .filter(|c| c.from_component == component_name)
            .collect()
    }

    /// Returns all connections arriving at the given component.
    ///
    /// # COLD PATH
    pub fn incoming_connections(&self, component_name: &str) -> Vec<&LinkedConnection> {
        self.connections
            .iter()
            .filter(|c| c.to_component == component_name)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::StreamId;

    fn make_linked_pipeline() -> LinkedPipeline {
        LinkedPipeline {
            name: "test-pipeline".into(),
            components: vec![
                LinkedComponent {
                    name: "src".into(),
                    role: ComponentRole::Source,
                    artifact_path: "src.wasm".into(),
                    resolved_imports: HashMap::new(),
                    capability_grants: vec![],
                    config: None,
                },
                LinkedComponent {
                    name: "snk".into(),
                    role: ComponentRole::Sink,
                    artifact_path: "snk.wasm".into(),
                    resolved_imports: HashMap::new(),
                    capability_grants: vec![],
                    config: Some("{\"output\":\"stdout\"}".into()),
                },
            ],
            connections: vec![LinkedConnection {
                stream_id: StreamId::new(0),
                from_component: "src".into(),
                from_port: "output".into(),
                to_component: "snk".into(),
                to_port: "input".into(),
                queue_depth: 64,
                backpressure_policy: BackpressurePolicy::BlockProducer,
            }],
            topological_order: vec!["src".into(), "snk".into()],
        }
    }

    #[test]
    fn test_linked_pipeline_component_count() {
        let lp = make_linked_pipeline();
        assert_eq!(lp.component_count(), 2);
    }

    #[test]
    fn test_linked_pipeline_connection_count() {
        let lp = make_linked_pipeline();
        assert_eq!(lp.connection_count(), 1);
    }

    #[test]
    fn test_linked_pipeline_get_component() {
        let lp = make_linked_pipeline();
        assert!(lp.get_component("src").is_some());
        assert!(lp.get_component("nonexistent").is_none());
    }

    #[test]
    fn test_linked_pipeline_outgoing_connections() {
        let lp = make_linked_pipeline();
        let out = lp.outgoing_connections("src");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_component, "snk");
    }

    #[test]
    fn test_linked_pipeline_incoming_connections() {
        let lp = make_linked_pipeline();
        let inc = lp.incoming_connections("snk");
        assert_eq!(inc.len(), 1);
        assert_eq!(inc[0].from_component, "src");
    }

    #[test]
    fn test_linked_pipeline_topological_order() {
        let lp = make_linked_pipeline();
        assert_eq!(lp.topological_order, vec!["src", "snk"]);
    }
}
