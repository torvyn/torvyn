//! Integration tests for the torvyn-security crate.
//!
//! These tests verify cross-module interactions: capability resolution ->
//! sandbox configuration -> guard enforcement -> audit logging.

use std::sync::Arc;
use torvyn_security::*;
use torvyn_types::ComponentId;

/// End-to-end: component declares capabilities, operator grants subset,
/// sandbox is configured, guard allows granted and denies ungranted.
#[test]
fn test_end_to_end_capability_flow() {
    // Step 1: Component declares capabilities
    let caps = ComponentCapabilities::new(
        vec![
            Capability::WallClock,
            Capability::MonotonicClock,
            Capability::ResourceAllocate { pool: None },
        ],
        vec![Capability::Stderr, Capability::CustomMetrics],
    );

    // Step 2: Operator grants subset
    let grants = OperatorGrants::new(vec![
        Capability::WallClock,
        Capability::MonotonicClock,
        Capability::ResourceAllocate { pool: None },
        Capability::CustomMetrics,
        // Stderr is NOT granted (optional, so just a warning)
    ]);

    // Step 3: Resolve
    let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
    assert_eq!(result.resolved.len(), 4);
    assert!(result.warnings.iter().any(|w| matches!(
        w,
        ResolutionWarning::OptionalNotGranted { .. }
    )));

    // Step 4: Build sandbox config
    let configurator = DefaultSandboxConfigurator;
    let sandbox = configurator
        .configure(
            ComponentId::new(1),
            TenantId::default_tenant(),
            &caps,
            &grants,
            ResourceBudget::default(),
            CpuBudget::default(),
            AuditSinkHandle::noop(),
        )
        .unwrap();

    // Step 5: Verify WASI config
    assert!(sandbox.wasi_config.allow_wall_clock);
    assert!(sandbox.wasi_config.allow_monotonic_clock);
    assert!(!sandbox.wasi_config.allow_stderr); // Not granted

    // Step 6: Create guard and check capabilities
    let guard = CapabilityGuard::new(
        ComponentId::new(1),
        sandbox.resolved_capabilities.clone(),
        AuditSinkHandle::noop(),
    );

    // Granted: should succeed
    assert!(guard.check(&Capability::RuntimeInspect).is_err()); // Not granted
    assert!(guard.check(&Capability::WallClock).is_ok());

    // Step 7: Hot-path checks
    let hp = sandbox.resolved_capabilities.hot_path();
    assert!(hp.can_allocate_resource());
    assert!(hp.can_emit_custom_metrics());
    assert!(!hp.can_emit_backpressure()); // Not requested/granted
}

/// Test: deny-all default denies everything.
#[test]
fn test_deny_all_default() {
    let caps = ComponentCapabilities::new(
        vec![Capability::WallClock],
        vec![],
    );
    let grants = OperatorGrants::deny_all();

    // Resolution should fail
    let result = DefaultCapabilityResolver::resolve(&caps, &grants);
    assert!(result.is_err());

    // Direct sandbox configurator should also fail
    let configurator = DefaultSandboxConfigurator;
    let result = configurator.configure(
        ComponentId::new(1),
        TenantId::default_tenant(),
        &caps,
        &grants,
        ResourceBudget::default(),
        CpuBudget::default(),
        AuditSinkHandle::noop(),
    );
    assert!(result.is_err());
}

/// Test: component with no capabilities succeeds with deny-all grants.
#[test]
fn test_pure_computation_component() {
    let caps = ComponentCapabilities::empty();
    let grants = OperatorGrants::deny_all();

    let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
    assert!(result.resolved.is_empty());
    assert!(result.warnings.is_empty());

    // Guard denies everything
    let guard = CapabilityGuard::new(
        ComponentId::new(1),
        Arc::new(result.resolved),
        AuditSinkHandle::noop(),
    );
    assert!(guard.check(&Capability::WallClock).is_err());
    assert!(guard.check(&Capability::Stderr).is_err());
}

/// Test: filesystem scoping — broad grant, narrow request.
#[test]
fn test_filesystem_scoping_integration() {
    let caps = ComponentCapabilities::new(
        vec![Capability::FilesystemRead {
            path: PathScope::new("/data/input"),
        }],
        vec![],
    );
    let grants = OperatorGrants::new(vec![Capability::FilesystemRead {
        path: PathScope::new("/data"),
    }]);

    let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
    assert_eq!(result.resolved.len(), 1);

    let wasi = WasiConfiguration::from_resolved(&result.resolved);
    assert_eq!(wasi.preopened_dirs.len(), 1);
    assert_eq!(wasi.preopened_dirs[0].host_path, "/data/input");
    assert!(wasi.preopened_dirs[0].read);
}

/// Test: capability string roundtrip through config -> resolution.
#[test]
fn test_capability_config_roundtrip() {
    let config_grant = torvyn_config::CapabilityGrant {
        capabilities: vec![
            "clock:wall".to_owned(),
            "clock:monotonic".to_owned(),
            "filesystem:read:/data/input".to_owned(),
        ],
    };

    let grants = OperatorGrants::from_config_grant(&config_grant).unwrap();
    assert_eq!(grants.grants().len(), 3);
}

/// Test: audit events are emitted during sandbox configuration.
#[test]
fn test_audit_events_during_sandbox_config() {
    use std::sync::Mutex;

    // Custom sink that collects events
    struct CollectingSink {
        events: Mutex<Vec<AuditEvent>>,
    }

    impl AuditSink for CollectingSink {
        fn record(&self, event: AuditEvent) {
            self.events.lock().unwrap().push(event);
        }
        fn flush(&self) {}
    }

    let sink = Arc::new(CollectingSink {
        events: Mutex::new(Vec::new()),
    });
    let handle = AuditSinkHandle::new(AuditSinkWrapper(sink.clone()));

    let caps = ComponentCapabilities::new(
        vec![Capability::WallClock],
        vec![Capability::Stderr], // optional, not granted
    );
    let grants = OperatorGrants::new(vec![Capability::WallClock]);

    let configurator = DefaultSandboxConfigurator;
    let _ = configurator.configure(
        ComponentId::new(1),
        TenantId::default_tenant(),
        &caps,
        &grants,
        ResourceBudget::default(),
        CpuBudget::default(),
        handle,
    );

    let events = sink.events.lock().unwrap();
    // Should have at least: resolution warning + component instantiated
    assert!(events.len() >= 2);
}

/// Wrapper to make Arc<CollectingSink> implement AuditSink
struct AuditSinkWrapper(Arc<dyn AuditSink>);

impl AuditSink for AuditSinkWrapper {
    fn record(&self, event: AuditEvent) {
        self.0.record(event);
    }
    fn flush(&self) {
        self.0.flush();
    }
}

/// Test: has_capability_by_name backing the WIT `has-capability` function.
#[test]
fn test_has_capability_by_name_for_wit() {
    let resolved = ResolvedCapabilitySet::new(vec![
        Capability::WallClock,
        Capability::FilesystemRead {
            path: PathScope::new("/data"),
        },
    ]);

    // Exact match
    assert!(resolved.has_capability_by_name("clock:wall"));
    // Scoped: the resolved set has /data, so /data/input should be permitted
    assert!(resolved.has_capability_by_name("filesystem:read:/data/input"));
    // Not granted
    assert!(!resolved.has_capability_by_name("stdio:stderr"));
    // Invalid string
    assert!(!resolved.has_capability_by_name("garbage"));
}

/// Test: scoped capabilities — filesystem read /tmp/* allows /tmp/foo, denies /etc/foo
#[test]
fn test_scoped_filesystem_allows_and_denies() {
    let caps = ComponentCapabilities::new(
        vec![Capability::FilesystemRead {
            path: PathScope::new("/tmp"),
        }],
        vec![],
    );
    let grants = OperatorGrants::new(vec![Capability::FilesystemRead {
        path: PathScope::new("/tmp"),
    }]);

    let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
    let guard = CapabilityGuard::new(
        ComponentId::new(1),
        Arc::new(result.resolved),
        AuditSinkHandle::noop(),
    );

    // /tmp/foo is within /tmp scope
    assert!(guard
        .check(&Capability::FilesystemRead {
            path: PathScope::new("/tmp/foo"),
        })
        .is_ok());

    // /etc/foo is NOT within /tmp scope
    assert!(guard
        .check(&Capability::FilesystemRead {
            path: PathScope::new("/etc/foo"),
        })
        .is_err());
}

/// Test: component attempts ungranted capability — verify denial + audit log entry
#[test]
fn test_ungranted_capability_denial_with_audit() {
    use std::sync::Mutex;

    struct CollectingSink {
        events: Mutex<Vec<AuditEvent>>,
    }

    impl AuditSink for CollectingSink {
        fn record(&self, event: AuditEvent) {
            self.events.lock().unwrap().push(event);
        }
        fn flush(&self) {}
    }

    let sink = Arc::new(CollectingSink {
        events: Mutex::new(Vec::new()),
    });
    let handle = AuditSinkHandle::new(AuditSinkWrapper(Arc::clone(&sink) as Arc<dyn AuditSink>));

    let resolved = ResolvedCapabilitySet::new(vec![Capability::WallClock]);
    let guard = CapabilityGuard::new(ComponentId::new(42), Arc::new(resolved), handle);

    // Attempt ungranted capability
    let result = guard.check(&Capability::Stderr);
    assert!(result.is_err());

    // Verify audit log entry
    let events = sink.events.lock().unwrap();
    assert!(!events.is_empty());
    let last_event = &events[events.len() - 1];
    assert_eq!(last_event.severity, AuditSeverity::Critical);
    assert!(matches!(
        &last_event.event,
        AuditEventKind::CapabilityDeniedAtRuntime { .. }
    ));
}

/// Test: deny-all default — component with zero grants gets zero capabilities
#[test]
fn test_zero_grants_zero_capabilities() {
    let caps = ComponentCapabilities::empty();
    let grants = OperatorGrants::deny_all();

    let configurator = DefaultSandboxConfigurator;
    let sandbox = configurator
        .configure(
            ComponentId::new(1),
            TenantId::default_tenant(),
            &caps,
            &grants,
            ResourceBudget::default(),
            CpuBudget::default(),
            AuditSinkHandle::noop(),
        )
        .unwrap();

    assert!(sandbox.resolved_capabilities.is_empty());
    assert!(!sandbox.wasi_config.allow_wall_clock);
    assert!(!sandbox.wasi_config.allow_stdout);
    assert!(!sandbox.wasi_config.allow_stderr);
    assert!(sandbox.wasi_config.preopened_dirs.is_empty());

    let hp = sandbox.resolved_capabilities.hot_path();
    assert!(!hp.can_allocate_resource());
    assert!(!hp.can_emit_backpressure());
    assert!(!hp.can_emit_custom_metrics());
    assert!(!hp.can_inspect_flow_meta());
}
