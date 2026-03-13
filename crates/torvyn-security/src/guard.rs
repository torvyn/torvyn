//! Runtime capability enforcement.
//!
//! `CapabilityGuard` is the runtime check used in Torvyn host function handlers.
//! `HotPathCapabilities` is a pre-computed bitmask for per-element capabilities
//! that must add less than 1ns of overhead per check.

use crate::audit::{AuditEvent, AuditEventKind, AuditSeverity, AuditSinkHandle};
use crate::capability::Capability;
use crate::resolver::ResolvedCapabilitySet;
use std::sync::Arc;
use torvyn_types::ComponentId;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// HotPathCapabilities
// ---------------------------------------------------------------------------

/// Pre-computed bitmask for hot-path capability checks.
///
/// Set once at component instantiation. Immutable during execution.
/// Each field is a single boolean — checking costs one load instruction.
///
/// Per Doc 06 §4.4, hot-path capability checks must add less than 1ns
/// per element. A boolean read trivially meets this target.
///
/// # Invariants
/// - Fields are set at construction time and never modified.
///
/// # Examples
/// ```
/// use torvyn_security::{HotPathCapabilities, Capability};
///
/// let caps = HotPathCapabilities::from_capabilities(&[
///     Capability::ResourceAllocate { pool: None },
///     Capability::CustomMetrics,
/// ]);
/// assert!(caps.can_allocate_resource());
/// assert!(caps.can_emit_custom_metrics());
/// assert!(!caps.can_emit_backpressure());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HotPathCapabilities {
    resource_allocate: bool,
    emit_backpressure: bool,
    custom_metrics: bool,
    inspect_flow_meta: bool,
}

impl HotPathCapabilities {
    /// Compute the bitmask from a set of resolved capabilities.
    ///
    /// # COLD PATH — called once at component instantiation.
    pub fn from_capabilities(capabilities: &[Capability]) -> Self {
        let mut result = Self::deny_all();
        for cap in capabilities {
            match cap {
                Capability::ResourceAllocate { .. } => result.resource_allocate = true,
                Capability::EmitBackpressure => result.emit_backpressure = true,
                Capability::CustomMetrics => result.custom_metrics = true,
                Capability::InspectFlowMeta => result.inspect_flow_meta = true,
                _ => {} // Not a hot-path capability
            }
        }
        result
    }

    /// Create a deny-all bitmask.
    pub const fn deny_all() -> Self {
        Self {
            resource_allocate: false,
            emit_backpressure: false,
            custom_metrics: false,
            inspect_flow_meta: false,
        }
    }

    /// Check whether the component can allocate resources.
    ///
    /// # HOT PATH — called per resource allocation request.
    #[inline(always)]
    pub fn can_allocate_resource(&self) -> bool {
        self.resource_allocate
    }

    /// Check whether the component can emit backpressure signals.
    ///
    /// # HOT PATH — called per backpressure emission.
    #[inline(always)]
    pub fn can_emit_backpressure(&self) -> bool {
        self.emit_backpressure
    }

    /// Check whether the component can emit custom metrics.
    ///
    /// # HOT PATH — called per metric emission.
    #[inline(always)]
    pub fn can_emit_custom_metrics(&self) -> bool {
        self.custom_metrics
    }

    /// Check whether the component can inspect flow metadata.
    ///
    /// # HOT PATH — called per flow metadata access.
    #[inline(always)]
    pub fn can_inspect_flow_meta(&self) -> bool {
        self.inspect_flow_meta
    }
}

// ---------------------------------------------------------------------------
// CapabilityGuard
// ---------------------------------------------------------------------------

/// Guard that checks capability authorization before executing a host function.
///
/// Threaded through Torvyn host function handlers for runtime capability checks.
/// Emits audit events for both granted and denied checks. For hot-path capabilities,
/// use `hot_path()` on the `ResolvedCapabilitySet` instead — this guard is for
/// cold-path checks where individual audit events are acceptable.
///
/// Per Doc 06 §4.3, this is used for Torvyn-specific capabilities (Layer 2 enforcement).
/// WASI capabilities are enforced by WasiCtx configuration (Layer 1) at zero runtime cost.
///
/// # Examples
/// ```
/// use torvyn_security::{CapabilityGuard, ResolvedCapabilitySet, Capability, AuditSinkHandle};
/// use torvyn_types::ComponentId;
///
/// let resolved = ResolvedCapabilitySet::new(vec![Capability::RuntimeInspect]);
/// let guard = CapabilityGuard::new(
///     ComponentId::new(1),
///     std::sync::Arc::new(resolved),
///     AuditSinkHandle::noop(),
/// );
/// assert!(guard.check(&Capability::RuntimeInspect).is_ok());
/// assert!(guard.check(&Capability::CustomMetrics).is_err());
/// ```
pub struct CapabilityGuard {
    component_id: ComponentId,
    resolved: Arc<ResolvedCapabilitySet>,
    audit_sink: AuditSinkHandle,
}

impl CapabilityGuard {
    /// Create a new `CapabilityGuard`.
    ///
    /// # COLD PATH — called during component instantiation.
    pub fn new(
        component_id: ComponentId,
        resolved: Arc<ResolvedCapabilitySet>,
        audit_sink: AuditSinkHandle,
    ) -> Self {
        Self {
            component_id,
            resolved,
            audit_sink,
        }
    }

    /// Check whether the component is authorized for the given capability.
    ///
    /// Emits an audit event for the check (granted or denied).
    /// Returns `Ok(())` if authorized, `Err(CapabilityDenied)` if not.
    ///
    /// # WARM PATH — called per cold-path capability exercise.
    /// Do NOT use for hot-path capabilities (ResourceAllocate, EmitBackpressure,
    /// CustomMetrics, InspectFlowMeta) — use `HotPathCapabilities` instead.
    ///
    /// # Postconditions
    /// - An audit event is always emitted (regardless of outcome).
    /// - On denial, the returned error contains the component ID and capability.
    pub fn check(&self, capability: &Capability) -> Result<(), CapabilityDenied> {
        let authorized = self.resolved.permits(capability);

        // Emit audit event for cold-path checks
        self.audit_sink.record_sync(AuditEvent::new(
            if authorized {
                AuditSeverity::Info
            } else {
                AuditSeverity::Critical
            },
            Some(self.component_id),
            None, // flow_id populated by caller if available
            None, // tenant_id populated by SecurityContext
            if authorized {
                AuditEventKind::CapabilityExercised {
                    capability: capability.to_string(),
                    detail: None,
                }
            } else {
                AuditEventKind::CapabilityDeniedAtRuntime {
                    capability: capability.to_string(),
                    detail: "capability not in resolved set".to_owned(),
                }
            },
        ));

        if authorized {
            Ok(())
        } else {
            Err(CapabilityDenied {
                component_id: self.component_id,
                capability: capability.clone(),
            })
        }
    }

    /// Returns a reference to the resolved capability set.
    #[inline]
    pub fn resolved(&self) -> &ResolvedCapabilitySet {
        &self.resolved
    }

    /// Returns the component ID this guard is associated with.
    #[inline]
    pub fn component_id(&self) -> ComponentId {
        self.component_id
    }
}

// ---------------------------------------------------------------------------
// CapabilityDenied
// ---------------------------------------------------------------------------

/// Error returned when a runtime capability check fails.
///
/// Contains the component ID and the denied capability for actionable
/// error reporting and audit logging.
#[derive(Debug, Clone)]
pub struct CapabilityDenied {
    /// The component that was denied.
    pub component_id: ComponentId,
    /// The capability that was denied.
    pub capability: Capability,
}

impl std::fmt::Display for CapabilityDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[E0503] Component {} denied capability '{}'. \
             Grant the capability in the pipeline configuration or verify the component's \
             declared capabilities match the operator grants.",
            self.component_id, self.capability
        )
    }
}

impl std::error::Error for CapabilityDenied {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hot_path_deny_all() {
        let hp = HotPathCapabilities::deny_all();
        assert!(!hp.can_allocate_resource());
        assert!(!hp.can_emit_backpressure());
        assert!(!hp.can_emit_custom_metrics());
        assert!(!hp.can_inspect_flow_meta());
    }

    #[test]
    fn test_hot_path_from_capabilities() {
        let caps = vec![
            Capability::ResourceAllocate { pool: None },
            Capability::EmitBackpressure,
            Capability::WallClock, // not hot-path
        ];
        let hp = HotPathCapabilities::from_capabilities(&caps);
        assert!(hp.can_allocate_resource());
        assert!(hp.can_emit_backpressure());
        assert!(!hp.can_emit_custom_metrics());
        assert!(!hp.can_inspect_flow_meta());
    }

    #[test]
    fn test_guard_allows_granted_capability() {
        let resolved = ResolvedCapabilitySet::new(vec![Capability::RuntimeInspect]);
        let guard = CapabilityGuard::new(
            ComponentId::new(1),
            Arc::new(resolved),
            AuditSinkHandle::noop(),
        );
        assert!(guard.check(&Capability::RuntimeInspect).is_ok());
    }

    #[test]
    fn test_guard_denies_ungranted_capability() {
        let resolved = ResolvedCapabilitySet::new(vec![Capability::RuntimeInspect]);
        let guard = CapabilityGuard::new(
            ComponentId::new(1),
            Arc::new(resolved),
            AuditSinkHandle::noop(),
        );
        let result = guard.check(&Capability::CustomMetrics);
        assert!(result.is_err());
        let denied = result.unwrap_err();
        assert_eq!(denied.component_id, ComponentId::new(1));
    }

    #[test]
    fn test_guard_denies_everything_with_empty_set() {
        let resolved = ResolvedCapabilitySet::empty();
        let guard = CapabilityGuard::new(
            ComponentId::new(1),
            Arc::new(resolved),
            AuditSinkHandle::noop(),
        );
        assert!(guard.check(&Capability::WallClock).is_err());
        assert!(guard.check(&Capability::RuntimeInspect).is_err());
        assert!(guard.check(&Capability::Stderr).is_err());
    }
}
