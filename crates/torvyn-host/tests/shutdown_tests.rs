//! Integration tests for shutdown behavior.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use torvyn_host::host::FlowRecord;
use torvyn_host::shutdown::graceful_shutdown;
use torvyn_host::ShutdownOutcome;
use torvyn_types::{FlowId, FlowState};

#[tokio::test]
async fn test_shutdown_empty_flows() {
    let flows = Arc::new(RwLock::new(HashMap::new()));
    let outcome = graceful_shutdown(&flows, Duration::from_secs(5)).await;
    assert_eq!(outcome, ShutdownOutcome::already_stopped());
}

#[tokio::test]
async fn test_shutdown_all_already_completed() {
    let flows = Arc::new(RwLock::new(HashMap::new()));
    {
        let mut guard = flows.write().await;
        guard.insert(
            FlowId::new(1),
            FlowRecord {
                flow_id: FlowId::new(1),
                name: "done-1".into(),
                state: FlowState::Completed,
            },
        );
        guard.insert(
            FlowId::new(2),
            FlowRecord {
                flow_id: FlowId::new(2),
                name: "done-2".into(),
                state: FlowState::Completed,
            },
        );
    }

    let outcome = graceful_shutdown(&flows, Duration::from_secs(5)).await;
    assert_eq!(outcome.completed, 2);
    assert_eq!(outcome.timed_out, 0);
}

#[tokio::test]
async fn test_shutdown_cancelled_flows_counted() {
    let flows = Arc::new(RwLock::new(HashMap::new()));
    {
        let mut guard = flows.write().await;
        guard.insert(
            FlowId::new(1),
            FlowRecord {
                flow_id: FlowId::new(1),
                name: "cancelled-flow".into(),
                state: FlowState::Cancelled,
            },
        );
    }

    let outcome = graceful_shutdown(&flows, Duration::from_secs(5)).await;
    assert_eq!(outcome.cancelled, 1);
    assert_eq!(outcome.completed, 0);
}

#[tokio::test]
async fn test_shutdown_mixed_states() {
    let flows = Arc::new(RwLock::new(HashMap::new()));
    {
        let mut guard = flows.write().await;
        guard.insert(
            FlowId::new(1),
            FlowRecord {
                flow_id: FlowId::new(1),
                name: "completed".into(),
                state: FlowState::Completed,
            },
        );
        guard.insert(
            FlowId::new(2),
            FlowRecord {
                flow_id: FlowId::new(2),
                name: "cancelled".into(),
                state: FlowState::Cancelled,
            },
        );
        guard.insert(
            FlowId::new(3),
            FlowRecord {
                flow_id: FlowId::new(3),
                name: "failed".into(),
                state: FlowState::Failed,
            },
        );
    }

    let outcome = graceful_shutdown(&flows, Duration::from_secs(5)).await;
    assert_eq!(outcome.completed, 1);
    assert_eq!(outcome.cancelled, 2); // Cancelled + Failed both count as cancelled
    assert_eq!(outcome.timed_out, 0);
    assert_eq!(outcome.total(), 3);
}

#[tokio::test]
async fn test_shutdown_outcome_total_calculation() {
    let outcome = ShutdownOutcome {
        completed: 5,
        cancelled: 3,
        timed_out: 2,
    };
    assert_eq!(outcome.total(), 10);
}

#[tokio::test]
async fn test_shutdown_outcome_already_stopped() {
    let outcome = ShutdownOutcome::already_stopped();
    assert_eq!(outcome.completed, 0);
    assert_eq!(outcome.cancelled, 0);
    assert_eq!(outcome.timed_out, 0);
    assert_eq!(outcome.total(), 0);
}
