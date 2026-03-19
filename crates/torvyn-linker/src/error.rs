//! Linker error types for the Torvyn runtime.
//!
//! Error code range: E0200–E0299 (per Doc 09, G-08).
//!
//! The linker collects ALL errors during linking rather than failing on the
//! first error. This provides developers with a complete diagnostic picture.

use std::fmt;
use thiserror::Error;

use torvyn_types::LinkError;

/// Aggregated result from a linking operation.
///
/// Contains all errors and warnings discovered during the link phase.
/// The pipeline is only valid if `errors` is empty.
///
/// # Invariants
/// - If `errors` is empty, the linking succeeded and a `LinkedPipeline` was produced.
/// - If `errors` is non-empty, the linking failed — `warnings` may still be present.
/// - `warnings` are informational and do not prevent linking.
///
/// # Examples
/// ```
/// use torvyn_linker::LinkReport;
///
/// let report = LinkReport::new();
/// assert!(report.is_ok());
/// assert_eq!(report.error_count(), 0);
/// ```
#[derive(Debug, Clone)]
pub struct LinkReport {
    /// Errors that prevent the pipeline from being linked.
    pub errors: Vec<LinkDiagnostic>,
    /// Warnings that do not prevent linking but indicate potential issues.
    pub warnings: Vec<LinkDiagnostic>,
}

impl LinkReport {
    /// Create a new, empty (successful) link report.
    ///
    /// # COLD PATH
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Returns `true` if the link operation succeeded (no errors).
    ///
    /// # COLD PATH
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns the total number of errors.
    ///
    /// # COLD PATH
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Returns the total number of warnings.
    ///
    /// # COLD PATH
    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    /// Add an error to the report.
    ///
    /// # COLD PATH
    pub fn push_error(&mut self, diagnostic: LinkDiagnostic) {
        self.errors.push(diagnostic);
    }

    /// Add a warning to the report.
    ///
    /// # COLD PATH
    pub fn push_warning(&mut self, diagnostic: LinkDiagnostic) {
        self.warnings.push(diagnostic);
    }

    /// Merge another report into this one.
    ///
    /// # COLD PATH
    pub fn merge(&mut self, other: LinkReport) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
    }

    /// Format all diagnostics into a human-readable string.
    ///
    /// # COLD PATH
    pub fn format_all(&self) -> String {
        let mut output = String::new();
        if !self.errors.is_empty() {
            output.push_str(&format!(
                "Linking failed with {} error(s):\n",
                self.errors.len()
            ));
            for (i, err) in self.errors.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", i + 1, err));
            }
        }
        if !self.warnings.is_empty() {
            output.push_str(&format!("{} warning(s):\n", self.warnings.len()));
            for (i, warn) in self.warnings.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", i + 1, warn));
            }
        }
        if self.is_ok() && self.warnings.is_empty() {
            output.push_str("Linking succeeded with no issues.\n");
        }
        output
    }

    /// Convert all errors into `LinkError` values from `torvyn-types`.
    ///
    /// # COLD PATH
    pub fn into_link_errors(self) -> Vec<LinkError> {
        self.errors
            .into_iter()
            .map(|d| d.into_link_error())
            .collect()
    }
}

impl Default for LinkReport {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for LinkReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error_count())
    }
}

/// A single diagnostic from the linker.
///
/// Each diagnostic contains a category, a detailed message with remediation
/// guidance, and optional context about which components are involved.
///
/// # Invariants
/// - `message` is always non-empty and contains actionable guidance.
/// - `category` determines whether this is an error or warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkDiagnostic {
    /// The error/warning category.
    pub category: LinkDiagnosticCategory,
    /// Human-readable message with what went wrong and how to fix it.
    pub message: String,
    /// The component that caused the diagnostic (if applicable).
    pub component: Option<String>,
    /// A second component involved (for connection issues).
    pub related_component: Option<String>,
    /// The interface or import name involved (if applicable).
    pub interface_name: Option<String>,
}

impl LinkDiagnostic {
    /// Convert this diagnostic into a `LinkError` from `torvyn-types`.
    ///
    /// # COLD PATH
    pub fn into_link_error(self) -> LinkError {
        match self.category {
            LinkDiagnosticCategory::UnresolvedImport => LinkError::UnresolvedImport {
                component: self.component.unwrap_or_default(),
                import_name: self.interface_name.unwrap_or_default(),
            },
            LinkDiagnosticCategory::InterfaceMismatch => LinkError::InterfaceMismatch {
                from_component: self.component.unwrap_or_default(),
                to_component: self.related_component.unwrap_or_default(),
                interface_name: self.interface_name.unwrap_or_default(),
                detail: self.message,
            },
            LinkDiagnosticCategory::CapabilityDenied => LinkError::CapabilityDenied {
                component: self.component.unwrap_or_default(),
                capability: self.interface_name.unwrap_or_default(),
            },
            LinkDiagnosticCategory::CyclicDependency => LinkError::CyclicDependency {
                cycle: vec![self.message],
            },
            LinkDiagnosticCategory::CompilationFailed => LinkError::CompilationFailed {
                component: self.component.unwrap_or_default(),
                reason: self.message,
            },
            LinkDiagnosticCategory::TopologyError
            | LinkDiagnosticCategory::AmbiguousProvider
            | LinkDiagnosticCategory::RoleViolation
            | LinkDiagnosticCategory::VersionIncompatible => LinkError::InterfaceMismatch {
                from_component: self.component.unwrap_or_default(),
                to_component: self.related_component.unwrap_or_default(),
                interface_name: self.interface_name.unwrap_or_default(),
                detail: self.message,
            },
        }
    }
}

impl fmt::Display for LinkDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.category.error_code();
        write!(f, "[{code}] {}", self.message)?;
        if let Some(ref comp) = self.component {
            write!(f, " (component: '{comp}')")?;
        }
        if let Some(ref related) = self.related_component {
            write!(f, " (related: '{related}')")?;
        }
        if let Some(ref iface) = self.interface_name {
            write!(f, " (interface: '{iface}')")?;
        }
        Ok(())
    }
}

/// Category of a link diagnostic, mapping to error code ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkDiagnosticCategory {
    /// E0200: A required import was not satisfied.
    UnresolvedImport,
    /// E0201: Connected components have incompatible interfaces.
    InterfaceMismatch,
    /// E0202: A capability required by the component was not granted.
    CapabilityDenied,
    /// E0203: The pipeline topology contains a cycle.
    CyclicDependency,
    /// E0204: A component could not be compiled.
    CompilationFailed,
    /// E0205: Topology structural error (disconnected, invalid endpoints).
    TopologyError,
    /// E0206: Multiple providers export the same interface without disambiguation.
    AmbiguousProvider,
    /// E0207: Component role does not match connection direction.
    RoleViolation,
    /// E0208: Contract versions are incompatible.
    VersionIncompatible,
}

impl LinkDiagnosticCategory {
    /// Returns the error code string for this category.
    ///
    /// # COLD PATH
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::UnresolvedImport => "E0200",
            Self::InterfaceMismatch => "E0201",
            Self::CapabilityDenied => "E0202",
            Self::CyclicDependency => "E0203",
            Self::CompilationFailed => "E0204",
            Self::TopologyError => "E0205",
            Self::AmbiguousProvider => "E0206",
            Self::RoleViolation => "E0207",
            Self::VersionIncompatible => "E0208",
        }
    }
}

/// Top-level linker error for functions that return a single error.
///
/// Used by the `PipelineLinker::link` orchestrator when the entire
/// operation fails. The `LinkReport` inside contains all individual
/// diagnostics.
///
/// # Examples
/// ```
/// use torvyn_linker::{LinkerError, LinkReport};
///
/// let report = LinkReport::new();
/// assert!(report.is_ok());
/// ```
#[derive(Debug, Error)]
pub enum LinkerError {
    /// The pipeline configuration is invalid.
    #[error("Invalid pipeline configuration: {0}")]
    InvalidConfig(String),

    /// Linking failed with one or more errors.
    /// The `LinkReport` contains all diagnostics.
    #[error("Linking failed with {0} error(s)")]
    LinkFailed(LinkReport),

    /// A component could not be loaded from disk.
    #[error("Failed to load component '{component}' from '{path}': {reason}")]
    ComponentLoadFailed {
        /// Component name.
        component: String,
        /// Artifact path.
        path: String,
        /// Failure reason.
        reason: String,
    },

    /// Engine error during compilation.
    #[error("Engine error: {0}")]
    EngineError(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_report_new_is_ok() {
        let report = LinkReport::new();
        assert!(report.is_ok());
        assert_eq!(report.error_count(), 0);
        assert_eq!(report.warning_count(), 0);
    }

    #[test]
    fn test_link_report_with_error_is_not_ok() {
        let mut report = LinkReport::new();
        report.push_error(LinkDiagnostic {
            category: LinkDiagnosticCategory::UnresolvedImport,
            message: "missing import 'torvyn:streaming/processor'".into(),
            component: Some("my-sink".into()),
            related_component: None,
            interface_name: Some("torvyn:streaming/processor".into()),
        });
        assert!(!report.is_ok());
        assert_eq!(report.error_count(), 1);
    }

    #[test]
    fn test_link_report_format_all_shows_errors() {
        let mut report = LinkReport::new();
        report.push_error(LinkDiagnostic {
            category: LinkDiagnosticCategory::CapabilityDenied,
            message: "capability 'wasi-filesystem-read' not granted".into(),
            component: Some("file-reader".into()),
            related_component: None,
            interface_name: Some("wasi-filesystem-read".into()),
        });
        let formatted = report.format_all();
        assert!(formatted.contains("1 error(s)"));
        assert!(formatted.contains("wasi-filesystem-read"));
    }

    #[test]
    fn test_link_report_merge() {
        let mut a = LinkReport::new();
        a.push_error(LinkDiagnostic {
            category: LinkDiagnosticCategory::UnresolvedImport,
            message: "error A".into(),
            component: None,
            related_component: None,
            interface_name: None,
        });

        let mut b = LinkReport::new();
        b.push_error(LinkDiagnostic {
            category: LinkDiagnosticCategory::CapabilityDenied,
            message: "error B".into(),
            component: None,
            related_component: None,
            interface_name: None,
        });
        b.push_warning(LinkDiagnostic {
            category: LinkDiagnosticCategory::VersionIncompatible,
            message: "warning B".into(),
            component: None,
            related_component: None,
            interface_name: None,
        });

        a.merge(b);
        assert_eq!(a.error_count(), 2);
        assert_eq!(a.warning_count(), 1);
    }

    #[test]
    fn test_link_diagnostic_display_includes_code() {
        let diag = LinkDiagnostic {
            category: LinkDiagnosticCategory::CyclicDependency,
            message: "A → B → A".into(),
            component: Some("A".into()),
            related_component: None,
            interface_name: None,
        };
        let display = format!("{}", diag);
        assert!(display.contains("E0203"));
        assert!(display.contains("A → B → A"));
    }

    #[test]
    fn test_link_diagnostic_into_link_error() {
        let diag = LinkDiagnostic {
            category: LinkDiagnosticCategory::CapabilityDenied,
            message: "denied".into(),
            component: Some("comp-a".into()),
            related_component: None,
            interface_name: Some("wasi-filesystem-read".into()),
        };
        let err = diag.into_link_error();
        match err {
            LinkError::CapabilityDenied {
                component,
                capability,
            } => {
                assert_eq!(component, "comp-a");
                assert_eq!(capability, "wasi-filesystem-read");
            }
            _ => panic!("expected CapabilityDenied"),
        }
    }

    #[test]
    fn test_error_codes_are_unique() {
        use std::collections::HashSet;
        let categories = [
            LinkDiagnosticCategory::UnresolvedImport,
            LinkDiagnosticCategory::InterfaceMismatch,
            LinkDiagnosticCategory::CapabilityDenied,
            LinkDiagnosticCategory::CyclicDependency,
            LinkDiagnosticCategory::CompilationFailed,
            LinkDiagnosticCategory::TopologyError,
            LinkDiagnosticCategory::AmbiguousProvider,
            LinkDiagnosticCategory::RoleViolation,
            LinkDiagnosticCategory::VersionIncompatible,
        ];
        let codes: HashSet<_> = categories.iter().map(|c| c.error_code()).collect();
        assert_eq!(
            codes.len(),
            categories.len(),
            "all error codes must be unique"
        );
    }

    #[test]
    fn test_into_link_errors_empty() {
        let report = LinkReport::new();
        let errors = report.into_link_errors();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_link_report_format_all_success() {
        let report = LinkReport::new();
        let formatted = report.format_all();
        assert!(formatted.contains("succeeded"));
    }
}
