//! Pipeline shutdown: graceful draining and forced cancellation.
//!
//! Per Doc 02, Section 8.2:
//! 1. Stop sources (no new data).
//! 2. Drain in-flight elements.
//! 3. Call `lifecycle.teardown()` on all components (per C02-10).
//! 4. Release all resources.
//! 5. Flush observability.
//!
//! Bounded time: graceful shutdown must complete within a configurable
//! timeout (default: 30 seconds).

use std::time::Duration;

use tracing::{info, instrument, warn};

use crate::error::PipelineError;
use crate::handle::PipelineHandle;

/// Default timeout for graceful shutdown.
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// Default timeout for individual component teardown.
pub const DEFAULT_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Initiate graceful shutdown of a pipeline.
///
/// # COLD PATH — called once per flow teardown.
///
/// # Steps
/// 1. Signal the reactor to begin draining the flow.
/// 2. Wait for draining to complete, up to `timeout`.
/// 3. If timeout expires, force-cancel remaining components.
/// 4. Call `lifecycle.teardown()` on all components.
/// 5. Release all flow resources via the resource manager.
/// 6. Flush observability events.
///
/// # Errors
/// Returns `PipelineError::ShutdownTimeout` if graceful shutdown times out.
/// Returns `PipelineError::TeardownFailed` if a component's teardown fails
/// (non-fatal — shutdown continues).
///
/// # Postconditions
/// After this function returns (success or error), all flow resources
/// have been released and the flow is no longer registered with the reactor.
///
/// # Implementation Note
/// The full implementation depends on concrete types from torvyn-reactor,
/// torvyn-resources, and torvyn-engine. The current implementation is a
/// placeholder that will be completed once cross-crate integration is finalized.
#[instrument(skip(handle), fields(flow_id = %handle.flow_id()))]
pub async fn shutdown_pipeline(
    handle: &PipelineHandle,
    timeout: Duration,
) -> Result<(), Vec<PipelineError>> {
    info!(
        "Initiating graceful shutdown of pipeline '{}' ({}) with timeout {:?}",
        handle.name(),
        handle.flow_id(),
        timeout,
    );

    let errors: Vec<PipelineError> = Vec::new();

    // Steps 1–5 are placeholder stubs:
    // Step 1: Signal reactor to drain
    // CROSS-CRATE DEPENDENCY: reactor.cancel_flow(flow_id, CancellationReason::Shutdown)
    //
    // Step 2: Wait for drain completion (with timeout)
    //
    // Step 3: Teardown components in reverse topological order (per C02-10)
    //
    // Step 4: Release flow resources
    // CROSS-CRATE DEPENDENCY: resources.release_flow_resources(flow_id)
    //
    // Step 5: Flush observability

    if errors.is_empty() {
        info!("Pipeline '{}' shut down cleanly", handle.name());
        Ok(())
    } else {
        warn!(
            "Pipeline '{}' shut down with {} error(s)",
            handle.name(),
            errors.len()
        );
        Err(errors)
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
    use torvyn_types::{ComponentRole, FlowId};

    #[tokio::test]
    async fn test_shutdown_pipeline_compiles() {
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

        let handle = PipelineHandle::new(FlowId::new(1), "test".into(), topo);

        // Currently a no-op — full test requires mock subsystems
        let result = shutdown_pipeline(&handle, DEFAULT_SHUTDOWN_TIMEOUT).await;
        assert!(result.is_ok());
    }
}
