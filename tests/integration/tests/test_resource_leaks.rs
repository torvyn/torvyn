//! End-to-end test: Resource leak detection.
//!
//! Verifies that the resource manager properly tracks and releases
//! all resources after pipeline operations.
//!
//! # LLI DEVIATION: Uses `DefaultResourceManager::new_for_testing()` and
//! `live_resource_count()` instead of the design doc's `new_default(event_sink)`
//! and `leak_count()` which do not exist. Uses `allocate(owner, capacity, flow_id)`
//! instead of `allocate_for_flow(flow_id, component, capacity, None)`.

use torvyn_integration_tests::{ComponentId, FlowId};
use torvyn_resources::handle::OwnerId;
use torvyn_resources::DefaultResourceManager;

#[test]
fn test_allocate_release_no_leaks() {
    let mgr = DefaultResourceManager::new_for_testing();
    let flow_id = FlowId::new(1);
    let component = ComponentId::new(10);

    mgr.register_flow(flow_id);
    mgr.register_component(component, None);

    // Allocate several buffers.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let handle = mgr
            .allocate(OwnerId::Component(component), 256, flow_id)
            .unwrap();
        handles.push(handle);
    }

    assert!(
        mgr.live_resource_count() >= 10,
        "should have at least 10 live resources"
    );

    // Release all.
    for h in handles {
        mgr.release(h, OwnerId::Component(component)).unwrap();
    }

    assert_eq!(
        mgr.live_resource_count(),
        0,
        "no resources should remain after releasing all"
    );
}

#[test]
fn test_flow_resource_cleanup() {
    let mgr = DefaultResourceManager::new_for_testing();
    let flow_id = FlowId::new(1);
    let component = ComponentId::new(10);

    mgr.register_flow(flow_id);
    mgr.register_component(component, None);

    // Allocate buffers for the flow.
    for _ in 0..5 {
        mgr.allocate(OwnerId::Component(component), 128, flow_id)
            .unwrap();
    }

    let before = mgr.live_resource_count();
    assert!(before >= 5, "should have at least 5 live resources");

    // Release all resources for the flow at once.
    let stats = mgr.release_flow_resources(flow_id).unwrap();
    let total_released = stats.returned_to_pool + stats.deallocated;
    assert_eq!(total_released, 5, "should have released 5 resources");

    assert_eq!(
        mgr.live_resource_count(),
        0,
        "no resources should remain after flow cleanup"
    );
}

#[test]
fn test_force_reclaim_component_resources() {
    let mgr = DefaultResourceManager::new_for_testing();
    let flow_id = FlowId::new(1);
    let component = ComponentId::new(10);

    mgr.register_flow(flow_id);
    mgr.register_component(component, None);

    for _ in 0..3 {
        mgr.allocate(OwnerId::Component(component), 64, flow_id)
            .unwrap();
    }

    let reclaimed = mgr.force_reclaim(component);
    assert_eq!(reclaimed.len(), 3, "should reclaim 3 resources");

    assert_eq!(
        mgr.live_resource_count(),
        0,
        "no resources should remain after force reclaim"
    );
}
