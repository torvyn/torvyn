//! Error types for the torvyn-security crate.
//!
//! Error code range: E0500-E0599 (per Doc 09 G-08 / Doc 10 C07-2).

use std::fmt;
use torvyn_types::ComponentId;

// ---------------------------------------------------------------------------
// CapabilityResolutionError
// ---------------------------------------------------------------------------

/// An error encountered during capability resolution.
///
/// These are link-time errors — a pipeline will not start if any
/// required capability resolution fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityResolutionError {
    /// A required capability was not granted by the operator.
    MissingRequired {
        /// The capability that is missing.
        capability: String,
    },
    /// A grant exists for the same capability kind, but the scopes are incompatible.
    IncompatibleScope {
        /// The capability with incompatible scope.
        capability: String,
        /// Details about the scope mismatch.
        detail: String,
    },
}

impl fmt::Display for CapabilityResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilityResolutionError::MissingRequired { capability } => {
                write!(
                    f,
                    "[E0520] Required capability '{capability}' is not granted. \
                     Add it to the component's grant list in the pipeline configuration."
                )
            }
            CapabilityResolutionError::IncompatibleScope { capability, detail } => {
                write!(
                    f,
                    "[E0521] Capability '{capability}' scope is incompatible: {detail}. \
                     Verify that the operator grant scope covers the component's request."
                )
            }
        }
    }
}

impl std::error::Error for CapabilityResolutionError {}

// ---------------------------------------------------------------------------
// ResolutionWarning
// ---------------------------------------------------------------------------

/// A warning produced during capability resolution.
///
/// Warnings do not prevent the pipeline from starting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionWarning {
    /// An optional capability was not granted.
    OptionalNotGranted {
        /// The optional capability that was not granted.
        capability: String,
    },
    /// An operator grant was not matched by any component request.
    UnusedGrant {
        /// The unused grant capability.
        capability: String,
    },
}

impl fmt::Display for ResolutionWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolutionWarning::OptionalNotGranted { capability } => {
                write!(
                    f,
                    "Optional capability '{capability}' was not granted. \
                     The component will function without it but may have reduced functionality."
                )
            }
            ResolutionWarning::UnusedGrant { capability } => {
                write!(
                    f,
                    "Grant for '{capability}' was not requested by any component. \
                     This may indicate a configuration error."
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SandboxError
// ---------------------------------------------------------------------------

/// Errors from sandbox configuration.
#[derive(Debug)]
pub enum SandboxError {
    /// Capability resolution failed.
    CapabilityResolutionFailed {
        /// The component whose resolution failed.
        component_id: ComponentId,
        /// The resolution errors.
        errors: Vec<CapabilityResolutionError>,
    },
    /// WASI configuration generation failed.
    WasiConfigFailed {
        /// The component whose WASI config failed.
        component_id: ComponentId,
        /// The reason for failure.
        reason: String,
    },
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::CapabilityResolutionFailed {
                component_id,
                errors,
            } => {
                writeln!(
                    f,
                    "[E0530] Capability resolution failed for {component_id}:"
                )?;
                for err in errors {
                    writeln!(f, "  - {err}")?;
                }
                Ok(())
            }
            SandboxError::WasiConfigFailed {
                component_id,
                reason,
            } => {
                write!(
                    f,
                    "[E0531] WASI configuration failed for {component_id}: {reason}."
                )
            }
        }
    }
}

impl std::error::Error for SandboxError {}

/// Convert to the cross-crate `SecurityError` from `torvyn-types`.
impl From<SandboxError> for torvyn_types::SecurityError {
    fn from(err: SandboxError) -> Self {
        match err {
            SandboxError::CapabilityResolutionFailed {
                component_id,
                errors,
            } => torvyn_types::SecurityError::CapabilityDenied {
                component: component_id,
                capability: errors
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            },
            SandboxError::WasiConfigFailed {
                component_id,
                reason,
            } => torvyn_types::SecurityError::InvalidSandboxConfig {
                component: component_id,
                reason,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// AuditError
// ---------------------------------------------------------------------------

/// Errors from audit logging operations.
#[derive(Debug)]
pub enum AuditError {
    /// Failed to write audit event.
    WriteFailed {
        /// The reason for the write failure.
        reason: String,
    },
    /// Failed to rotate audit log file.
    RotationFailed {
        /// The reason for the rotation failure.
        reason: String,
    },
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditError::WriteFailed { reason } => {
                write!(f, "[E0540] Audit write failed: {reason}.")
            }
            AuditError::RotationFailed { reason } => {
                write!(f, "[E0541] Audit log rotation failed: {reason}.")
            }
        }
    }
}

impl std::error::Error for AuditError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolution_error_display() {
        let err = CapabilityResolutionError::MissingRequired {
            capability: "clock:wall".to_owned(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0520"));
        assert!(msg.contains("clock:wall"));
    }

    #[test]
    fn test_sandbox_error_display() {
        let err = SandboxError::CapabilityResolutionFailed {
            component_id: ComponentId::new(42),
            errors: vec![CapabilityResolutionError::MissingRequired {
                capability: "stdio:stderr".to_owned(),
            }],
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0530"));
        assert!(msg.contains("component-42"));
    }

    #[test]
    fn test_sandbox_error_converts_to_security_error() {
        let err = SandboxError::CapabilityResolutionFailed {
            component_id: ComponentId::new(1),
            errors: vec![CapabilityResolutionError::MissingRequired {
                capability: "clock:wall".to_owned(),
            }],
        };
        let security_err: torvyn_types::SecurityError = err.into();
        assert!(matches!(
            security_err,
            torvyn_types::SecurityError::CapabilityDenied { .. }
        ));
    }

    #[test]
    fn test_resolution_warning_display() {
        let w = ResolutionWarning::UnusedGrant {
            capability: "stdio:stdout".to_owned(),
        };
        let msg = format!("{w}");
        assert!(msg.contains("stdio:stdout"));
        assert!(msg.contains("not requested"));
    }
}
