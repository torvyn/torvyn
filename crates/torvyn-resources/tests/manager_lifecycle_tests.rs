//! Integration tests for the full resource manager lifecycle.

use std::sync::Arc;

use torvyn_resources::manager::*;
use torvyn_resources::pool::TierConfig;
use torvyn_resources::*;
use torvyn_types::*;

fn make_manager() -> DefaultResourceManager {
    let config = ResourceManagerConfig {
        table_capacity: 256,
        pool_configs: [
            TierConfig {
                tier: PoolTier::Small,
                pool_size: 64,
                preallocate: true,
            },
            TierConfig {
                tier: PoolTier::Medium,
                pool_size: 32,
                preallocate: true,
            },
            TierConfig {
                tier: PoolTier::Large,
                pool_size: 8,
                preallocate: true,
            },
            TierConfig {
                tier: PoolTier::Huge,
                pool_size: 2,
                preallocate: false,
            },
        ],
        default_budget_bytes: 64 * 1024 * 1024,
    };
    DefaultResourceManager::new(config, Arc::new(NoopEventSink))
}

// ---------------------------------------------------------------------------
// Full lifecycle: allocate → borrow → return → transfer → reclaim
// ---------------------------------------------------------------------------

#[test]
fn test_full_lifecycle() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), None);
    mgr.register_component(ComponentId::new(2), None);

    let owner_a = OwnerId::Component(ComponentId::new(1));
    let owner_b = OwnerId::Component(ComponentId::new(2));

    // Allocate
    let handle = mgr.allocate(owner_a, 256, flow).unwrap();
    let info = mgr.inspect(handle).unwrap();
    assert_eq!(info.state, ResourceState::Owned);
    assert_eq!(info.owner, owner_a);

    // Write
    mgr.write_payload(handle, owner_a, 0, b"test data", flow)
        .unwrap();

    // Borrow
    mgr.borrow_start(handle, ComponentId::new(2)).unwrap();
    let info = mgr.inspect(handle).unwrap();
    assert_eq!(info.state, ResourceState::Borrowed);

    // Read during borrow
    let data = mgr.read_payload(handle, owner_a, 0, 9, flow).unwrap();
    assert_eq!(&data, b"test data");

    // Return borrow
    mgr.borrow_end(handle, ComponentId::new(2)).unwrap();
    let info = mgr.inspect(handle).unwrap();
    assert_eq!(info.state, ResourceState::Owned);

    // Transfer
    mgr.transfer_ownership(handle, owner_a, owner_b).unwrap();
    let info = mgr.inspect(handle).unwrap();
    assert_eq!(info.owner, owner_b);

    // Release
    mgr.release(handle, owner_b).unwrap();
    assert_eq!(mgr.live_resource_count(), 0);
}

// ---------------------------------------------------------------------------
// Source → Processor → Sink pipeline: verify exact copy count
// ---------------------------------------------------------------------------

#[test]
fn test_source_sink_pipeline_copy_count() {
    // Source → Sink: 2 copies per element (source writes, sink reads)
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), None); // source
    mgr.register_component(ComponentId::new(2), None); // sink

    let source = OwnerId::Component(ComponentId::new(1));
    let payload = [42u8; 128];

    for _ in 0..100 {
        // Source produces
        let buf = mgr.allocate(source, 128, flow).unwrap();
        mgr.write_payload(buf, source, 0, &payload, flow).unwrap(); // copy 1

        // Transfer to host
        mgr.transfer_ownership(buf, source, OwnerId::Host).unwrap();

        // Sink reads (via borrow)
        mgr.borrow_start(buf, ComponentId::new(2)).unwrap();
        let _ = mgr.read_payload(buf, OwnerId::Host, 0, 128, flow).unwrap(); // copy 2
        mgr.borrow_end(buf, ComponentId::new(2)).unwrap();

        // Release
        mgr.release(buf, OwnerId::Host).unwrap();
    }

    let stats = mgr.flow_copy_stats(flow);
    assert_eq!(stats.total_copy_ops, 200); // 100 * 2
    assert_eq!(mgr.live_resource_count(), 0);
}

#[test]
fn test_three_stage_pipeline_copy_count() {
    // Source → Processor → Sink: 4 copies per element
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), None);
    mgr.register_component(ComponentId::new(2), None);
    mgr.register_component(ComponentId::new(3), None);

    let source = OwnerId::Component(ComponentId::new(1));
    let processor = OwnerId::Component(ComponentId::new(2));

    for _ in 0..50 {
        // Source writes
        let src_buf = mgr.allocate(source, 128, flow).unwrap();
        mgr.write_payload(src_buf, source, 0, &[1u8; 128], flow)
            .unwrap(); // copy 1

        mgr.transfer_ownership(src_buf, source, OwnerId::Host)
            .unwrap();

        // Processor reads input
        mgr.borrow_start(src_buf, ComponentId::new(2)).unwrap();
        let _ = mgr
            .read_payload(src_buf, OwnerId::Host, 0, 128, flow)
            .unwrap(); // copy 2
        mgr.borrow_end(src_buf, ComponentId::new(2)).unwrap();

        // Processor writes output
        let proc_buf = mgr.allocate(processor, 128, flow).unwrap();
        mgr.write_payload(proc_buf, processor, 0, &[2u8; 128], flow)
            .unwrap(); // copy 3

        // Release source buffer
        mgr.release(src_buf, OwnerId::Host).unwrap();

        // Transfer processor output
        mgr.transfer_ownership(proc_buf, processor, OwnerId::Host)
            .unwrap();

        // Sink reads
        mgr.borrow_start(proc_buf, ComponentId::new(3)).unwrap();
        let _ = mgr
            .read_payload(proc_buf, OwnerId::Host, 0, 128, flow)
            .unwrap(); // copy 4
        mgr.borrow_end(proc_buf, ComponentId::new(3)).unwrap();

        mgr.release(proc_buf, OwnerId::Host).unwrap();
    }

    let stats = mgr.flow_copy_stats(flow);
    assert_eq!(stats.total_copy_ops, 200); // 50 elements * 4 copies
    assert_eq!(mgr.live_resource_count(), 0);
}

// ---------------------------------------------------------------------------
// 10,000 elements through 3-stage pipeline: zero leaked resources
// ---------------------------------------------------------------------------

#[test]
fn test_10000_elements_zero_leaks() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), None);
    mgr.register_component(ComponentId::new(2), None);
    mgr.register_component(ComponentId::new(3), None);

    let source = OwnerId::Component(ComponentId::new(1));
    let processor = OwnerId::Component(ComponentId::new(2));

    for i in 0u64..10_000 {
        // Source writes
        let src_buf = mgr.allocate(source, 64, flow).unwrap();
        let payload = (i as u32).to_le_bytes();
        mgr.write_payload(src_buf, source, 0, &payload, flow)
            .unwrap();

        mgr.transfer_ownership(src_buf, source, OwnerId::Host)
            .unwrap();

        // Processor reads
        mgr.borrow_start(src_buf, ComponentId::new(2)).unwrap();
        let _ = mgr
            .read_payload(src_buf, OwnerId::Host, 0, 4, flow)
            .unwrap();
        mgr.borrow_end(src_buf, ComponentId::new(2)).unwrap();

        // Processor produces output
        let proc_buf = mgr.allocate(processor, 64, flow).unwrap();
        mgr.write_payload(proc_buf, processor, 0, &payload, flow)
            .unwrap();

        // Release source
        mgr.release(src_buf, OwnerId::Host).unwrap();

        // Transfer to sink
        mgr.transfer_ownership(proc_buf, processor, OwnerId::Host)
            .unwrap();
        mgr.borrow_start(proc_buf, ComponentId::new(3)).unwrap();
        let _ = mgr
            .read_payload(proc_buf, OwnerId::Host, 0, 4, flow)
            .unwrap();
        mgr.borrow_end(proc_buf, ComponentId::new(3)).unwrap();

        mgr.release(proc_buf, OwnerId::Host).unwrap();
    }

    assert_eq!(mgr.live_resource_count(), 0);

    let stats = mgr.flow_copy_stats(flow);
    assert_eq!(stats.total_copy_ops, 40_000); // 10,000 * 4 copies
}

// ---------------------------------------------------------------------------
// Memory budget enforcement: within, at limit, over limit
// ---------------------------------------------------------------------------

#[test]
fn test_budget_within_limit() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), Some(1024));

    let owner = OwnerId::Component(ComponentId::new(1));
    // Small tier = 256 bytes. 1024 / 256 = 4 buffers fit.
    let mut handles = Vec::new();
    for _ in 0..4 {
        handles.push(mgr.allocate(owner, 100, flow).unwrap());
    }

    for h in handles {
        mgr.release(h, owner).unwrap();
    }
}

#[test]
fn test_budget_at_limit() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), Some(256));

    let owner = OwnerId::Component(ComponentId::new(1));
    // Exactly one Small buffer fills the budget
    let h = mgr.allocate(owner, 100, flow).unwrap();

    // Second should fail
    let result = mgr.allocate(owner, 100, flow);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ResourceError::BudgetExceeded { .. }
    ));

    mgr.release(h, owner).unwrap();
}

#[test]
fn test_budget_over_limit() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), Some(100));

    let owner = OwnerId::Component(ComponentId::new(1));
    // Small tier = 256 bytes > 100 byte budget
    let result = mgr.allocate(owner, 50, flow);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        ResourceError::BudgetExceeded { .. }
    ));
}

// ---------------------------------------------------------------------------
// Component crash cleanup: force_reclaim releases all resources
// ---------------------------------------------------------------------------

#[test]
fn test_force_reclaim_during_active_borrows() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), None);

    let owner = OwnerId::Component(ComponentId::new(1));
    let handle = mgr.allocate(owner, 100, flow).unwrap();

    // Start a borrow (simulating a component function in progress)
    mgr.borrow_start(handle, ComponentId::new(1)).unwrap();

    // Crash cleanup
    let reclaimed = mgr.force_reclaim(ComponentId::new(1));
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].previous_state, ResourceState::Borrowed);
    assert_eq!(mgr.live_resource_count(), 0);
}

#[test]
fn test_force_reclaim_multiple_resources() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), None);

    let owner = OwnerId::Component(ComponentId::new(1));
    let _h1 = mgr.allocate(owner, 100, flow).unwrap();
    let _h2 = mgr.allocate(owner, 200, flow).unwrap();
    let h3 = mgr.allocate(owner, 300, flow).unwrap();
    mgr.borrow_start(h3, ComponentId::new(1)).unwrap();

    assert_eq!(mgr.live_resource_count(), 3);

    let reclaimed = mgr.force_reclaim(ComponentId::new(1));
    assert_eq!(reclaimed.len(), 3);
    assert_eq!(mgr.live_resource_count(), 0);
}

// ---------------------------------------------------------------------------
// Concurrent flow isolation
// ---------------------------------------------------------------------------

#[test]
fn test_concurrent_flow_isolation() {
    let mgr = make_manager();
    let flow_a = FlowId::new(1);
    let flow_b = FlowId::new(2);
    mgr.register_flow(flow_a);
    mgr.register_flow(flow_b);

    // Allocate in flow A
    let ha = mgr.allocate(OwnerId::Host, 100, flow_a).unwrap();
    mgr.write_payload(ha, OwnerId::Host, 0, b"flow-a", flow_a)
        .unwrap();

    // Allocate in flow B
    let hb = mgr.allocate(OwnerId::Host, 100, flow_b).unwrap();
    mgr.write_payload(hb, OwnerId::Host, 0, b"flow-b", flow_b)
        .unwrap();

    // Release flow A resources
    let stats_a = mgr.release_flow_resources(flow_a).unwrap();
    assert_eq!(stats_a.returned_to_pool + stats_a.deallocated, 1);

    // Flow B should still be alive
    assert_eq!(mgr.live_resource_count(), 1);
    let info = mgr.inspect(hb).unwrap();
    assert_eq!(info.flow_id, flow_b);

    mgr.release(hb, OwnerId::Host).unwrap();
}

// ---------------------------------------------------------------------------
// Pool reuse after release
// ---------------------------------------------------------------------------

#[test]
fn test_pool_reuse_after_release() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);

    // Allocate and release many times — pool should handle reuse
    for _ in 0..500 {
        let handle = mgr.allocate(OwnerId::Host, 100, flow).unwrap();
        mgr.release(handle, OwnerId::Host).unwrap();
    }

    assert_eq!(mgr.live_resource_count(), 0);

    // Pool metrics should show reuse
    let metrics = mgr.pool_metrics(PoolTier::Small);
    assert_eq!(metrics.alloc_count, 500);
    assert!(metrics.fallback_count < 500); // Most should come from pool
}

// ---------------------------------------------------------------------------
// Property-like: random operation sequences never produce invalid state
// ---------------------------------------------------------------------------

#[test]
fn test_sequential_ops_never_invalid_state() {
    let mgr = make_manager();
    let flow = FlowId::new(1);
    mgr.register_flow(flow);
    mgr.register_component(ComponentId::new(1), None);
    mgr.register_component(ComponentId::new(2), None);

    let source = OwnerId::Component(ComponentId::new(1));
    let sink = OwnerId::Component(ComponentId::new(2));

    // Run a variety of operations and ensure the state machine never panics
    for round in 0..100u64 {
        let h = mgr.allocate(source, 128, flow).unwrap();
        let payload = format!("round-{round}");
        mgr.write_payload(h, source, 0, payload.as_bytes(), flow)
            .unwrap();

        // Transfer to host
        mgr.transfer_ownership(h, source, OwnerId::Host).unwrap();

        // Try double-borrow
        mgr.borrow_start(h, ComponentId::new(1)).unwrap();
        mgr.borrow_start(h, ComponentId::new(2)).unwrap();

        // Read while borrowed
        let data = mgr
            .read_payload(h, OwnerId::Host, 0, payload.len() as u32, flow)
            .unwrap();
        assert_eq!(data, payload.as_bytes());

        // End both borrows
        mgr.borrow_end(h, ComponentId::new(2)).unwrap();
        mgr.borrow_end(h, ComponentId::new(1)).unwrap();

        // Transfer to sink
        mgr.transfer_ownership(h, OwnerId::Host, sink).unwrap();

        // Release from sink
        mgr.release(h, sink).unwrap();
    }

    assert_eq!(mgr.live_resource_count(), 0);
}
