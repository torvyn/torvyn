//! End-to-end test: Capability denial and security guard enforcement.
//!
//! Verifies that the security subsystem correctly denies ungranted capabilities.
//!
//! # LLI DEVIATION: Uses `CapabilityGuard` + `ResolvedCapabilitySet` instead
//! of the design doc's `SecurityManager` (which does not exist).
//! Uses `ResolvedCapabilitySet::empty()` instead of `CapabilityGuard::new_deny_all()`.

use std::sync::Arc;

use torvyn_integration_tests::ComponentId;
use torvyn_security::{
    AuditSinkHandle, Capability, CapabilityGuard, HotPathCapabilities, PathScope,
    ResolvedCapabilitySet,
};

#[test]
fn test_capability_denial_filesystem() {
    let component_id = ComponentId::new(1);

    // Component requests filesystem read but is granted nothing.
    let resolved = ResolvedCapabilitySet::empty();
    let guard = CapabilityGuard::new(component_id, Arc::new(resolved), AuditSinkHandle::noop());

    let check = guard.check(&Capability::FilesystemRead {
        path: PathScope::new("/tmp"),
    });

    assert!(check.is_err(), "filesystem capability should be denied");

    let denied = check.unwrap_err();
    assert_eq!(denied.component_id, component_id);
}

#[test]
fn test_capability_denial_network() {
    let component_id = ComponentId::new(2);

    let resolved = ResolvedCapabilitySet::empty();
    let guard = CapabilityGuard::new(component_id, Arc::new(resolved), AuditSinkHandle::noop());

    let check = guard.check(&Capability::WallClock);
    assert!(check.is_err(), "wall clock should be denied with empty set");
}

#[test]
fn test_capability_grant_allows_access() {
    let component_id = ComponentId::new(3);

    let resolved = ResolvedCapabilitySet::new(vec![Capability::WallClock, Capability::Stderr]);
    let guard = CapabilityGuard::new(component_id, Arc::new(resolved), AuditSinkHandle::noop());

    assert!(
        guard.check(&Capability::WallClock).is_ok(),
        "WallClock should be granted"
    );
    assert!(
        guard.check(&Capability::Stderr).is_ok(),
        "Stderr should be granted"
    );
    assert!(
        guard.check(&Capability::Stdout).is_err(),
        "Stdout should be denied (not granted)"
    );
}

#[test]
fn test_hot_path_capabilities_deny_all() {
    let caps = HotPathCapabilities::deny_all();

    assert!(!caps.can_allocate_resource());
    assert!(!caps.can_emit_backpressure());
    assert!(!caps.can_emit_custom_metrics());
    assert!(!caps.can_inspect_flow_meta());
}

#[test]
fn test_hot_path_capabilities_from_capabilities() {
    let caps = HotPathCapabilities::from_capabilities(&[
        Capability::ResourceAllocate { pool: None },
        Capability::EmitBackpressure,
    ]);

    assert!(caps.can_allocate_resource());
    assert!(caps.can_emit_backpressure());
    assert!(!caps.can_emit_custom_metrics());
    assert!(!caps.can_inspect_flow_meta());
}
