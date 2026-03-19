//! `torvyn-linker` — Component linking and pipeline composition for Torvyn.
//!
//! This crate implements the static, ahead-of-execution linking algorithm
//! described in HLI Document 02, Section 4. It resolves import/export
//! relationships between components, verifies contract compatibility
//! (via `torvyn-contracts`), checks capability grants, handles diamond
//! dependencies, and produces a `LinkedPipeline` for instantiation.
//!
//! # Architecture
//!
//! The linking process has five phases:
//! 1. **Topology validation** — structural DAG checks (cycles, connectivity, roles).
//! 2. **Topological ordering** — compute processing order.
//! 3. **Import resolution** — match each import to a provider.
//! 4. **Capability checking** — verify capability grants satisfy requirements.
//! 5. **Pipeline construction** — build the `LinkedPipeline` output.
//!
//! All code in this crate is **COLD PATH** — it runs once at pipeline startup,
//! not during stream processing.
//!
//! # Usage
//!
//! ```
//! use torvyn_linker::{PipelineLinker, PipelineTopology, TopologyNode, TopologyEdge};
//! use torvyn_types::ComponentRole;
//!
//! let mut topo = PipelineTopology::new("my-pipeline".into());
//! topo.add_node(TopologyNode {
//!     name: "src".into(),
//!     role: ComponentRole::Source,
//!     artifact_path: "src.wasm".into(),
//!     config: None,
//!     capability_grants: vec![],
//! });
//! topo.add_node(TopologyNode {
//!     name: "snk".into(),
//!     role: ComponentRole::Sink,
//!     artifact_path: "snk.wasm".into(),
//!     config: None,
//!     capability_grants: vec![],
//! });
//! topo.add_edge(TopologyEdge {
//!     from_node: "src".into(),
//!     from_port: "output".into(),
//!     to_node: "snk".into(),
//!     to_port: "input".into(),
//!     queue_depth: 64,
//!     backpressure_policy: Default::default(),
//! });
//!
//! let mut linker = PipelineLinker::new();
//! let linked = linker.link_topology_only(&topo).unwrap();
//! assert_eq!(linked.component_count(), 2);
//! ```

#![deny(missing_docs)]

pub mod error;
pub mod linked_pipeline;
pub mod linker;
pub mod resolver;
pub mod topology;

// Re-exports for convenience
pub use error::{LinkDiagnostic, LinkDiagnosticCategory, LinkReport, LinkerError};
pub use linked_pipeline::{LinkedComponent, LinkedConnection, LinkedPipeline};
pub use linker::PipelineLinker;
pub use resolver::{ComponentResolution, ImportResolution, PipelineResolution};
pub use topology::{
    CapabilityGrant, PipelineTopology, TopologyEdge, TopologyNode, DEFAULT_MAX_FAN_IN,
    DEFAULT_MAX_FAN_OUT,
};
