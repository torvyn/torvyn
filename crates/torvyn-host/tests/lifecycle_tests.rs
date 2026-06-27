//! Integration tests for the full host lifecycle.

use torvyn_host::{HostBuilder, HostStatus, ShutdownOutcome};
use torvyn_types::FlowId;

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
async fn test_start_unknown_flow_rejected() {
    let mut host = HostBuilder::new().build().await.unwrap();
    // No flow named "test-pipeline" is defined, so the start is rejected and
    // no flow record is created.
    let err = host.start_flow("test-pipeline").await.unwrap_err();
    assert!(format!("{err}").contains("No flow named"));
    assert!(host.list_flows().await.is_empty());
}

#[tokio::test]
async fn test_start_flow_after_shutdown_rejected() {
    let mut host = HostBuilder::new().build().await.unwrap();
    host.shutdown().await.unwrap();

    let result = host.start_flow("late-flow").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_start_invalid_flow_surfaces_startup_error() {
    use std::collections::BTreeMap;
    use torvyn_config::{FlowDef, NodeDef};

    // A flow with only a sink (no source) is a defined-but-invalid topology.
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

    let mut host = HostBuilder::new()
        .with_flow_definition("invalid", flow)
        .build()
        .await
        .unwrap();

    // The host reaches the pipeline, where topology construction fails; the
    // error propagates and no flow record is created.
    assert!(host.start_flow("invalid").await.is_err());
    assert!(host.list_flows().await.is_empty());
}
