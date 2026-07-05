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

use torvyn_types::{ComponentId, EventSink, FlowId, FlowState, StreamId};

use torvyn_engine::{ComponentInstance, ComponentInvoker};
use torvyn_resources::DefaultResourceManager;

use crate::cancellation::{CancellationReason, FlowCancellation};
use crate::config::FlowConfig;
use crate::error::{FlowCreationError, FlowError};
use crate::events::{ReactorCommand, ReactorEvent, ShutdownResult};
use crate::flow_driver::{FlowDriver, FlowDriverHandle};
use crate::metrics::FlowCompletionStats;
use crate::stream::StreamState;

/// Internal state for a managed flow.
struct FlowEntry {
    handle: FlowDriverHandle,
    join_handle: JoinHandle<(FlowId, FlowState, FlowCompletionStats)>,
    /// Component identities of this flow's stages, captured from the topology
    /// at spawn time. Used to reclaim each component's host-managed resources
    /// when the flow reaches a terminal state.
    component_ids: Vec<ComponentId>,
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
    /// The component invoker, shared across flow drivers via `Arc::clone`.
    invoker: Arc<I>,
    /// The event sink for observability. Cloned into each flow driver.
    event_sink: Arc<E>,
    /// Shared resource manager. When a flow reaches a terminal state the
    /// coordinator returns the flow's buffers, budget, and resource bookkeeping
    /// here (retaining its copy-ledger stats for post-mortem observability), so
    /// completed flows do not accumulate host-managed resources for the
    /// engine's lifetime.
    resources: Arc<DefaultResourceManager>,
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
        resources: Arc<DefaultResourceManager>,
    ) -> Self {
        Self {
            command_rx,
            event_tx,
            flows: HashMap::new(),
            next_flow_id: AtomicU64::new(1),
            invoker,
            event_sink,
            resources,
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
            ReactorCommand::SpawnFlow {
                config,
                instances,
                reply,
            } => {
                if self.shutting_down {
                    let _ = reply.send(Err(FlowCreationError::ReactorShuttingDown));
                    return;
                }
                let result = self.spawn_flow_with_instances(config, instances);
                let _ = reply.send(result);
            }
        }
    }

    /// Spawn a fully-instantiated flow. Builds streams from the topology,
    /// constructs a real `FlowDriver`, spawns its `run()` on a Tokio task,
    /// and stores the join handle.
    ///
    /// # COLD PATH — called once per `ReactorCommand::SpawnFlow`.
    ///
    /// # Errors
    /// - [`FlowCreationError::InvalidTopology`] if the topology fails
    ///   validation or `instances.len()` does not match the stage count.
    fn spawn_flow_with_instances(
        &mut self,
        config: FlowConfig,
        mut instances: Vec<ComponentInstance>,
    ) -> Result<FlowId, FlowCreationError> {
        // 1. Validate topology.
        config.topology.validate()?;

        // 2. Validate instance/stage parity.
        let stage_count = config.topology.stages.len();
        if instances.len() != stage_count {
            return Err(FlowCreationError::InvalidTopology(format!(
                "instance count ({}) does not match topology stage count ({})",
                instances.len(),
                stage_count,
            )));
        }

        // 3. Assign flow ID.
        let flow_id = FlowId::new(self.next_flow_id.fetch_add(1, Ordering::Relaxed));

        // Capture the stage component identities before `config` is moved into
        // the driver task, so the flow's host-managed resources can be
        // reclaimed by component when it reaches a terminal state.
        let component_ids: Vec<ComponentId> = config
            .topology
            .stages
            .iter()
            .map(|stage| stage.component_id)
            .collect();

        // Stamp the reactor-assigned flow id onto every component's store and
        // register the flow with the resource manager's copy ledger, both
        // *before* the driver is spawned. At instantiation each store carries
        // the unassigned sentinel; without this, host-side copy accounting
        // would be attributed to the wrong flow (or dropped). Registration must
        // precede any resource operation so the ledger has a live entry.
        for instance in &mut instances {
            instance.set_flow_id(flow_id);
        }
        self.resources.register_flow(flow_id);

        // Pre-register the flow with the observability sink *before* the driver
        // is spawned, so no `record_*` call emitted by the driver can race ahead
        // of its metric allocation. The stream ids mirror the indices assigned
        // to connections below (and in `FlowDriver::make_streams`). The sink is
        // a no-op when observability is disabled.
        let stream_ids: Vec<StreamId> = (0..config.topology.connections.len() as u64)
            .map(StreamId::new)
            .collect();
        self.event_sink
            .on_flow_start(flow_id, &component_ids, &stream_ids);

        // 4. Build streams from connections.
        let streams: Vec<StreamState> = config
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

        // 5. Cancellation token. The handle keeps a clone; the driver
        //    receives the original which it threads into its select! loop.
        let cancellation = FlowCancellation::new();
        let handle_cancellation = cancellation.clone();

        let handle = FlowDriverHandle {
            flow_id,
            cancellation: handle_cancellation,
            state: FlowState::Instantiated,
        };

        // 6. Clone the shared subsystems for the driver.
        // `Arc<I>: ComponentInvoker` via the blanket impl in torvyn-engine.
        let invoker_for_driver: Arc<I> = Arc::clone(&self.invoker);
        let event_sink_for_driver: E = (*self.event_sink).clone();
        let event_tx_for_driver = self.event_tx.clone();

        // 7. Spawn the flow driver task. The driver consumes config,
        //    instances, streams, and cancellation; `Arc<I>` and the event
        //    sink clone keep the host's shared subsystems alive.
        let join_handle = tokio::spawn(async move {
            let driver = FlowDriver::new(
                flow_id,
                config,
                instances,
                streams,
                invoker_for_driver,
                event_sink_for_driver,
                cancellation,
                event_tx_for_driver,
            );
            driver.run().await
        });

        self.flows.insert(
            flow_id,
            FlowEntry {
                handle,
                join_handle,
                component_ids,
            },
        );

        info!(
            flow_id = %flow_id,
            stages = stage_count,
            "flow spawned"
        );
        Ok(flow_id)
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

        let component_ids: Vec<ComponentId> = config
            .topology
            .stages
            .iter()
            .map(|stage| stage.component_id)
            .collect();

        self.flows.insert(
            flow_id,
            FlowEntry {
                handle,
                join_handle,
                component_ids,
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
                let FlowEntry {
                    join_handle,
                    component_ids,
                    ..
                } = entry;
                match join_handle.await {
                    Ok((_, state, _stats)) => {
                        debug!(flow_id = %flow_id, state = %state, "flow reaped");
                    }
                    Err(e) => {
                        error!(flow_id = %flow_id, error = %e, "flow task panicked");
                    }
                }
                // The driver task has fully terminated (and already emitted its
                // `FlowCompleted` event with stats), so any buffers the flow's
                // components still held are now orphaned in the resource
                // manager. Reclaim them and release the components' budget; the
                // copy-ledger entries are retained for post-terminal
                // observability.
                self.reclaim_flow_resources(flow_id, &component_ids);
            }
        }
    }

    /// Reclaim a terminal flow's host-managed resources from the shared
    /// resource manager by reclaiming each of its components' outstanding
    /// buffers and releasing their budget, while leaving the copy-ledger stats
    /// in place for post-terminal observability.
    ///
    /// Keyed by component identity, which is stable regardless of how flow
    /// identifiers are derived, so this stays correct as the resource/flow-id
    /// wiring evolves. Reclaiming a component whose buffers were already
    /// returned during normal operation is a no-op, so this is safe to call for
    /// every reaped flow.
    ///
    /// # COLD PATH — called once per flow when it reaches a terminal state.
    fn reclaim_flow_resources(&self, flow_id: FlowId, component_ids: &[ComponentId]) {
        let mut buffers = 0usize;
        let mut bytes = 0u64;
        for &component_id in component_ids {
            for reclaimed in self.resources.force_reclaim(component_id) {
                buffers += 1;
                bytes += u64::from(reclaimed.payload_capacity);
            }
        }
        if buffers > 0 {
            debug!(
                flow_id = %flow_id,
                buffers,
                bytes,
                "reclaimed orphaned component buffers at flow terminal",
            );
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
