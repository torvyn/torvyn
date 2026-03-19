//! Integration tests for the full host lifecycle.

use torvyn_host::{HostBuilder, HostStatus, ShutdownOutcome};
use torvyn_types::{FlowId, FlowState};

#[tokio::test]
async fn test_full_lifecycle_no_flows() {
    // Build → check ready → shutdown → check stopped
    let mut host = HostBuilder::new().build().await.unwrap();
    assert_eq!(host.status(), HostStatus::Ready);

    let outcome = host.shutdown().await.unwrap();
    assert_eq!(outcome, ShutdownOutcome::already_stopped());
    assert_eq!(host.status(), HostStatus::Stopped);
}

#[tokio::test]
async fn test_inspection_handle_works_across_tasks() {
    let host = HostBuilder::new().build().await.unwrap();
    let handle = host.inspection_handle();

    // Spawn a separate task that uses the inspection handle
    let task = tokio::spawn(async move {
        let flows = handle.list_flows().await;
        assert!(flows.is_empty());
    });

    task.await.unwrap();
}

#[tokio::test]
async fn test_flow_state_query_not_found() {
    let host = HostBuilder::new().build().await.unwrap();
    let result = host.flow_state(FlowId::new(42)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_double_shutdown_is_safe() {
    let mut host = HostBuilder::new().build().await.unwrap();

    let outcome1 = host.shutdown().await.unwrap();
    let outcome2 = host.shutdown().await.unwrap();

    assert_eq!(outcome1, ShutdownOutcome::already_stopped());
    assert_eq!(outcome2, ShutdownOutcome::already_stopped());
    assert_eq!(host.status(), HostStatus::Stopped);
}

#[tokio::test]
async fn test_start_flow_then_cancel() {
    let mut host = HostBuilder::new().build().await.unwrap();
    let flow_id = host.start_flow("test-pipeline").await.unwrap();

    assert_eq!(host.status(), HostStatus::Running);
    assert_eq!(host.flow_state(flow_id).await.unwrap(), FlowState::Running);

    host.cancel_flow(flow_id).await.unwrap();
    assert_eq!(
        host.flow_state(flow_id).await.unwrap(),
        FlowState::Cancelled
    );
}

#[tokio::test]
async fn test_start_flow_after_shutdown_rejected() {
    let mut host = HostBuilder::new().build().await.unwrap();
    host.shutdown().await.unwrap();

    let result = host.start_flow("late-flow").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_multiple_flows_listed() {
    let mut host = HostBuilder::new().build().await.unwrap();

    let id1 = host.start_flow("flow-alpha").await.unwrap();
    let id2 = host.start_flow("flow-beta").await.unwrap();
    let id3 = host.start_flow("flow-gamma").await.unwrap();

    let flows = host.list_flows().await;
    assert_eq!(flows.len(), 3);

    let ids: Vec<FlowId> = flows.iter().map(|f| f.flow_id).collect();
    assert!(ids.contains(&id1));
    assert!(ids.contains(&id2));
    assert!(ids.contains(&id3));
}

#[tokio::test]
async fn test_inspection_handle_reflects_flow_state() {
    let mut host = HostBuilder::new().build().await.unwrap();
    let flow_id = host.start_flow("observable").await.unwrap();

    let handle = host.inspection_handle();

    // From another task: query state
    let task = tokio::spawn(async move {
        let summary = handle.get_flow(flow_id).await;
        assert!(summary.is_some());
        let s = summary.unwrap();
        assert_eq!(s.name, "observable");
        assert_eq!(s.state, FlowState::Running);
    });

    task.await.unwrap();
}
