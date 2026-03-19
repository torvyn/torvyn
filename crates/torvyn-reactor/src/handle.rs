//! ReactorHandle: public API for managing flows.
//!
//! Per Doc 04 §12.2: the handle through which the host creates, manages,
//! and queries flows.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use torvyn_types::{FlowId, FlowState};

use crate::cancellation::CancellationReason;
use crate::config::FlowConfig;
use crate::error::{FlowCreationError, FlowError};
use crate::events::{ReactorCommand, ShutdownResult};
use crate::fairness::FlowPriority;

// ---------------------------------------------------------------------------
// ReactorHandle
// ---------------------------------------------------------------------------

/// Handle to the reactor, used by the host to manage flows.
///
/// The handle communicates with the reactor coordinator via Tokio channels.
/// It is `Clone`-able: multiple handles can be used from different tasks.
///
/// # Examples
/// ```no_run
/// use torvyn_reactor::{ReactorHandle, FlowConfig, FlowTopology};
///
/// # async fn example(handle: ReactorHandle) {
/// let config = FlowConfig::default_with_topology(FlowTopology::empty());
/// let flow_id = handle.create_flow(config).await.unwrap();
/// let state = handle.flow_state(flow_id).await.unwrap();
/// # }
/// ```
#[derive(Clone)]
pub struct ReactorHandle {
    command_tx: mpsc::Sender<ReactorCommand>,
}

impl ReactorHandle {
    /// Create a new `ReactorHandle` from a command channel sender.
    ///
    /// # COLD PATH
    pub fn new(command_tx: mpsc::Sender<ReactorCommand>) -> Self {
        Self { command_tx }
    }

    /// Create and start a new flow.
    ///
    /// # COLD PATH
    ///
    /// # Errors
    /// - [`FlowCreationError::InvalidTopology`] if the topology is invalid.
    /// - [`FlowCreationError::ReactorShuttingDown`] if the reactor is shutting down.
    pub async fn create_flow(&self, config: FlowConfig) -> Result<FlowId, FlowCreationError> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(ReactorCommand::CreateFlow(config, tx))
            .await
            .map_err(|_| FlowCreationError::ReactorShuttingDown)?;
        rx.await
            .map_err(|_| FlowCreationError::Internal("coordinator dropped".into()))?
    }

    /// Cancel a running flow.
    ///
    /// # COLD PATH
    pub async fn cancel_flow(
        &self,
        flow_id: FlowId,
        reason: CancellationReason,
    ) -> Result<(), FlowError> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(ReactorCommand::CancelFlow(flow_id, reason, tx))
            .await
            .map_err(|_| FlowError::Internal("reactor shut down".into()))?;
        rx.await
            .map_err(|_| FlowError::Internal("coordinator dropped".into()))?
    }

    /// Query the current state of a flow.
    ///
    /// # COLD PATH
    pub async fn flow_state(&self, flow_id: FlowId) -> Result<FlowState, FlowError> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(ReactorCommand::QueryFlowState(flow_id, tx))
            .await
            .map_err(|_| FlowError::Internal("reactor shut down".into()))?;
        rx.await
            .map_err(|_| FlowError::Internal("coordinator dropped".into()))?
    }

    /// List all active flows.
    ///
    /// # COLD PATH
    pub async fn list_flows(&self) -> Vec<(FlowId, FlowState)> {
        let (tx, rx) = oneshot::channel();
        if self
            .command_tx
            .send(ReactorCommand::ListFlows(tx))
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Update the priority of a running flow.
    ///
    /// # COLD PATH
    pub async fn update_flow_priority(
        &self,
        flow_id: FlowId,
        priority: FlowPriority,
    ) -> Result<(), FlowError> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(ReactorCommand::UpdatePriority(flow_id, priority, tx))
            .await
            .map_err(|_| FlowError::Internal("reactor shut down".into()))?;
        rx.await
            .map_err(|_| FlowError::Internal("coordinator dropped".into()))?
    }

    /// Shut down the reactor gracefully, draining all flows.
    ///
    /// # COLD PATH
    pub async fn shutdown(&self, timeout: Duration) -> ShutdownResult {
        let (tx, rx) = oneshot::channel();
        if self
            .command_tx
            .send(ReactorCommand::Shutdown(timeout, tx))
            .await
            .is_err()
        {
            return ShutdownResult {
                completed: 0,
                cancelled: 0,
                timed_out: 0,
            };
        }
        rx.await.unwrap_or(ShutdownResult {
            completed: 0,
            cancelled: 0,
            timed_out: 0,
        })
    }
}

impl std::fmt::Debug for ReactorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReactorHandle").finish()
    }
}
