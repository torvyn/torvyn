//! Pipeline instantiation: from topology to running flow.
//!
//! Per Doc 02, Section 8.1 and Doc 10, Section 3.4.
//!
//! Steps:
//! 1. For each node in topological order:
//!    a. Compile the component (or fetch from cache).
//!    b. Configure security sandbox.
//!    c. Instantiate the component via `WasmEngine`.
//!    d. Call `lifecycle.init()` if the component exports it.
//! 2. Build the reactor `FlowConfig` from topology + instances.
//! 3. Register the flow with the reactor.
//! 4. Return a `PipelineHandle`.

use tracing::{info, instrument};

use torvyn_types::FlowId;

use crate::error::PipelineError;
use crate::handle::PipelineHandle;
use crate::topology::PipelineTopology;

// ---------------------------------------------------------------------------
// instantiate_pipeline
// ---------------------------------------------------------------------------

/// Instantiate a validated pipeline topology into a running flow.
///
/// # COLD PATH — called once per flow during pipeline startup.
///
/// # Steps
/// 1. For each node in topological order:
///    a. Look up component bytes (from file path or OCI reference).
///    b. Compile the component via `engine.compile_component()`.
///    c. Configure security sandbox via `security.configure_sandbox()`.
///    d. Instantiate the component via `engine.instantiate()`.
///    e. Call `invoker.invoke_init()` if the component exports lifecycle.
/// 2. Build `FlowConfig` from topology + instantiated components.
/// 3. Register flow with reactor via `reactor.create_flow()`.
/// 4. Return `PipelineHandle`.
///
/// # Errors
/// Returns `PipelineError` for any failure in the instantiation sequence.
/// All component instances created before the failure are cleaned up.
///
/// # Panics
/// Panics if `topology.execution_order()` contains an out-of-bounds node index,
/// which cannot happen with a topology built via `PipelineTopologyBuilder::build`.
///
/// # Preconditions
/// - `topology` has been validated (built via `PipelineTopologyBuilder::build`).
/// - All subsystem handles are initialized and functional.
///
/// # Postconditions
/// - On success, the reactor is running the flow. The returned
///   `PipelineHandle` provides lifecycle management.
/// - On failure, no resources are leaked (partial instantiation is cleaned up).
///
/// # Implementation Note
/// The full implementation depends on the concrete types from torvyn-engine,
/// torvyn-reactor, torvyn-resources, and torvyn-security. The current
/// implementation is a placeholder that will be completed once those crates
/// provide their mock features.
///
/// CROSS-CRATE DEPENDENCY: This function is the primary integration point
/// across all Torvyn subsystems. Verify all trait method signatures against
/// their respective LLI documents before full implementation.
#[instrument(skip(topology), fields(flow_name = %topology.name()))]
pub async fn instantiate_pipeline(
    topology: &PipelineTopology,
    // CROSS-CRATE DEPENDENCY: Full context will include:
    // engine: &impl WasmEngine,
    // invoker: &impl ComponentInvoker,
    // reactor: &ReactorHandle,
    // resources: &ResourceManager,
    // security: &SecurityManager,
) -> Result<PipelineHandle, PipelineError> {
    info!(
        "Instantiating pipeline '{}' with {} nodes and {} edges",
        topology.name(),
        topology.node_count(),
        topology.edge_count(),
    );

    // Step 1: Instantiate components in topological order
    for &node_idx in topology.execution_order() {
        let node = topology
            .node(node_idx)
            .expect("valid index from execution_order");

        info!(
            node_name = node.name(),
            role = %node.role(),
            component_ref = node.component_ref(),
            "Instantiating component"
        );

        // Steps 1a–1e are placeholder stubs:
        // In production, this delegates to WasmEngine for compilation
        // and instantiation, SecurityManager for sandbox configuration,
        // and ComponentInvoker for lifecycle.init() calls.
    }

    // Steps 2–3: Build FlowConfig and register with reactor are stubs.
    // Will be completed when cross-crate integration is finalized.

    // Step 4: Return handle
    // IMPLEMENTATION NOTE: For now, return a placeholder. The real
    // implementation constructs from the flow_id and reactor handle.
    let flow_id = FlowId::new(0); // placeholder

    info!(
        flow_id = %flow_id,
        "Pipeline '{}' instantiated successfully",
        topology.name(),
    );

    Ok(PipelineHandle::new(
        flow_id,
        topology.name().to_owned(),
        topology.clone(),
    ))
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

    // NOTE: Full instantiation tests require mock implementations of
    // WasmEngine, ComponentInvoker, ReactorHandle, ResourceManager, and
    // SecurityManager. These will be available once those crates provide
    // their `mock` feature implementations.
    //
    // For Phase 0, we test that the function signature compiles and that
    // topology-level preconditions are met.

    #[tokio::test]
    async fn test_instantiate_pipeline_compiles() {
        let topo = PipelineTopologyBuilder::new("test")
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

        // The function compiles and the topology is valid
        let result = instantiate_pipeline(&topo).await;
        // Currently returns a placeholder — will be fully tested with mocks
        assert!(result.is_ok());
    }
}
