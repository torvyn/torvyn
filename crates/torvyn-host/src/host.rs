//! [`TorvynHost`]: the central orchestrator for the Torvyn runtime.
//!
//! Holds Arc references to all subsystem handles. Provides the public API
//! for flow management, runtime lifecycle, and inspection.
//!
//! **This is a thin orchestration shell.** All complex logic lives in
//! the subsystem crates (reactor, pipeline, engine, resources, etc.).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use torvyn_config::FlowDef;
use torvyn_engine::{WasmtimeEngine, WasmtimeInvoker};
use torvyn_observability::ObservabilityCollector;
use torvyn_pipeline::{flow_def_to_topology, instantiate_pipeline};
use torvyn_reactor::{cancellation::CancellationReason, ReactorHandle};
use torvyn_types::{FlowId, FlowState};

use crate::builder::HostConfig;
use crate::error::{HostError, StartupError, StartupStage};
use crate::inspection::InspectionHandle;
use crate::shutdown::ShutdownOutcome;

// ---------------------------------------------------------------------------
// FlowRecord
// ---------------------------------------------------------------------------

/// Record of an active or completed flow.
///
/// Tracks the flow's identity, name, and current state. The actual
/// flow execution state is managed by the reactor — this record is
/// the host's bookkeeping layer.
///
/// # Invariants
/// - `flow_id` is unique within the host.
/// - `state` is kept in sync with the reactor's flow state.
#[derive(Debug, Clone)]
pub struct FlowRecord {
    /// The flow identifier from the reactor.
    pub flow_id: FlowId,

    /// Human-readable pipeline name.
    pub name: String,

    /// Current flow state (cached from reactor queries).
    pub state: FlowState,
}

// ---------------------------------------------------------------------------
// HostStatus
// ---------------------------------------------------------------------------

/// The lifecycle state of the host itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStatus {
    /// The host is constructed but not yet running flows.
    Ready,

    /// The host has started and is processing flows.
    Running,

    /// The host is shutting down (draining flows).
    ShuttingDown,

    /// The host has shut down completely.
    Stopped,
}

// ---------------------------------------------------------------------------
// TorvynHost
// ---------------------------------------------------------------------------

/// The Torvyn runtime host.
///
/// Per Doc 02, Section 10.3: owns all subsystems and manages their lifecycle.
/// The host is a thin orchestration shell — it delegates to subsystem crates
/// for all complex logic.
///
/// # Thread Safety
/// `TorvynHost` is `Send` but not `Sync`. It is owned by a single async
/// task (the main runtime loop). Flow management state is behind `RwLock`
/// for inspection access from other tasks.
///
/// # Examples
/// ```no_run
/// use torvyn_host::HostBuilder;
///
/// # async fn example() -> Result<(), torvyn_host::HostError> {
/// let mut host = HostBuilder::new()
///     .with_config_file("Torvyn.toml")
///     .build()
///     .await?;
///
/// host.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct TorvynHost {
    /// Aggregated host configuration.
    config: HostConfig,

    /// Wasm engine (shared via Arc for use by linker and invoker).
    engine: Arc<WasmtimeEngine>,

    /// Component invoker, shared with the reactor coordinator and (via
    /// `Arc::clone`) with each flow driver during flow spawning.
    invoker: Arc<WasmtimeInvoker>,

    /// Reactor handle for creating and managing flows. Sends commands to
    /// the coordinator task spawned during `HostBuilder::build`.
    reactor: ReactorHandle,

    /// Join handle for the reactor coordinator task. Held for the
    /// lifetime of the host so the coordinator stays alive; on host drop,
    /// the reactor's `command_tx` closes and the coordinator exits.
    /// Marked `_` because we do not await it directly — Tokio runtime
    /// teardown reaps the task on drop.
    _coordinator_join: Option<JoinHandle<()>>,

    /// Active flow records. Protected by `RwLock` for concurrent inspection.
    flows: Arc<RwLock<HashMap<FlowId, FlowRecord>>>,

    /// Host lifecycle status.
    status: HostStatus,

    /// Flow definitions available to start, keyed by name. Loaded from the
    /// pipeline configuration and/or registered programmatically via the
    /// builder.
    flow_defs: BTreeMap<String, FlowDef>,

    /// Observability collector wired into the reactor as its event sink.
    /// Records per-flow invocations, latencies, throughput, and errors as
    /// flows run, and is the handle through which metrics are inspected.
    /// Shared (`Arc`) with the reactor coordinator and every flow driver.
    observability: Arc<ObservabilityCollector>,
}

// LLI DEVIATION: Manual Debug impl because WasmtimeEngine does not derive Debug.
impl std::fmt::Debug for TorvynHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorvynHost")
            .field("config", &self.config)
            .field("status", &self.status)
            .field("flow_defs", &self.flow_defs.keys().collect::<Vec<_>>())
            .field("reactor", &self.reactor)
            .finish_non_exhaustive()
    }
}

impl TorvynHost {
    /// Construct a new host. Called by `HostBuilder::build()`.
    ///
    /// # COLD PATH
    ///
    /// # Preconditions
    /// - `config` has been validated.
    /// - `engine` is initialized and ready.
    /// - All subsystem handles (reactor, resources, security, observability)
    ///   are initialized. Currently commented out pending cross-crate integration.
    pub(crate) fn new(
        config: HostConfig,
        engine: Arc<WasmtimeEngine>,
        invoker: Arc<WasmtimeInvoker>,
        reactor: ReactorHandle,
        coordinator_join: Option<JoinHandle<()>>,
        flow_defs: BTreeMap<String, FlowDef>,
        observability: Arc<ObservabilityCollector>,
    ) -> Self {
        Self {
            config,
            engine,
            invoker,
            reactor,
            _coordinator_join: coordinator_join,
            flows: Arc::new(RwLock::new(HashMap::new())),
            status: HostStatus::Ready,
            flow_defs,
            observability,
        }
    }

    /// Returns a reference to the shared Wasm engine.
    ///
    /// # COLD PATH — used by `instantiate_pipeline` callers and
    /// inspection tools.
    #[inline]
    #[must_use]
    pub fn engine(&self) -> &Arc<WasmtimeEngine> {
        &self.engine
    }

    /// Returns a reference to the shared component invoker.
    ///
    /// # COLD PATH
    #[inline]
    #[must_use]
    pub fn invoker(&self) -> &Arc<WasmtimeInvoker> {
        &self.invoker
    }

    /// Returns a reference to the reactor handle for flow management.
    ///
    /// # COLD PATH
    #[inline]
    #[must_use]
    pub fn reactor(&self) -> &ReactorHandle {
        &self.reactor
    }

    /// Returns a reference to the observability collector wired into the
    /// reactor. Use it to read per-flow metrics (invocations, latency
    /// histograms, throughput, errors) recorded as flows run, or to take a
    /// metrics snapshot for a flow.
    ///
    /// # COLD PATH — inspection and reporting.
    #[inline]
    #[must_use]
    pub fn observability(&self) -> &Arc<ObservabilityCollector> {
        &self.observability
    }

    /// Start a flow from a pipeline definition.
    ///
    /// Executes the full startup sequence for a single flow:
    /// topology construction -> validation -> contract check -> linking ->
    /// compilation -> instantiation -> `lifecycle.init` -> reactor registration.
    ///
    /// # COLD PATH — called once per flow.
    ///
    /// # Errors
    /// Returns `HostError::Startup` with the specific stage and reason.
    /// Returns `HostError::Internal` if the host is shutting down.
    ///
    /// # Postconditions
    /// On success, the flow is registered with the reactor and actively
    /// processing. The returned `FlowId` can be used for inspection,
    /// cancellation, and shutdown.
    pub async fn start_flow(&mut self, flow_name: &str) -> Result<FlowId, HostError> {
        if self.status == HostStatus::ShuttingDown || self.status == HostStatus::Stopped {
            return Err(HostError::Internal(
                "Cannot start flow: host is shutting down".into(),
            ));
        }

        // Look up the flow definition by name.
        let flow_def = self.flow_defs.get(flow_name).ok_or_else(|| {
            HostError::config(format!(
                "No flow named '{flow_name}' is defined in the configuration"
            ))
        })?;

        info!(flow_name = flow_name, "Starting flow");

        // Build the validated topology, resolving each component's capability
        // grants into its WASI sandbox.
        let topology =
            flow_def_to_topology(flow_name, flow_def, &self.config.security).map_err(|errors| {
                StartupError::FlowStartup {
                    flow_name: flow_name.to_owned(),
                    stage: StartupStage::TopologyConstruction,
                    reason: errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                }
            })?;

        // Compile, instantiate, run `lifecycle.init`, and register the flow
        // with the reactor. The reactor assigns the canonical flow identifier.
        let handle = instantiate_pipeline(&topology, &*self.engine, &self.invoker, &self.reactor)
            .await
            .map_err(|e| StartupError::FlowStartup {
                flow_name: flow_name.to_owned(),
                stage: StartupStage::Instantiation,
                reason: e.to_string(),
            })?;

        let flow_id = handle.flow_id();
        let record = FlowRecord {
            flow_id,
            name: flow_name.to_owned(),
            state: FlowState::Running,
        };

        self.flows.write().await.insert(flow_id, record);
        self.status = HostStatus::Running;

        info!(flow_id = %flow_id, flow_name = flow_name, "Flow started successfully");

        Ok(flow_id)
    }

    /// Cancel a specific flow.
    ///
    /// Initiates graceful drain then termination for the specified flow.
    ///
    /// # COLD PATH
    ///
    /// # Errors
    /// Returns [`HostError::Flow`] with `FlowError::NotFound`
    /// if the flow does not exist.
    pub async fn cancel_flow(&self, flow_id: FlowId) -> Result<(), HostError> {
        let flows = self.flows.read().await;
        if !flows.contains_key(&flow_id) {
            return Err(HostError::flow_not_found(flow_id));
        }
        drop(flows);

        info!(flow_id = %flow_id, "Cancelling flow");

        // Request cancellation from the reactor, which drives the flow's
        // cooperative drain and termination.
        self.reactor
            .cancel_flow(flow_id, CancellationReason::OperatorRequest)
            .await
            .map_err(|e| {
                HostError::Flow(crate::error::FlowError::Reactor {
                    detail: e.to_string(),
                })
            })?;

        // Update the local record.
        let mut flows = self.flows.write().await;
        if let Some(record) = flows.get_mut(&flow_id) {
            record.state = FlowState::Cancelled;
        }

        Ok(())
    }

    /// Inspect the current state of a flow.
    ///
    /// # COLD PATH
    ///
    /// # Errors
    /// Returns [`HostError::Flow`] if the flow is not found.
    pub async fn flow_state(&self, flow_id: FlowId) -> Result<FlowState, HostError> {
        // The flow must be known to the host.
        let cached = {
            let flows = self.flows.read().await;
            flows.get(&flow_id).map(|r| r.state)
        }
        .ok_or_else(|| HostError::flow_not_found(flow_id))?;

        // Prefer the reactor's live state. If the reactor no longer tracks the
        // flow, it reached a terminal state and was reaped: report the cached
        // state if it is already terminal, otherwise treat it as completed.
        match self.reactor.flow_state(flow_id).await {
            Ok(state) => Ok(state),
            Err(_) if cached.is_terminal() => Ok(cached),
            Err(_) => Ok(FlowState::Completed),
        }
    }

    /// List all active flows.
    ///
    /// # COLD PATH
    pub async fn list_flows(&self) -> Vec<FlowRecord> {
        self.flows.read().await.values().cloned().collect()
    }

    /// Interval between reactor polls in [`wait_for_all_flows`](Self::wait_for_all_flows).
    ///
    /// Flow completion is a cold-path event (a pipeline draining), so a short
    /// fixed poll keeps the wait dependency-free without adding meaningful
    /// latency to shutdown.
    const FLOW_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(25);

    /// Wait until every active flow has reached a terminal state.
    ///
    /// The reactor drops a flow from its tracking table once the flow is reaped
    /// after termination, so a flow that is absent from
    /// [`ReactorHandle::list_flows`] has already finished. This returns once
    /// every still-tracked flow reports a terminal state — including
    /// immediately when no flows are active.
    ///
    /// Callers that drive a single flow (for example `torvyn run`) can start it
    /// with [`start_flow`](Self::start_flow) and await this to block until that
    /// flow finishes, typically racing it against a timeout or interrupt.
    ///
    /// # COLD PATH — the waiting is the point; hot-path work is in the reactor.
    pub async fn wait_for_all_flows(&self) {
        loop {
            let flows = self.reactor.list_flows().await;
            if flows.iter().all(|(_, state)| state.is_terminal()) {
                return;
            }
            tokio::time::sleep(Self::FLOW_COMPLETION_POLL_INTERVAL).await;
        }
    }

    /// Run the host until all flows complete, a shutdown signal is
    /// received, or an unrecoverable error occurs.
    ///
    /// This is the main runtime loop. It:
    /// 1. Starts all flows defined in the pipeline configuration.
    /// 2. Registers signal handlers (SIGINT, SIGTERM) if enabled.
    /// 3. Waits for completion or shutdown signal.
    /// 4. Executes graceful shutdown.
    ///
    /// # COLD PATH (the waiting is the point; hot-path processing is in the reactor).
    ///
    /// # Errors
    /// Returns `HostError` if startup fails or if shutdown times out.
    pub async fn run(&mut self) -> Result<(), HostError> {
        info!("Torvyn host starting");

        // Step 1: Start every flow defined in the configuration. Names are
        // collected first so the mutable `start_flow` borrow does not overlap
        // the `flow_defs` borrow.
        let flow_names: Vec<String> = self.flow_defs.keys().cloned().collect();
        for flow_name in &flow_names {
            self.start_flow(flow_name).await?;
        }

        let flow_count = self.flows.read().await.len();
        info!(
            flow_count = flow_count,
            "Torvyn host started — {} flow(s) active", flow_count
        );

        // Step 2: Run until every flow reaches a terminal state or a shutdown
        // signal arrives, whichever comes first. Without the `signal` feature
        // (embedded/library use), completion is the only exit.
        #[cfg(feature = "signal")]
        {
            tokio::select! {
                () = self.wait_for_all_flows() => {
                    info!("All flows reached a terminal state");
                }
                () = crate::signal::wait_for_shutdown_signal() => {
                    info!("Shutdown signal received");
                }
            }
        }

        #[cfg(not(feature = "signal"))]
        {
            self.wait_for_all_flows().await;
            info!("All flows reached a terminal state");
        }

        // Step 3: Graceful shutdown
        let outcome = self.shutdown().await?;

        info!(
            completed = outcome.completed,
            cancelled = outcome.cancelled,
            timed_out = outcome.timed_out,
            "Torvyn host stopped"
        );

        Ok(())
    }

    /// Initiate graceful shutdown of the entire host.
    ///
    /// # COLD PATH
    ///
    /// # Steps (per Doc 02, Section 8.2)
    /// 1. Set host status to `ShuttingDown`.
    /// 2. Signal the reactor to drain all flows.
    /// 3. Wait for completion up to `shutdown_timeout`.
    /// 4. Force-terminate any remaining flows.
    /// 5. Flush observability.
    /// 6. Set host status to Stopped.
    ///
    /// # Errors
    /// Returns `HostError::ShutdownTimeout` if graceful shutdown
    /// does not complete in time (but the host still terminates).
    pub async fn shutdown(&mut self) -> Result<ShutdownOutcome, HostError> {
        if self.status == HostStatus::Stopped {
            return Ok(ShutdownOutcome::already_stopped());
        }

        info!("Initiating graceful shutdown");
        self.status = HostStatus::ShuttingDown;

        let timeout = self.config.shutdown_timeout;

        // Drain the reactor: cancel and await all active flow drivers within
        // the timeout. The reactor is the source of truth for how flows
        // terminated.
        let reactor_result = self.reactor.shutdown(timeout).await;
        let outcome = ShutdownOutcome {
            completed: reactor_result.completed,
            cancelled: reactor_result.cancelled,
            timed_out: reactor_result.timed_out,
        };

        // Reflect the terminal disposition in the local flow records.
        {
            let mut flows = self.flows.write().await;
            for record in flows.values_mut() {
                if !record.state.is_terminal() {
                    record.state = FlowState::Cancelled;
                }
            }
        }

        self.status = HostStatus::Stopped;

        // Check if we timed out
        if outcome.timed_out > 0 {
            warn!(
                timed_out = outcome.timed_out,
                "Some flows did not drain within timeout"
            );
        }

        info!("Host shutdown complete");
        Ok(outcome)
    }

    /// Get a handle for runtime inspection (used by CLI and diagnostics).
    ///
    /// # COLD PATH
    #[must_use]
    pub fn inspection_handle(&self) -> InspectionHandle {
        InspectionHandle::new(
            self.flows.clone(),
            // self.reactor.clone(),
        )
    }

    /// Returns the current host status.
    #[inline]
    #[must_use]
    pub fn status(&self) -> HostStatus {
        self.status
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::HostBuilder;
    use torvyn_config::NodeDef;

    /// A host with a live reactor coordinator and no flow definitions.
    ///
    /// Unlike a hand-constructed host, this uses the real builder so the
    /// reactor commands issued by `start_flow`, `cancel_flow`, `flow_state`,
    /// and `shutdown` reach a running coordinator.
    async fn make_test_host() -> TorvynHost {
        HostBuilder::new()
            .build()
            .await
            .expect("host with default configuration must build")
    }

    /// A host pre-loaded with a single sink-only flow named `flow_name`.
    ///
    /// The topology is intentionally invalid (a sink with no source), so
    /// `start_flow` exercises the host → pipeline wiring and surfaces a
    /// startup error without requiring real Wasm components.
    async fn host_with_invalid_flow(flow_name: &str) -> TorvynHost {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "sink".to_owned(),
            NodeDef {
                component: "file:///nonexistent/sink.wasm".to_owned(),
                interface: "torvyn:streaming/sink".to_owned(),
                ..NodeDef::default()
            },
        );
        let flow = FlowDef {
            nodes,
            ..FlowDef::default()
        };
        HostBuilder::new()
            .with_flow_definition(flow_name, flow)
            .build()
            .await
            .expect("host must build")
    }

    #[tokio::test]
    async fn test_host_initial_status() {
        let host = make_test_host().await;
        assert_eq!(host.status(), HostStatus::Ready);
    }

    #[tokio::test]
    async fn test_host_list_flows_empty() {
        let host = make_test_host().await;
        assert!(host.list_flows().await.is_empty());
    }

    #[tokio::test]
    async fn test_host_flow_state_not_found() {
        let host = make_test_host().await;
        let err = host.flow_state(FlowId::new(999)).await.unwrap_err();
        assert!(format!("{err}").contains("E0920"));
    }

    #[tokio::test]
    async fn test_host_cancel_flow_not_found() {
        let host = make_test_host().await;
        assert!(host.cancel_flow(FlowId::new(999)).await.is_err());
    }

    #[tokio::test]
    async fn test_host_start_unknown_flow_rejected() {
        let mut host = make_test_host().await;
        let err = host.start_flow("does-not-exist").await.unwrap_err();
        assert!(format!("{err}").contains("No flow named"));
        // A failed start leaves no flow record behind.
        assert!(host.list_flows().await.is_empty());
    }

    #[tokio::test]
    async fn test_host_start_invalid_flow_surfaces_startup_error() {
        let mut host = host_with_invalid_flow("sink-only").await;
        // The flow is defined, so the host proceeds into the pipeline, where
        // topology construction fails (no source). The error propagates and no
        // flow record is created.
        assert!(host.start_flow("sink-only").await.is_err());
        assert!(host.list_flows().await.is_empty());
    }

    #[tokio::test]
    async fn test_host_start_flow_after_shutdown_rejected() {
        let mut host = make_test_host().await;
        let _ = host.shutdown().await.unwrap();
        let err = host.start_flow("anything").await.unwrap_err();
        assert!(format!("{err}").contains("shutting down"));
    }

    #[tokio::test]
    async fn test_host_shutdown_when_no_flows() {
        let mut host = make_test_host().await;
        let outcome = host.shutdown().await.unwrap();
        assert_eq!(outcome.completed, 0);
        assert_eq!(outcome.cancelled, 0);
        assert_eq!(outcome.timed_out, 0);
        assert_eq!(host.status(), HostStatus::Stopped);
    }

    #[tokio::test]
    async fn test_host_shutdown_idempotent() {
        let mut host = make_test_host().await;
        let _ = host.shutdown().await.unwrap();
        let outcome = host.shutdown().await.unwrap();
        assert_eq!(outcome, ShutdownOutcome::already_stopped());
    }

    #[tokio::test]
    async fn test_host_inspection_handle_empty() {
        let host = make_test_host().await;
        let handle = host.inspection_handle();
        assert!(handle.list_flows().await.is_empty());
    }

    #[tokio::test]
    async fn test_host_observability_collector_wired_at_production() {
        // The default configuration enables observability, so the host exposes
        // a collector recording at Production level.
        let host = make_test_host().await;
        assert_eq!(
            host.observability().current_level(),
            torvyn_types::ObservabilityLevel::Production,
        );
    }

    #[tokio::test]
    async fn test_host_observability_collector_off_when_disabled() {
        // Disabling both tracing and metrics collapses the collector to Off,
        // making recording a zero-cost no-op.
        let observability = torvyn_config::ObservabilityConfig {
            tracing_enabled: false,
            metrics_enabled: false,
            ..torvyn_config::ObservabilityConfig::default()
        };
        let host = HostBuilder::new()
            .with_observability_config(observability)
            .build()
            .await
            .expect("host must build with observability disabled");
        assert_eq!(
            host.observability().current_level(),
            torvyn_types::ObservabilityLevel::Off,
        );
    }

    #[tokio::test]
    async fn test_wait_for_all_flows_returns_immediately_when_no_flows() {
        let host = make_test_host().await;
        // With no active flows there is nothing to wait for; this must resolve
        // promptly rather than block.
        tokio::time::timeout(Duration::from_secs(2), host.wait_for_all_flows())
            .await
            .expect("wait_for_all_flows must return immediately when no flows are active");
    }

    #[tokio::test]
    async fn test_run_returns_when_no_flows_are_configured() {
        // `run()` starts every configured flow, waits for completion or a
        // shutdown signal, then shuts down. With no flows configured, the
        // completion path fires immediately, so `run()` must return without a
        // signal — previously it blocked forever on `wait_for_shutdown_signal`.
        let mut host = make_test_host().await;
        tokio::time::timeout(Duration::from_secs(5), host.run())
            .await
            .expect("run() must return promptly when there are no flows to wait for")
            .expect("run() must complete without error");
        assert_eq!(host.status(), HostStatus::Stopped);
    }
}
