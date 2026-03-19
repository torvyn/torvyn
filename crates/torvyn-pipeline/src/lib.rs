//! `torvyn-pipeline` — Pipeline topology construction, validation, and instantiation.
//!
//! This crate is the orchestration layer for Torvyn pipeline lifecycle:
//! - **Topology model**: [`PipelineTopology`], [`TopologyNode`], [`TopologyEdge`]
//! - **Builder**: [`PipelineTopologyBuilder`] — fluent API for programmatic construction
//! - **Validation**: [`validate::validate_topology`] — DAG, connectedness, role checks
//! - **Config conversion**: [`convert::flow_def_to_topology`]
//! - **Instantiation**: [`instantiate::instantiate_pipeline`]
//! - **Shutdown**: [`shutdown::shutdown_pipeline`]
//! - **Handle**: [`PipelineHandle`] — runtime handle for a running flow
//!
//! All code in this crate is **COLD PATH**. Hot-path element processing
//! lives in `torvyn-reactor`.

#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::module_name_repetitions)]

pub mod builder;
pub mod convert;
pub mod error;
pub mod handle;
pub mod instantiate;
pub mod shutdown;
pub mod topology;
pub mod validate;

// Re-exports for convenience
pub use builder::PipelineTopologyBuilder;
pub use convert::flow_def_to_topology;
pub use error::{PipelineError, ValidationReport};
pub use handle::PipelineHandle;
pub use topology::{
    EdgeConfig, ErrorPolicy, NodeConfig, PipelineTopology, TopologyEdge, TopologyNode,
};
