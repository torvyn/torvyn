//! Reactor coordinator: manages flow lifecycle.
//!
//! Per Doc 04 §1.3: a single long-lived Tokio task that handles
//! flow creation/teardown and administrative commands.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use torvyn_types::{EventSink, FlowId, FlowState, StreamId};

use torvyn_engine::{ComponentInstance, ComponentInvoker};

use crate::cancellation::{CancellationReason, FlowCancellation};
use crate::config::FlowConfig;
use crate::error::{FlowCreationError, FlowError};
use crate::events::{ReactorCommand, ReactorEvent, ShutdownResult};
use crate::flow_driver::FlowDriverHandle;
use crate::metrics::FlowCompletionStats;
use crate::stream::StreamState;

/// Internal state for a managed flow.
struct FlowEntry {
    handle: FlowDriverHandle,
    join_handle: JoinHandle<(FlowId, FlowState, FlowCompletionStats)>,
}

/// The reactor coordinator.
///
/// Runs as a Tokio task, receives commands from the [`ReactorHandle`](crate::handle::ReactorHandle),
/// and spawns/manages flow driver tasks.
///
/// # Type Parameters
/// - `I`: The [`ComponentInvoker`] implementation.
/// - `E`: The [`EventSink`] implementation.
pub struct ReactorCoordinator<I: ComponentInvoker, E: EventSink> {
    /// Channel to receive commands.
    command_rx: mpsc::Receiver<ReactorCommand>,
    /// Channel to send events to the handle.
    event_tx: mpsc::Sender<ReactorEvent>,
    /// Active flows.
    flows: HashMap<FlowId, FlowEntry>,
    /// Next flow ID (monotonically increasing).
    next_flow_id: AtomicU64,
    /// The component invoker (shared across flow drivers).
    /// CROSS-CRATE DEPENDENCY: requires ComponentInvoker from torvyn-engine.
    // LLI DEVIATION: prefixed with _ to suppress dead_code warning;
    // real implementation will use this when spawning flow drivers.
    _invoker: Arc<I>,
    /// The event sink for observability.
    // LLI DEVIATION: prefixed with _ to suppress dead_code warning;
    // real implementation will clone this into each flow driver.
    _event_sink: Arc<E>,
    /// Whether the coordinator is shutting down.
    shutting_down: bool,
}

impl<I: ComponentInvoker + 'static, E: EventSink + Clone + 'static> ReactorCoordinator<I, E> {
    /// Create a new coordinator.
    ///
    /// # COLD PATH
    pub fn new(
        command_rx: mpsc::Receiver<ReactorCommand>,
        event_tx: mpsc::Sender<ReactorEvent>,
        invoker: Arc<I>,
        event_sink: Arc<E>,
    ) -> Self {
        Self {
            command_rx,
            event_tx,
            flows: HashMap::new(),
            next_flow_id: AtomicU64::new(1),
            _invoker: invoker,
            _event_sink: event_sink,
            shutting_down: false,
        }
    }

    /// Run the coordinator event loop.
    ///
    /// This method should be spawned as a Tokio task.
    pub async fn run(mut self) {
        info!("reactor coordinator started");

        loop {
            tokio::select! {
                cmd = self.command_rx.recv() => {
                    match cmd {
                        Some(command) => {
                            self.handle_command(command).await;
                        }
                        None => {
                            // All senders dropped; shut down.
                            info!("reactor coordinator: all handles dropped, shutting down");
                            break;
                        }
                    }
                }
            }

            // Reap completed flows.
            self.reap_completed_flows().await;
        }

        info!("reactor coordinator stopped");
    }

    /// Handle a single command.
    async fn handle_command(&mut self, command: ReactorCommand) {
        match command {
            ReactorCommand::CreateFlow(config, reply) => {
                if self.shutting_down {
                    let _ = reply.send(Err(FlowCreationError::ReactorShuttingDown));
                    return;
                }

                let result = self.create_flow(config).await;
                let _ = reply.send(result);
            }
            ReactorCommand::CancelFlow(flow_id, reason, reply) => {
                let result = self.cancel_flow(flow_id, reason);
                let _ = reply.send(result);
            }
            ReactorCommand::QueryFlowState(flow_id, reply) => {
                let result = match self.flows.get(&flow_id) {
                    Some(entry) => Ok(entry.handle.state),
                    None => Err(FlowError::Internal(format!("flow {flow_id} not found"))),
                };
                let _ = reply.send(result);
            }
            ReactorCommand::ListFlows(reply) => {
                let list: Vec<_> = self
                    .flows
                    .iter()
                    .map(|(id, entry)| (*id, entry.handle.state))
                    .collect();
                let _ = reply.send(list);
            }
            ReactorCommand::UpdatePriority(_flow_id, _priority, reply) => {
                // Phase 1: dynamic priority updates.
                let _ = reply.send(Err(FlowError::Internal(
                    "dynamic priority update not yet implemented".into(),
                )));
            }
            ReactorCommand::Shutdown(timeout, reply) => {
                let result = self.shutdown(timeout).await;
                let _ = reply.send(result);
            }
        }
    }

    /// Create and start a new flow.
    ///
    /// # COLD PATH
    async fn create_flow(&mut self, config: FlowConfig) -> Result<FlowId, FlowCreationError> {
        // Validate topology.
        config.topology.validate()?;

        // Assign flow ID.
        let flow_id = FlowId::new(self.next_flow_id.fetch_add(1, Ordering::Relaxed));

        // Build streams from connections.
        let _streams: Vec<StreamState> = config
            .topology
            .connections
            .iter()
            .enumerate()
            .map(|(idx, conn)| {
                let capacity = conn
                    .config
                    .capacity
                    .unwrap_or(config.default_queue_capacity);
                let policy = conn
                    .config
                    .backpressure_policy
                    .unwrap_or(config.default_backpressure_policy);
                let low_wm = conn
                    .config
                    .low_watermark_ratio
                    .unwrap_or(config.default_low_watermark_ratio);

                StreamState::new(
                    StreamId::new(idx as u64),
                    flow_id,
                    config.topology.stages[conn.from_stage].component_id,
                    config.topology.stages[conn.to_stage].component_id,
                    capacity,
                    policy,
                    low_wm,
                )
            })
            .collect();

        // Placeholder: in production, instances come from the host runtime.
        // For now, this is a placeholder for the integration point.
        // CROSS-CRATE DEPENDENCY: requires instantiated ComponentInstances
        // from the host runtime's WasmEngine.
        let _instances: Vec<ComponentInstance> = Vec::new();

        // NOTE: In real usage, `instances` would be populated by the host
        // before calling create_flow. The coordinator should receive
        // pre-instantiated components.

        // Create cancellation token.
        let cancellation = FlowCancellation::new();

        // Create internal event channel for this flow driver.
        let _flow_event_tx = self.event_tx.clone();

        let handle = FlowDriverHandle {
            flow_id,
            cancellation: cancellation.clone(),
            state: FlowState::Instantiated,
        };

        // NOTE: Spawning the flow driver requires instances to be populated.
        // This is the integration point with the host runtime.
        // For now, we store the handle without spawning.
        // In a fully integrated system, we'd spawn here:
        //
        // let invoker_clone = Arc::clone(&self._invoker);
        // let event_sink_clone = (*Arc::clone(&self._event_sink)).clone();
        // let join_handle = tokio::spawn(async move {
        //     let driver = FlowDriver::new(
        //         flow_id, config, instances, streams,
        //         invoker_clone, event_sink_clone,
        //         driver_cancellation, flow_event_tx,
        //     );
        //     driver.run().await
        // });

        // Temporary: create a stub join handle that resolves immediately.
        let join_handle = tokio::spawn(async move {
            // Stub: real implementation runs FlowDriver::run().
            (
                flow_id,
                FlowState::Completed,
                FlowCompletionStats::new(Duration::ZERO),
            )
        });

        self.flows.insert(
            flow_id,
            FlowEntry {
                handle,
                join_handle,
            },
        );

        info!(flow_id = %flow_id, "flow created");
        Ok(flow_id)
    }

    /// Cancel a running flow.
    fn cancel_flow(
        &mut self,
        flow_id: FlowId,
        reason: CancellationReason,
    ) -> Result<(), FlowError> {
        match self.flows.get_mut(&flow_id) {
            Some(entry) => {
                entry.handle.cancellation.cancel(reason);
                Ok(())
            }
            None => Err(FlowError::Internal(format!("flow {flow_id} not found"))),
        }
    }

    /// Reap completed flow driver tasks.
    async fn reap_completed_flows(&mut self) {
        let mut completed_ids = Vec::new();
        for (flow_id, entry) in &self.flows {
            if entry.join_handle.is_finished() {
                completed_ids.push(*flow_id);
            }
        }
        for flow_id in completed_ids {
            if let Some(entry) = self.flows.remove(&flow_id) {
                match entry.join_handle.await {
                    Ok((_, state, _stats)) => {
                        debug!(flow_id = %flow_id, state = %state, "flow reaped");
                    }
                    Err(e) => {
                        error!(flow_id = %flow_id, error = %e, "flow task panicked");
                    }
                }
            }
        }
    }

    /// Gracefully shut down all flows.
    async fn shutdown(&mut self, timeout: Duration) -> ShutdownResult {
        self.shutting_down = true;
        let mut result = ShutdownResult {
            completed: 0,
            cancelled: 0,
            timed_out: 0,
        };

        // Cancel all active flows.
        for entry in self.flows.values() {
            entry
                .handle
                .cancellation
                .cancel(CancellationReason::OperatorRequest);
        }

        // Wait for all flows to complete within timeout.
        let deadline = Instant::now() + timeout;
        while !self.flows.is_empty() && Instant::now() < deadline {
            self.reap_completed_flows().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Count remaining (timed out) flows.
        result.timed_out = self.flows.len();
        result.cancelled = 0; // Simplified for Phase 0
        result.completed = 0;

        info!(result = %result, "reactor shutdown complete");
        result
    }
}
