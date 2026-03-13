//! Capability grant resolution.
//!
//! The resolver computes the effective set of capabilities for a component by
//! intersecting what the component declares it needs with what the operator
//! has granted. Per Doc 06 §3.3:
//!
//! - Missing required capability -> link-time error.
//! - Missing optional capability -> warning.
//! - Unused grant -> warning.
//! - Scope intersection -> effective scope is the most restrictive combination.

use crate::capability::Capability;
use crate::error::{CapabilityResolutionError, ResolutionWarning};
use crate::guard::HotPathCapabilities;
use crate::manifest::{ComponentCapabilities, OperatorGrants};
use std::collections::HashSet;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ResolvedCapabilitySet
// ---------------------------------------------------------------------------

/// The effective set of capabilities for an instantiated component.
///
/// Computed once at link/instantiation time. Immutable during execution.
/// Shared across subsystems via `Arc`.
///
/// # Invariants
/// - Once created, the capability set never changes.
/// - The `hot_path` bitmask is consistent with `capabilities`.
///
/// # Examples
/// ```
/// use torvyn_security::{ResolvedCapabilitySet, Capability};
///
/// let resolved = ResolvedCapabilitySet::new(vec![Capability::WallClock]);
/// assert!(resolved.permits(&Capability::WallClock));
/// assert!(!resolved.permits(&Capability::Stderr));
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResolvedCapabilitySet {
    /// Full set of resolved capabilities with their effective scopes.
    capabilities: Vec<Capability>,
    /// Pre-computed bitmask for hot-path checks.
    hot_path: HotPathCapabilities,
}

impl ResolvedCapabilitySet {
    /// Create a new `ResolvedCapabilitySet` from a list of effective capabilities.
    ///
    /// Automatically computes the `HotPathCapabilities` bitmask.
    ///
    /// # COLD PATH — called during component instantiation.
    pub fn new(capabilities: Vec<Capability>) -> Self {
        let hot_path = HotPathCapabilities::from_capabilities(&capabilities);
        Self {
            capabilities,
            hot_path,
        }
    }

    /// Create an empty capability set (deny-all).
    pub fn empty() -> Self {
        Self {
            capabilities: Vec::new(),
            hot_path: HotPathCapabilities::deny_all(),
        }
    }

    /// Check whether a specific capability is permitted.
    ///
    /// For scoped capabilities, checks whether any granted capability of the
    /// same kind has a scope that contains the requested scope.
    ///
    /// # WARM PATH — called during cold-path capability enforcement.
    /// For hot-path capabilities, use `hot_path()` instead.
    pub fn permits(&self, requested: &Capability) -> bool {
        self.capabilities
            .iter()
            .any(|granted| granted.satisfies(requested))
    }

    /// Returns the hot-path capability bitmask for zero-overhead checks.
    ///
    /// # HOT PATH — the returned struct is used per-element.
    #[inline]
    pub fn hot_path(&self) -> &HotPathCapabilities {
        &self.hot_path
    }

    /// Returns all resolved capabilities.
    #[inline]
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Returns the number of resolved capabilities.
    #[inline]
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Returns `true` if no capabilities are granted.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Check whether a capability with the given string name is available.
    ///
    /// This is the backing implementation for the `has-capability` WIT function
    /// (Doc 06 §3.4, DI-17: string-based for v1).
    ///
    /// # COLD PATH — called during component initialization.
    pub fn has_capability_by_name(&self, name: &str) -> bool {
        // Try to parse the name as a Capability and check
        if let Ok(requested) = name.parse::<Capability>() {
            self.permits(&requested)
        } else {
            // Unrecognized capability name: not granted
            false
        }
    }
}

// ---------------------------------------------------------------------------
// ResolutionResult
// ---------------------------------------------------------------------------

/// The result of capability resolution.
///
/// Contains the resolved set plus any warnings produced during resolution.
#[derive(Debug)]
pub struct ResolutionResult {
    /// The resolved capability set.
    pub resolved: ResolvedCapabilitySet,
    /// Warnings produced during resolution (optional caps not granted, unused grants).
    pub warnings: Vec<ResolutionWarning>,
}

// ---------------------------------------------------------------------------
// DefaultCapabilityResolver
// ---------------------------------------------------------------------------

/// Default implementation of the capability resolution algorithm.
///
/// Implements the algorithm from Doc 06 §3.3:
/// 1. For each required capability, find a matching grant.
///    - Missing -> error.
///    - Found -> intersect scopes -> add effective capability.
/// 2. For each optional capability, find a matching grant.
///    - Missing -> warning.
///    - Found -> intersect scopes -> add effective capability.
/// 3. For each unused grant -> warning.
///
/// # Examples
/// ```
/// use torvyn_security::{
///     DefaultCapabilityResolver, ComponentCapabilities, OperatorGrants, Capability,
/// };
///
/// let caps = ComponentCapabilities::new(
///     vec![Capability::WallClock],
///     vec![Capability::Stderr],
/// );
/// let grants = OperatorGrants::new(vec![Capability::WallClock]);
///
/// let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
/// assert!(result.resolved.permits(&Capability::WallClock));
/// assert!(!result.resolved.permits(&Capability::Stderr));
/// assert_eq!(result.warnings.len(), 1); // optional Stderr not granted
/// ```
pub struct DefaultCapabilityResolver;

impl DefaultCapabilityResolver {
    /// Resolve capabilities by intersecting component declarations with operator grants.
    ///
    /// # COLD PATH — called during `torvyn link` and component instantiation.
    ///
    /// # Errors
    /// Returns `Err(Vec<CapabilityResolutionError>)` if any required capability
    /// is missing or has incompatible scope.
    ///
    /// # Postconditions
    /// - On success, the returned `ResolvedCapabilitySet` contains only capabilities
    ///   that are both requested AND granted.
    /// - Warnings include: optional capabilities not granted, unused grants.
    pub fn resolve(
        component_caps: &ComponentCapabilities,
        operator_grants: &OperatorGrants,
    ) -> Result<ResolutionResult, Vec<CapabilityResolutionError>> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut resolved = Vec::new();
        let mut matched_grant_indices = HashSet::new();

        // Phase 1: Resolve required capabilities
        for required in component_caps.required() {
            match Self::find_matching_grant(required, operator_grants) {
                Some((grant_idx, effective)) => {
                    matched_grant_indices.insert(grant_idx);
                    resolved.push(effective);
                }
                None => {
                    // Check if there's a same-kind grant with incompatible scope
                    let has_same_kind = operator_grants
                        .grants()
                        .iter()
                        .any(|g| g.same_kind(required));

                    if has_same_kind {
                        errors.push(CapabilityResolutionError::IncompatibleScope {
                            capability: required.to_string(),
                            detail: format!(
                                "a grant of the same kind exists but its scope does not cover \
                                 the requested scope '{required}'"
                            ),
                        });
                    } else {
                        errors.push(CapabilityResolutionError::MissingRequired {
                            capability: required.to_string(),
                        });
                    }
                }
            }
        }

        // Phase 2: Resolve optional capabilities
        for optional in component_caps.optional() {
            match Self::find_matching_grant(optional, operator_grants) {
                Some((grant_idx, effective)) => {
                    matched_grant_indices.insert(grant_idx);
                    resolved.push(effective);
                }
                None => {
                    warnings.push(ResolutionWarning::OptionalNotGranted {
                        capability: optional.to_string(),
                    });
                }
            }
        }

        // Phase 3: Check for unused grants
        for (idx, grant) in operator_grants.grants().iter().enumerate() {
            if !matched_grant_indices.contains(&idx) {
                warnings.push(ResolutionWarning::UnusedGrant {
                    capability: grant.to_string(),
                });
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(ResolutionResult {
            resolved: ResolvedCapabilitySet::new(resolved),
            warnings,
        })
    }

    /// Find a matching grant for a requested capability.
    ///
    /// Returns the grant index and the effective (intersected) capability,
    /// or `None` if no grant satisfies the request.
    ///
    /// # COLD PATH — called during resolution.
    fn find_matching_grant(
        request: &Capability,
        grants: &OperatorGrants,
    ) -> Option<(usize, Capability)> {
        for (idx, grant) in grants.grants().iter().enumerate() {
            if let Some(effective) = grant.intersect(request) {
                return Some((idx, effective));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::PathScope;

    #[test]
    fn test_resolve_all_required_granted() {
        let caps = ComponentCapabilities::new(
            vec![Capability::WallClock, Capability::MonotonicClock],
            vec![],
        );
        let grants = OperatorGrants::new(vec![
            Capability::WallClock,
            Capability::MonotonicClock,
        ]);
        let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
        assert_eq!(result.resolved.len(), 2);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_resolve_missing_required_errors() {
        let caps = ComponentCapabilities::new(
            vec![Capability::WallClock, Capability::Stderr],
            vec![],
        );
        let grants = OperatorGrants::new(vec![Capability::WallClock]);
        let result = DefaultCapabilityResolver::resolve(&caps, &grants);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            CapabilityResolutionError::MissingRequired { .. }
        ));
    }

    #[test]
    fn test_resolve_optional_not_granted_warning() {
        let caps = ComponentCapabilities::new(
            vec![Capability::WallClock],
            vec![Capability::Stderr],
        );
        let grants = OperatorGrants::new(vec![Capability::WallClock]);
        let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
        assert_eq!(result.resolved.len(), 1);
        assert_eq!(result.warnings.len(), 1);
        assert!(matches!(
            &result.warnings[0],
            ResolutionWarning::OptionalNotGranted { .. }
        ));
    }

    #[test]
    fn test_resolve_unused_grant_warning() {
        let caps = ComponentCapabilities::new(vec![Capability::WallClock], vec![]);
        let grants = OperatorGrants::new(vec![
            Capability::WallClock,
            Capability::Stderr, // not requested
        ]);
        let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
        assert_eq!(result.resolved.len(), 1);
        assert!(result.warnings.iter().any(|w| matches!(
            w,
            ResolutionWarning::UnusedGrant { .. }
        )));
    }

    #[test]
    fn test_resolve_scope_narrowing() {
        let caps = ComponentCapabilities::new(
            vec![Capability::FilesystemRead {
                path: PathScope::new("/data"),
            }],
            vec![],
        );
        // Operator grants a narrower scope
        let grants = OperatorGrants::new(vec![Capability::FilesystemRead {
            path: PathScope::new("/data/input"),
        }]);
        // The grant is narrower than the request — the intersection is the grant scope.
        // But the grant does NOT satisfy the request because /data/input doesn't cover /data.
        let result = DefaultCapabilityResolver::resolve(&caps, &grants);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_broader_grant_covers_narrower_request() {
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
        // Effective capability is the narrower (request) scope
        assert!(result
            .resolved
            .permits(&Capability::FilesystemRead {
                path: PathScope::new("/data/input")
            }));
    }

    #[test]
    fn test_resolve_deny_all_denies_everything() {
        let caps = ComponentCapabilities::new(
            vec![Capability::WallClock],
            vec![],
        );
        let grants = OperatorGrants::deny_all();
        let result = DefaultCapabilityResolver::resolve(&caps, &grants);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_empty_component_succeeds() {
        let caps = ComponentCapabilities::empty();
        let grants = OperatorGrants::deny_all();
        let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
        assert!(result.resolved.is_empty());
    }

    #[test]
    fn test_resolved_has_capability_by_name() {
        let resolved = ResolvedCapabilitySet::new(vec![
            Capability::WallClock,
            Capability::Stderr,
        ]);
        assert!(resolved.has_capability_by_name("clock:wall"));
        assert!(resolved.has_capability_by_name("stdio:stderr"));
        assert!(!resolved.has_capability_by_name("stdio:stdout"));
        assert!(!resolved.has_capability_by_name("garbage"));
    }

    #[test]
    fn test_resolve_incompatible_scope_error() {
        let caps = ComponentCapabilities::new(
            vec![Capability::FilesystemRead {
                path: PathScope::new("/data/input"),
            }],
            vec![],
        );
        let grants = OperatorGrants::new(vec![Capability::FilesystemRead {
            path: PathScope::new("/data/output"),
        }]);
        let result = DefaultCapabilityResolver::resolve(&caps, &grants);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(matches!(
            &errs[0],
            CapabilityResolutionError::IncompatibleScope { .. }
        ));
    }

    #[test]
    fn test_resolve_multiple_errors_collected() {
        let caps = ComponentCapabilities::new(
            vec![
                Capability::WallClock,
                Capability::MonotonicClock,
                Capability::CryptoRandom,
            ],
            vec![],
        );
        let grants = OperatorGrants::deny_all();
        let result = DefaultCapabilityResolver::resolve(&caps, &grants);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().len(), 3);
    }
}
