//! Component capability declarations and operator grants.
//!
//! This module converts the raw configuration types from `torvyn-config` into
//! typed capability sets that the resolver can process.

use crate::capability::{Capability, CapabilityParseError};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ComponentCapabilities
// ---------------------------------------------------------------------------

/// What a component declares it needs (parsed from component manifest).
///
/// Capabilities are split into `required` (link-time error if missing)
/// and `optional` (warning if missing, component can query at runtime).
///
/// # Invariants
/// - `required` and `optional` are disjoint — a capability cannot be both.
///
/// # Examples
/// ```
/// use torvyn_security::{ComponentCapabilities, Capability};
///
/// let caps = ComponentCapabilities::new(
///     vec![Capability::WallClock, Capability::MonotonicClock],
///     vec![Capability::Stderr],
/// );
/// assert_eq!(caps.required().len(), 2);
/// assert_eq!(caps.optional().len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ComponentCapabilities {
    required: Vec<Capability>,
    optional: Vec<Capability>,
}

impl ComponentCapabilities {
    /// Create a new `ComponentCapabilities`.
    ///
    /// # COLD PATH — called during manifest parsing.
    ///
    /// # Preconditions
    /// - `required` and `optional` should be disjoint.
    pub fn new(required: Vec<Capability>, optional: Vec<Capability>) -> Self {
        Self { required, optional }
    }

    /// Create empty capabilities (component needs nothing beyond pure computation).
    pub fn empty() -> Self {
        Self {
            required: Vec::new(),
            optional: Vec::new(),
        }
    }

    /// Returns the required capabilities.
    #[inline]
    pub fn required(&self) -> &[Capability] {
        &self.required
    }

    /// Returns the optional capabilities.
    #[inline]
    pub fn optional(&self) -> &[Capability] {
        &self.optional
    }

    /// Returns all capabilities (required + optional).
    pub fn all(&self) -> Vec<&Capability> {
        self.required.iter().chain(self.optional.iter()).collect()
    }

    /// Parse capabilities from a list of strings, splitting by required/optional prefix.
    ///
    /// String format: `"capability-string"` for required,
    /// `"?capability-string"` for optional (leading `?`).
    ///
    /// # COLD PATH — called during manifest parsing.
    ///
    /// # Errors
    /// Returns a list of parse errors for any unparseable capability strings.
    pub fn from_strings(strings: &[String]) -> Result<Self, Vec<CapabilityParseError>> {
        let mut required = Vec::new();
        let mut optional = Vec::new();
        let mut errors = Vec::new();

        for s in strings {
            let (is_optional, cap_str) = if let Some(rest) = s.strip_prefix('?') {
                (true, rest)
            } else {
                (false, s.as_str())
            };

            match cap_str.parse::<Capability>() {
                Ok(cap) => {
                    if is_optional {
                        optional.push(cap);
                    } else {
                        required.push(cap);
                    }
                }
                Err(e) => errors.push(e),
            }
        }

        if errors.is_empty() {
            Ok(Self { required, optional })
        } else {
            Err(errors)
        }
    }
}

impl Default for ComponentCapabilities {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// OperatorGrants
// ---------------------------------------------------------------------------

/// What the operator permits for a specific component (from pipeline config).
///
/// # Examples
/// ```
/// use torvyn_security::{OperatorGrants, Capability};
///
/// let grants = OperatorGrants::new(vec![Capability::WallClock]);
/// assert_eq!(grants.grants().len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OperatorGrants {
    grants: Vec<Capability>,
}

impl OperatorGrants {
    /// Create a new `OperatorGrants`.
    ///
    /// # COLD PATH — called during pipeline config parsing.
    pub fn new(grants: Vec<Capability>) -> Self {
        Self { grants }
    }

    /// Create empty grants (deny everything).
    pub fn deny_all() -> Self {
        Self {
            grants: Vec::new(),
        }
    }

    /// Returns the granted capabilities.
    #[inline]
    pub fn grants(&self) -> &[Capability] {
        &self.grants
    }

    /// Parse operator grants from a list of capability strings.
    ///
    /// # COLD PATH — called during pipeline config parsing.
    ///
    /// # Errors
    /// Returns a list of parse errors for any unparseable capability strings.
    pub fn from_strings(strings: &[String]) -> Result<Self, Vec<CapabilityParseError>> {
        let mut grants = Vec::new();
        let mut errors = Vec::new();

        for s in strings {
            match s.parse::<Capability>() {
                Ok(cap) => grants.push(cap),
                Err(e) => errors.push(e),
            }
        }

        if errors.is_empty() {
            Ok(Self { grants })
        } else {
            Err(errors)
        }
    }

    /// Convert from `torvyn_config::CapabilityGrant`.
    ///
    /// # COLD PATH — called during pipeline config parsing.
    ///
    /// # Errors
    /// Returns a list of parse errors for any unparseable capability strings.
    pub fn from_config_grant(
        grant: &torvyn_config::CapabilityGrant,
    ) -> Result<Self, Vec<CapabilityParseError>> {
        Self::from_strings(&grant.capabilities)
    }
}

impl Default for OperatorGrants {
    fn default() -> Self {
        Self::deny_all()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_component_capabilities_empty() {
        let caps = ComponentCapabilities::empty();
        assert!(caps.required().is_empty());
        assert!(caps.optional().is_empty());
    }

    #[test]
    fn test_component_capabilities_from_strings() {
        let strings = vec![
            "clock:wall".to_owned(),
            "?stdio:stderr".to_owned(),
            "torvyn:resource-allocate".to_owned(),
        ];
        let caps = ComponentCapabilities::from_strings(&strings).unwrap();
        assert_eq!(caps.required().len(), 2);
        assert_eq!(caps.optional().len(), 1);
        assert_eq!(caps.optional()[0], Capability::Stderr);
    }

    #[test]
    fn test_component_capabilities_from_strings_error() {
        let strings = vec!["invalid:garbage".to_owned()];
        let result = ComponentCapabilities::from_strings(&strings);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().len(), 1);
    }

    #[test]
    fn test_operator_grants_deny_all() {
        let grants = OperatorGrants::deny_all();
        assert!(grants.grants().is_empty());
    }

    #[test]
    fn test_operator_grants_from_strings() {
        let strings = vec![
            "clock:wall".to_owned(),
            "filesystem:read:/data".to_owned(),
        ];
        let grants = OperatorGrants::from_strings(&strings).unwrap();
        assert_eq!(grants.grants().len(), 2);
    }

    #[test]
    fn test_operator_grants_from_config_grant() {
        let config_grant = torvyn_config::CapabilityGrant {
            capabilities: vec!["clock:wall".to_owned(), "clock:monotonic".to_owned()],
        };
        let grants = OperatorGrants::from_config_grant(&config_grant).unwrap();
        assert_eq!(grants.grants().len(), 2);
    }

    #[test]
    fn test_component_capabilities_all() {
        let caps = ComponentCapabilities::new(
            vec![Capability::WallClock],
            vec![Capability::Stderr],
        );
        let all = caps.all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_component_capabilities_default_is_empty() {
        let caps = ComponentCapabilities::default();
        assert!(caps.required().is_empty());
        assert!(caps.optional().is_empty());
    }
}
