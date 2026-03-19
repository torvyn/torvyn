//! [`TorvynHost`]: the central orchestrator for the Torvyn runtime.
//!
//! Holds Arc references to all subsystem handles. Provides the public API
//! for flow management, runtime lifecycle, and inspection.
//!
//! **This is a thin orchestration shell.** All complex logic lives in
//! the subsystem crates (reactor, pipeline, engine, resources, etc.).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use torvyn_engine::WasmtimeEngine;
use torvyn_types::{FlowId, FlowState};

use crate::builder::HostConfig;
use crate::error::HostError;
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
    #[allow(dead_code)]
    engine: Arc<WasmtimeEngine>,

    /// Reactor handle for creating and managing flows.
    // CROSS-CRATE DEPENDENCY: ReactorHandle from torvyn-reactor.
    // reactor: ReactorHandle,

    /// Resource manager (shared via Arc).
    // CROSS-CRATE DEPENDENCY: ResourceManager from torvyn-resources.
    // resources: Arc<ResourceManager>,

    /// Security manager (shared via Arc).
    // CROSS-CRATE DEPENDENCY: SecurityManager from torvyn-security.
    // security: Arc<SecurityManager>,

    /// Observability collector.
    // CROSS-CRATE DEPENDENCY: ObservabilityCollector from torvyn-observability.
    // observability: ObservabilityCollector,

    /// Component invoker (shared via Arc for use by pipeline instantiation).
    // CROSS-CRATE DEPENDENCY: WasmtimeInvoker from torvyn-engine.
    // invoker: Arc<WasmtimeInvoker>,

    /// Active flow records. Protected by `RwLock` for concurrent inspection.
    flows: Arc<RwLock<HashMap<FlowId, FlowRecord>>>,

    /// Host lifecycle status.
    status: HostStatus,

    /// Monotonically increasing flow ID counter.
    next_flow_id: u64,
}

// LLI DEVIATION: Manual Debug impl because WasmtimeEngine does not derive Debug.
impl std::fmt::Debug for TorvynHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorvynHost")
            .field("config", &self.config)
            .field("status", &self.status)
            .field("next_flow_id", &self.next_flow_id)
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
        // reactor: ReactorHandle,
        // resources: Arc<ResourceManager>,
        // security: Arc<SecurityManager>,
        // observability: ObservabilityCollector,
        // invoker: Arc<WasmtimeInvoker>,
    ) -> Self {
        Self {
            config,
            engine,
            // reactor,
            // resources,
            // security,
            // observability,
            // invoker,
            flows: Arc::new(RwLock::new(HashMap::new())),
            status: HostStatus::Ready,
            next_flow_id: 1,
        }
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

        let flow_id = FlowId::new(self.next_flow_id);
        self.next_flow_id += 1;

        info!(flow_id = %flow_id, flow_name = flow_name, "Starting flow");

        // Delegate to the startup module for the full sequence.
        // CROSS-CRATE DEPENDENCY: startup::execute_flow_startup
        // uses torvyn-config, torvyn-contracts, torvyn-linker,
        // torvyn-engine, torvyn-pipeline, torvyn-reactor, torvyn-security.
        //
        // crate::startup::execute_flow_startup(
        //     flow_name,
        //     flow_id,
        //     &self.config,
        //     &self.engine,
        //     &self.invoker,
        //     &self.reactor,
        //     &self.resources,
        //     &self.security,
        // ).await?;

        // Record the flow
        let record = FlowRecord {
            flow_id,
            name: flow_name.to_owned(),
            state: FlowState::Running,
        };

        self.flows.write().await.insert(flow_id, record);
        self.status = HostStatus::Running;

        info!(flow_id = %flow_id, "Flow started successfully");

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

        // CROSS-CRATE DEPENDENCY: ReactorHandle::cancel_flow()
        // self.reactor.cancel_flow(
        //     flow_id,
        //     CancellationReason::OperatorRequested,
        // ).await.map_err(|e| FlowError::Reactor {
        //     detail: e.to_string(),
        // })?;

        // Update local record
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
        let flows = self.flows.read().await;
        flows
            .get(&flow_id)
            .map(|r| r.state)
            .ok_or_else(|| HostError::flow_not_found(flow_id))
    }

    /// List all active flows.
    ///
    /// # COLD PATH
    pub async fn list_flows(&self) -> Vec<FlowRecord> {
        self.flows.read().await.values().cloned().collect()
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

        // Step 1: Start all flows from configuration
        if let Some(ref pipeline_path) = self.config.pipeline_config_path {
            // CROSS-CRATE DEPENDENCY: Parse flow definitions from config.
            // let flow_defs = torvyn_config::parse_flow_definitions(pipeline_path)
            //     .map_err(|e| HostError::config(format!(
            //         "Failed to parse pipeline config '{}': {e}",
            //         pipeline_path.display()
            //     )))?;
            //
            // for flow_def in &flow_defs {
            //     self.start_flow(&flow_def.name).await?;
            // }

            info!(
                pipeline_config = %pipeline_path.display(),
                "Pipeline configuration loaded"
            );
        }

        let flow_count = self.flows.read().await.len();
        info!(
            flow_count = flow_count,
            "Torvyn host started — {} flow(s) active", flow_count
        );

        // Step 2: Wait for completion or shutdown signal
        #[cfg(feature = "signal")]
        {
            crate::signal::wait_for_shutdown_signal().await;
            info!("Shutdown signal received");
        }

        #[cfg(not(feature = "signal"))]
        {
            // Without signal support, wait for all flows to complete.
            // CROSS-CRATE DEPENDENCY: monitor flow states via reactor.
            // self.wait_for_all_flows().await;
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

        let outcome = crate::shutdown::graceful_shutdown(
            &self.flows,
            // &self.reactor,
            // &self.observability,
            timeout,
        )
        .await;

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
    use torvyn_engine::WasmtimeEngineConfig;

    fn make_test_host() -> TorvynHost {
        let config = HostConfig::default();
        let engine = Arc::new(WasmtimeEngine::new(WasmtimeEngineConfig::default()).unwrap());
        TorvynHost::new(config, engine)
    }

    #[test]
    fn test_host_initial_status() {
        let host = make_test_host();
        assert_eq!(host.status(), HostStatus::Ready);
    }

    #[tokio::test]
    async fn test_host_list_flows_empty() {
        let host = make_test_host();
        let flows = host.list_flows().await;
        assert!(flows.is_empty());
    }

    #[tokio::test]
    async fn test_host_flow_state_not_found() {
        let host = make_test_host();
        let result = host.flow_state(FlowId::new(999)).await;
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("E0920"));
    }

    #[tokio::test]
    async fn test_host_cancel_flow_not_found() {
        let host = make_test_host();
        let result = host.cancel_flow(FlowId::new(999)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_host_start_flow() {
        let mut host = make_test_host();
        let flow_id = host.start_flow("test-pipeline").await.unwrap();
        assert_eq!(flow_id, FlowId::new(1));
        assert_eq!(host.status(), HostStatus::Running);

        let state = host.flow_state(flow_id).await.unwrap();
        assert_eq!(state, FlowState::Running);
    }

    #[tokio::test]
    async fn test_host_start_multiple_flows() {
        let mut host = make_test_host();
        let id1 = host.start_flow("flow-1").await.unwrap();
        let id2 = host.start_flow("flow-2").await.unwrap();
        assert_ne!(id1, id2);

        let flows = host.list_flows().await;
        assert_eq!(flows.len(), 2);
    }

    #[tokio::test]
    async fn test_host_cancel_flow() {
        let mut host = make_test_host();
        let flow_id = host.start_flow("cancel-me").await.unwrap();

        host.cancel_flow(flow_id).await.unwrap();
        let state = host.flow_state(flow_id).await.unwrap();
        assert_eq!(state, FlowState::Cancelled);
    }

    #[tokio::test]
    async fn test_host_shutdown_when_no_flows() {
        let mut host = make_test_host();
        let outcome = host.shutdown().await.unwrap();
        assert_eq!(outcome.completed, 0);
        assert_eq!(outcome.cancelled, 0);
        assert_eq!(outcome.timed_out, 0);
        assert_eq!(host.status(), HostStatus::Stopped);
    }

    #[tokio::test]
    async fn test_host_shutdown_idempotent() {
        let mut host = make_test_host();
        let _ = host.shutdown().await.unwrap();
        let outcome = host.shutdown().await.unwrap();
        assert_eq!(outcome, ShutdownOutcome::already_stopped());
    }

    #[tokio::test]
    async fn test_host_start_flow_after_shutdown_fails() {
        let mut host = make_test_host();
        let _ = host.shutdown().await.unwrap();
        let result = host.start_flow("test").await;
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("shutting down"));
    }

    #[tokio::test]
    async fn test_host_inspection_handle() {
        let mut host = make_test_host();
        let _ = host.start_flow("inspectable").await.unwrap();

        let handle = host.inspection_handle();
        let flows = handle.list_flows().await;
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].name, "inspectable");
    }
}
