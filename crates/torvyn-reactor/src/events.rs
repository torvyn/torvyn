//! Reactor events and commands.
//!
//! Per Doc 04 §12.4: commands from host to coordinator, events from
//! reactor to host/observability.

use std::time::Duration;

use tokio::sync::oneshot;

use torvyn_engine::ComponentInstance;
use torvyn_types::{ComponentId, FlowId, FlowState, StreamId};

use crate::cancellation::CancellationReason;
use crate::config::FlowConfig;
use crate::error::{FlowCreationError, FlowError};
use crate::fairness::FlowPriority;
use crate::metrics::FlowCompletionStats;

// ---------------------------------------------------------------------------
// ReactorCommand
// ---------------------------------------------------------------------------

/// Commands from the host/operator to the reactor coordinator.
///
/// Sent via a Tokio `mpsc` channel. Each command includes a `oneshot`
/// sender for the response.
pub enum ReactorCommand {
    /// Create a new flow with the given configuration.
    ///
    /// Note: as of v0.1, this is a placeholder kept for backward
    /// compatibility. To run a flow with real component instances, use
    /// [`SpawnFlow`](Self::SpawnFlow).
    CreateFlow(
        FlowConfig,
        oneshot::Sender<Result<FlowId, FlowCreationError>>,
    ),
    /// Cancel a running flow with the given reason.
    CancelFlow(
        FlowId,
        CancellationReason,
        oneshot::Sender<Result<(), FlowError>>,
    ),
    /// Query the current state of a flow.
    QueryFlowState(FlowId, oneshot::Sender<Result<FlowState, FlowError>>),
    /// List all flows with their current states.
    ListFlows(oneshot::Sender<Vec<(FlowId, FlowState)>>),
    /// Update the priority of a running flow.
    UpdatePriority(FlowId, FlowPriority, oneshot::Sender<Result<(), FlowError>>),
    /// Initiate graceful shutdown with the given timeout.
    Shutdown(Duration, oneshot::Sender<ShutdownResult>),
    /// Spawn a fully-instantiated flow. This is the production path that
    /// pairs a `FlowConfig` with the matching `Vec<ComponentInstance>`
    /// produced by the host's engine. The coordinator builds streams,
    /// constructs a real [`FlowDriver`](crate::flow_driver::FlowDriver),
    /// and runs it on a Tokio task.
    SpawnFlow {
        /// Per-flow configuration (topology, timeouts, queue defaults).
        config: FlowConfig,
        /// Component instances, one per stage in `config.topology.stages`.
        /// The order must match `config.topology.stages` exactly.
        instances: Vec<ComponentInstance>,
        /// Reply channel for the assigned `FlowId`.
        reply: oneshot::Sender<Result<FlowId, FlowCreationError>>,
    },
}

impl std::fmt::Debug for ReactorCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReactorCommand::CreateFlow(cfg, _) => write!(f, "CreateFlow({:?})", cfg.priority),
            ReactorCommand::CancelFlow(id, reason, _) => {
                write!(f, "CancelFlow({id}, {reason})")
            }
            ReactorCommand::QueryFlowState(id, _) => write!(f, "QueryFlowState({id})"),
            ReactorCommand::ListFlows(_) => write!(f, "ListFlows"),
            ReactorCommand::UpdatePriority(id, pri, _) => {
                write!(f, "UpdatePriority({id}, {pri})")
            }
            ReactorCommand::Shutdown(timeout, _) => write!(f, "Shutdown({timeout:?})"),
            ReactorCommand::SpawnFlow {
                config, instances, ..
            } => {
                write!(
                    f,
                    "SpawnFlow(stages={}, instances={}, priority={:?})",
                    config.topology.stages.len(),
                    instances.len(),
                    config.priority,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ReactorEvent
// ---------------------------------------------------------------------------

/// Events emitted by the reactor for the host and observability system.
///
/// Per Doc 04 §12.4. Sent via a Tokio `mpsc` channel.
#[derive(Clone, Debug)]
pub enum ReactorEvent {
    /// A flow's state changed.
    FlowStateChanged {
        /// The flow whose state changed.
        flow_id: FlowId,
        /// The previous state.
        old_state: FlowState,
        /// The new state.
        new_state: FlowState,
    },
    /// A flow completed (with stats).
    FlowCompleted {
        /// The flow that completed.
        flow_id: FlowId,
        /// Completion statistics.
        stats: FlowCompletionStats,
    },
    /// A flow driver yielded to Tokio.
    FlowYielded {
        /// The flow that yielded.
        flow_id: FlowId,
        /// Elements processed since the last yield.
        elements_since_last_yield: u64,
    },
    /// A backpressure state change occurred.
    BackpressureChanged {
        /// The flow containing the stream.
        flow_id: FlowId,
        /// The stream whose backpressure state changed.
        stream_id: StreamId,
        /// Whether backpressure is now active.
        activated: bool,
        /// Current queue depth.
        queue_depth: u32,
    },
    /// A component error occurred.
    ComponentError {
        /// The flow containing the errored component.
        flow_id: FlowId,
        /// The component that errored.
        component_id: ComponentId,
        /// Description of the error.
        error: String,
    },
}

// ---------------------------------------------------------------------------
// ShutdownResult
// ---------------------------------------------------------------------------

/// Result of a graceful reactor shutdown.
#[derive(Clone, Debug)]
pub struct ShutdownResult {
    /// Number of flows that completed normally during shutdown.
    pub completed: usize,
    /// Number of flows that were forcefully cancelled.
    pub cancelled: usize,
    /// Number of flows that timed out during drain.
    pub timed_out: usize,
}

impl std::fmt::Display for ShutdownResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ShutdownResult(completed={}, cancelled={}, timed_out={})",
            self.completed, self.cancelled, self.timed_out
        )
    }
}
