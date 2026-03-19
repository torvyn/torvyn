//! Runtime inspection API.
//!
//! Per Doc 02, Section 10.7: used by `torvyn inspect` CLI commands
//! and the diagnostic HTTP/Unix-socket API.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use torvyn_types::{FlowId, FlowState};

use crate::host::FlowRecord;

// ---------------------------------------------------------------------------
// FlowSummary
// ---------------------------------------------------------------------------

/// Summary of a flow for inspection.
///
/// # Examples
/// ```
/// use torvyn_host::FlowSummary;
/// use torvyn_types::{FlowId, FlowState};
///
/// let summary = FlowSummary {
///     flow_id: FlowId::new(1),
///     name: "my-pipeline".into(),
///     state: FlowState::Running,
/// };
/// assert_eq!(summary.name, "my-pipeline");
/// ```
#[derive(Debug, Clone)]
pub struct FlowSummary {
    /// The flow identifier.
    pub flow_id: FlowId,
    /// Human-readable pipeline name.
    pub name: String,
    /// Current flow state.
    pub state: FlowState,
}

impl From<&FlowRecord> for FlowSummary {
    fn from(record: &FlowRecord) -> Self {
        Self {
            flow_id: record.flow_id,
            name: record.name.clone(),
            state: record.state,
        }
    }
}

// ---------------------------------------------------------------------------
// InspectionHandle
// ---------------------------------------------------------------------------

/// Handle for querying runtime state.
///
/// Per Doc 02, Section 10.7. Used by `torvyn inspect` and the diagnostic
/// HTTP/Unix-socket API.
///
/// The handle is `Clone`-able and `Send + Sync`. It reads from the host's
/// flow registry and (in the full implementation) queries the reactor
/// for detailed flow state.
///
/// # Examples
/// ```no_run
/// use torvyn_host::InspectionHandle;
///
/// # async fn example(handle: InspectionHandle) {
/// let flows = handle.list_flows().await;
/// for flow in &flows {
///     println!("{}: {} ({:?})", flow.flow_id, flow.name, flow.state);
/// }
/// # }
/// ```
#[derive(Clone)]
pub struct InspectionHandle {
    flows: Arc<RwLock<HashMap<FlowId, FlowRecord>>>,
    // CROSS-CRATE DEPENDENCY: ReactorHandle for detailed queries.
    // reactor: ReactorHandle,
}

impl InspectionHandle {
    /// Create a new inspection handle.
    ///
    /// # COLD PATH
    pub(crate) fn new(
        flows: Arc<RwLock<HashMap<FlowId, FlowRecord>>>,
        // reactor: ReactorHandle,
    ) -> Self {
        Self {
            flows,
            // reactor,
        }
    }

    /// List all active flows.
    ///
    /// # COLD PATH
    pub async fn list_flows(&self) -> Vec<FlowSummary> {
        self.flows
            .read()
            .await
            .values()
            .map(FlowSummary::from)
            .collect()
    }

    /// Get the state of a specific flow.
    ///
    /// # COLD PATH
    pub async fn get_flow(&self, flow_id: FlowId) -> Option<FlowSummary> {
        self.flows.read().await.get(&flow_id).map(FlowSummary::from)
    }

    /// Get detailed queue depths for all streams in a flow.
    ///
    /// # COLD PATH
    ///
    /// CROSS-CRATE DEPENDENCY: Requires `ReactorHandle` to query
    /// stream queue depths. Returns empty vec until reactor integration.
    #[allow(clippy::unused_async)] // Will use await when reactor integration is enabled
    pub async fn get_queue_depths(&self, _flow_id: FlowId) -> Vec<(torvyn_types::StreamId, usize)> {
        // CROSS-CRATE DEPENDENCY: reactor.list_flows() + per-flow query
        Vec::new()
    }
}

impl std::fmt::Debug for InspectionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionHandle").finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_handle() -> (InspectionHandle, Arc<RwLock<HashMap<FlowId, FlowRecord>>>) {
        let flows = Arc::new(RwLock::new(HashMap::new()));
        let handle = InspectionHandle::new(flows.clone());
        (handle, flows)
    }

    #[tokio::test]
    async fn test_inspection_list_flows_empty() {
        let (handle, _) = make_test_handle();
        let flows = handle.list_flows().await;
        assert!(flows.is_empty());
    }

    #[tokio::test]
    async fn test_inspection_list_flows_with_data() {
        let (handle, flows_store) = make_test_handle();
        {
            let mut guard = flows_store.write().await;
            guard.insert(
                FlowId::new(1),
                FlowRecord {
                    flow_id: FlowId::new(1),
                    name: "alpha".into(),
                    state: FlowState::Running,
                },
            );
            guard.insert(
                FlowId::new(2),
                FlowRecord {
                    flow_id: FlowId::new(2),
                    name: "beta".into(),
                    state: FlowState::Completed,
                },
            );
        }

        let listed = handle.list_flows().await;
        assert_eq!(listed.len(), 2);
    }

    #[tokio::test]
    async fn test_inspection_get_flow_found() {
        let (handle, flows_store) = make_test_handle();
        {
            let mut guard = flows_store.write().await;
            guard.insert(
                FlowId::new(42),
                FlowRecord {
                    flow_id: FlowId::new(42),
                    name: "target".into(),
                    state: FlowState::Running,
                },
            );
        }

        let summary = handle.get_flow(FlowId::new(42)).await;
        assert!(summary.is_some());
        let s = summary.unwrap();
        assert_eq!(s.name, "target");
        assert_eq!(s.state, FlowState::Running);
    }

    #[tokio::test]
    async fn test_inspection_get_flow_not_found() {
        let (handle, _) = make_test_handle();
        let summary = handle.get_flow(FlowId::new(999)).await;
        assert!(summary.is_none());
    }

    #[tokio::test]
    async fn test_inspection_get_queue_depths_empty_stub() {
        let (handle, _) = make_test_handle();
        let depths = handle.get_queue_depths(FlowId::new(1)).await;
        assert!(depths.is_empty());
    }

    #[test]
    fn test_inspection_handle_is_clone() {
        let flows = Arc::new(RwLock::new(HashMap::new()));
        let handle = InspectionHandle::new(flows);
        let _clone = handle.clone();
    }

    #[tokio::test]
    async fn test_flow_summary_from_record() {
        let record = FlowRecord {
            flow_id: FlowId::new(7),
            name: "test-flow".into(),
            state: FlowState::Draining,
        };
        let summary = FlowSummary::from(&record);
        assert_eq!(summary.flow_id, FlowId::new(7));
        assert_eq!(summary.name, "test-flow");
        assert_eq!(summary.state, FlowState::Draining);
    }
}
