//! Multi-tenant isolation types.
//!
//! Per Doc 06 §6, full multi-tenant isolation is Phase 2. This module defines
//! the `TenantId` type and tenant context for v1 preparatory hooks.
//! In single-tenant deployments, a default singleton tenant is used.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Identifies a tenant in a multi-tenant Torvyn deployment.
///
/// Per Doc 06 §6.1, a tenant is a logical isolation domain.
/// In single-tenant deployments, all components use `TenantId::default_tenant()`.
///
/// # Invariants
/// - The inner string is non-empty.
///
/// # Examples
/// ```
/// use torvyn_security::TenantId;
///
/// let default = TenantId::default_tenant();
/// assert_eq!(default.as_str(), "default");
///
/// let custom = TenantId::new("team-alpha");
/// assert_eq!(custom.as_str(), "team-alpha");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TenantId(String);

impl TenantId {
    /// Create a new `TenantId`.
    ///
    /// # COLD PATH — called during config parsing.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the default (singleton) tenant for single-tenant deployments.
    pub fn default_tenant() -> Self {
        Self("default".to_owned())
    }

    /// Returns the tenant ID as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` if this is the default singleton tenant.
    #[inline]
    pub fn is_default(&self) -> bool {
        self.0 == "default"
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for TenantId {
    fn default() -> Self {
        Self::default_tenant()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_id_default() {
        let t = TenantId::default_tenant();
        assert_eq!(t.as_str(), "default");
        assert!(t.is_default());
    }

    #[test]
    fn test_tenant_id_custom() {
        let t = TenantId::new("team-alpha");
        assert_eq!(t.as_str(), "team-alpha");
        assert!(!t.is_default());
    }

    #[test]
    fn test_tenant_id_display() {
        let t = TenantId::new("org-123");
        assert_eq!(format!("{t}"), "org-123");
    }

    #[test]
    fn test_tenant_id_equality() {
        let a = TenantId::new("x");
        let b = TenantId::new("x");
        let c = TenantId::new("y");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_tenant_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TenantId::new("a"));
        assert!(set.contains(&TenantId::new("a")));
    }
}
