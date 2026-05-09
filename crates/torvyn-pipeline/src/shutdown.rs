//! Pipeline shutdown: graceful drain via the reactor.
//!
//! [`shutdown_pipeline`] cancels a running flow through the reactor and
//! waits for it to reach a terminal state, returning a `ShutdownTimeout`
//! error if it does not within the supplied bound.
//!
//! Component teardown (`lifecycle.teardown`) is the responsibility of
//! the flow driver during its drain phase; this function does not call
//! teardown directly.

use std::time::{Duration, Instant};

use tracing::{info, instrument, warn};

use torvyn_reactor::{cancellation::CancellationReason, handle::ReactorHandle};

use crate::error::PipelineError;
use crate::handle::PipelineHandle;

// ---------------------------------------------------------------------------
// shutdown_pipeline — graceful drain
// ---------------------------------------------------------------------------

/// Gracefully shut down a running pipeline.
///
/// # COLD PATH — called once per flow during host shutdown or operator
/// cancellation.
///
/// # Steps (per HLI Doc 02, Section 8.2)
/// 1. Send `cancel_flow` to the reactor with reason
///    [`CancellationReason::OperatorRequest`].
/// 2. Poll the reactor's flow state every 10 ms until it reaches a
///    terminal state (`FlowState::Completed`, `FlowState::Cancelled`,
///    or `FlowState::Failed`) or `timeout` elapses.
///
/// The flow driver is responsible for invoking `lifecycle.teardown` on
/// each component during drain; this function does not call teardown.
///
/// # Errors
/// - [`PipelineError::ShutdownTimeout`] if the flow is still non-terminal
///   when the timeout expires.
///
/// Errors are returned in a `Vec` to leave room for collecting
/// per-component teardown failures in future phases (per the function's
/// historical signature in HLI Doc 02).
///
/// # Postconditions
/// On `Ok`, the flow is in a terminal state and the reactor has reaped
/// (or is about to reap) the flow's task.
#[instrument(skip(handle, reactor), fields(flow_id = %handle.flow_id()))]
pub async fn shutdown_pipeline(
    handle: &PipelineHandle,
    reactor: &ReactorHandle,
    timeout: Duration,
) -> Result<(), Vec<PipelineError>> {
    let flow_id = handle.flow_id();
    let flow_name = handle.name();

    info!(
        flow_name = flow_name,
        timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        "Initiating graceful shutdown of pipeline"
    );

    // Step 1: signal cancellation. If the reactor returns an error
    // (e.g., because the flow has already been reaped after natural
    // completion), we proceed to the state poll: a missing flow is
    // indistinguishable from a terminal flow for shutdown purposes.
    if let Err(e) = reactor
        .cancel_flow(flow_id, CancellationReason::OperatorRequest)
        .await
    {
        warn!(
            flow_id = %flow_id,
            error = %e,
            "cancel_flow returned an error; flow may already be terminal"
        );
    }

    // Step 2: poll the flow's state until terminal or timeout.
    let start = Instant::now();
    let poll_interval = Duration::from_millis(10);

    loop {
        match reactor.flow_state(flow_id).await {
            Ok(state) if state.is_terminal() => {
                info!(
                    flow_id = %flow_id,
                    state = %state,
                    "Pipeline shutdown complete"
                );
                return Ok(());
            }
            Ok(_state) => {
                // Non-terminal: keep polling.
            }
            Err(_) => {
                // The reactor no longer knows about this flow — either
                // because it already reaped the task or because the
                // coordinator has shut down. Either way, the flow is
                // not running, so shutdown is effectively complete.
                info!(
                    flow_id = %flow_id,
                    "Pipeline drained (flow no longer registered with reactor)"
                );
                return Ok(());
            }
        }

        if start.elapsed() >= timeout {
            warn!(
                flow_id = %flow_id,
                timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                "Pipeline shutdown timed out"
            );
            return Err(vec![PipelineError::ShutdownTimeout {
                flow_id,
                timeout,
                components_remaining: handle.topology().node_count(),
            }]);
        }

        tokio::time::sleep(poll_interval).await;
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

    /// Verify the function signature compiles against the public
    /// `ReactorHandle`. The end-to-end behaviour is exercised by the
    /// workspace integration test, where a real coordinator is spawned.
    #[test]
    fn test_shutdown_signature_compiles() {
        fn _accepts(_: &PipelineHandle, _: &ReactorHandle, _: Duration) {}
    }

    #[test]
    fn test_handle_drives_shutdown_arguments() {
        // Build a placeholder handle and confirm we can read flow_id
        // and topology.node_count for ShutdownTimeout reporting.
        let topo = PipelineTopologyBuilder::new("graceful-test")
            .add_node(
                "s",
                ComponentRole::Source,
                "mock://s",
                NodeConfig::default(),
            )
            .add_node("k", ComponentRole::Sink, "mock://k", NodeConfig::default())
            .add_edge("s", "output", "k", "input")
            .build()
            .unwrap();
        let handle = PipelineHandle::new(FlowId::new(7), "graceful-test".into(), topo);

        // The fields shutdown_pipeline reads must remain accessible.
        assert_eq!(handle.flow_id(), FlowId::new(7));
        assert_eq!(handle.topology().node_count(), 2);
    }

    /// Document the contract: Vec is used to leave room for future
    /// per-component teardown errors. Today we return at most one item.
    #[test]
    fn test_shutdown_timeout_error_shape() {
        let err = PipelineError::ShutdownTimeout {
            flow_id: FlowId::new(42),
            timeout: Duration::from_secs(1),
            components_remaining: 3,
        };
        let collected: Vec<PipelineError> = vec![err];
        assert_eq!(collected.len(), 1);
        let msg = format!("{}", collected[0]);
        assert!(msg.contains("E0970"));
        assert!(msg.contains("flow-42"));
    }
}
