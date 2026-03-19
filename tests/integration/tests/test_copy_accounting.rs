//! End-to-end test: Copy accounting accuracy.
//!
//! Verifies that `CopyLedger` correctly tracks copy operations and
//! byte counts across flows.
//!
//! # LLI DEVIATION: Uses `CopyLedger` directly instead of going through
//! `DefaultResourceManager`, because `record_copy` is on the ledger, not
//! the manager. The design doc assumed a unified API surface.

use torvyn_integration_tests::{ComponentId, FlowId, ResourceId};
use torvyn_resources::accounting::CopyLedger;
use torvyn_types::{CopyReason, NoopEventSink};

#[test]
fn test_copy_accounting_single_write_read() {
    let ledger = CopyLedger::new();
    let flow_id = FlowId::new(1);
    let src = ComponentId::new(1);
    let dst = ComponentId::new(2);
    let resource = ResourceId::new(0, 0);
    let sink = NoopEventSink;

    ledger.register_flow(flow_id);

    // Record a copy from source to host.
    ledger.record_copy(
        flow_id,
        resource,
        src,
        dst,
        256,
        CopyReason::ComponentToHost,
        &sink,
    );

    // Record a copy from host to sink.
    ledger.record_copy(
        flow_id,
        resource,
        src,
        dst,
        256,
        CopyReason::HostToComponent,
        &sink,
    );

    let stats = ledger.flow_stats(flow_id);
    assert_eq!(
        stats.total_copy_ops, 2,
        "should have exactly 2 copy operations"
    );
}

#[test]
fn test_copy_accounting_by_reason() {
    let ledger = CopyLedger::new();
    let flow_id = FlowId::new(1);
    let src = ComponentId::new(1);
    let dst = ComponentId::new(2);
    let resource = ResourceId::new(0, 0);
    let sink = NoopEventSink;

    ledger.register_flow(flow_id);

    // 3 ComponentToHost copies and 2 HostToComponent copies.
    for _ in 0..3 {
        ledger.record_copy(
            flow_id,
            resource,
            src,
            dst,
            128,
            CopyReason::ComponentToHost,
            &sink,
        );
    }
    for _ in 0..2 {
        ledger.record_copy(
            flow_id,
            resource,
            src,
            dst,
            128,
            CopyReason::HostToComponent,
            &sink,
        );
    }

    let stats = ledger.flow_stats(flow_id);
    assert_eq!(stats.total_copy_ops, 5);

    // Verify breakdown by reason.
    // CopyReason indices: HostToComponent=0, ComponentToHost=1, CrossComponent=2, PoolReturn=3
    assert_eq!(stats.copies_by_reason[0], 2, "HostToComponent count");
    assert_eq!(stats.copies_by_reason[1], 3, "ComponentToHost count");
    assert_eq!(stats.copies_by_reason[2], 0, "CrossComponent count");
    assert_eq!(stats.copies_by_reason[3], 0, "PoolReturn count");
}

#[test]
fn test_copy_stats_zero_for_unknown_flow() {
    let ledger = CopyLedger::new();
    let unknown_flow = FlowId::new(999);

    let stats = ledger.flow_stats(unknown_flow);
    assert_eq!(stats.total_copy_ops, 0);
    assert_eq!(stats.total_payload_bytes, 0);
}

#[test]
fn test_copy_accounting_remove_flow_clears_stats() {
    let ledger = CopyLedger::new();
    let flow_id = FlowId::new(1);
    let src = ComponentId::new(1);
    let dst = ComponentId::new(2);
    let resource = ResourceId::new(0, 0);
    let sink = NoopEventSink;

    ledger.register_flow(flow_id);
    ledger.record_copy(
        flow_id,
        resource,
        src,
        dst,
        100,
        CopyReason::ComponentToHost,
        &sink,
    );

    let stats = ledger.flow_stats(flow_id);
    assert_eq!(stats.total_copy_ops, 1);

    ledger.remove_flow(flow_id);

    let stats_after = ledger.flow_stats(flow_id);
    assert_eq!(
        stats_after.total_copy_ops, 0,
        "stats should be zeroed after flow removal"
    );
}
