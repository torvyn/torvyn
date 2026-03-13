//! Error types for the Torvyn contract validation subsystem.
//!
//! Error codes are allocated in the E0100–E0199 range (per Doc 09 G-08).
//! Every error is designed to be actionable: it tells the user what went
//! wrong, where, and how to fix it.

use std::fmt;
use std::path::PathBuf;

// COLD PATH — errors are only constructed during validation, never on the hot path.

/// A source location within a WIT file or manifest.
///
/// Invariants:
/// - `line` and `column` are 1-indexed.
/// - `file` is always a valid path relative to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// Path to the file containing the error.
    pub file: PathBuf,
    /// 1-indexed line number.
    pub line: u32,
    /// 1-indexed column number. 0 if column is unknown.
    pub column: u32,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.column > 0 {
            write!(f, "{}:{}:{}", self.file.display(), self.line, self.column)
        } else {
            write!(f, "{}:{}", self.file.display(), self.line)
        }
    }
}

/// Error codes for contract validation (E0100–E0199).
///
/// Each code is stable across versions and searchable in documentation.
///
/// Invariants:
/// - Codes are in the range 100..200 (exclusive).
/// - Each code maps to exactly one error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorCode {
    // === Parse errors (E0100–E0109) ===
    /// WIT file contains syntax errors.
    WitSyntaxError = 100,
    /// WIT file references an undefined type or interface.
    WitUnresolvedReference = 101,
    /// WIT package declaration is missing or malformed.
    WitPackageDeclaration = 102,

    // === Manifest errors (E0110–E0119) ===
    /// Torvyn.toml is not valid TOML.
    ManifestParseError = 110,
    /// Torvyn.toml is missing required fields.
    ManifestMissingField = 111,
    /// Capability declaration is syntactically invalid.
    ManifestCapabilityError = 112,
    /// Component version is not valid semver.
    ManifestVersionError = 113,

    // === Semantic validation errors (E0120–E0139) ===
    /// Component's world does not export any Torvyn processing interface.
    NoExportedInterface = 120,
    /// Component's WIT world imports an interface not available in the
    /// declared capability set.
    CapabilityMismatch = 121,
    /// Version constraint is unsatisfiable (internal contradiction).
    UnsatisfiableVersion = 122,
    /// Interface completeness violation — exported interface missing
    /// required functions.
    IncompleteInterface = 123,

    // === Compatibility errors (E0140–E0159) ===
    /// Major version mismatch between components.
    IncompatibleMajorVersion = 140,
    /// Consumer requires a function not present in provider's version.
    MissingFunction = 141,
    /// Consumer requires an interface not present in provider's version.
    MissingInterface = 142,
    /// Type signature changed between versions.
    TypeSignatureChanged = 143,
    /// Ownership semantics changed (borrow↔own).
    OwnershipSemanticChanged = 144,
    /// Field removed from record.
    FieldRemoved = 145,
    /// Variant case removed from variant/enum.
    VariantCaseRemoved = 146,

    // === Link errors (E0160–E0179) ===
    /// Pipeline topology is not a valid DAG.
    InvalidTopology = 160,
    /// Source component has incoming connections.
    SourceHasIncoming = 161,
    /// Sink component has outgoing connections.
    SinkHasOutgoing = 162,
    /// Component has no connections.
    DisconnectedComponent = 163,
    /// Required capability not granted in pipeline config.
    UnmetCapability = 164,
    /// Router port name does not match any downstream component.
    UnknownRouterPort = 165,
    /// Interface type mismatch between connected components.
    InterfaceTypeMismatch = 166,
    /// Version ranges have no satisfying intersection.
    VersionRangeUnsatisfiable = 167,
}

impl ErrorCode {
    /// Returns the formatted error code string (e.g., "E0100").
    ///
    /// # COLD PATH
    pub fn as_str(&self) -> String {
        format!("E{:04}", *self as u16)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E{:04}", *self as u16)
    }
}

/// A single validation diagnostic.
///
/// Follows the `rustc` error message style: error code, location,
/// explanation, and actionable help.
///
/// Invariants:
/// - `code` is always set.
/// - `message` is non-empty.
/// - `locations` has at least one entry for context.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// The error code (stable, machine-parseable).
    pub code: ErrorCode,
    /// Severity level.
    pub severity: Severity,
    /// Primary error message (what went wrong).
    pub message: String,
    /// Source locations relevant to this diagnostic.
    pub locations: Vec<LocationContext>,
    /// Additional notes providing context.
    pub notes: Vec<String>,
    /// Actionable help text (how to fix it).
    pub help: Option<String>,
}

/// Severity level for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational hint.
    Hint,
    /// Warning — does not block validation but indicates a potential issue.
    Warning,
    /// Error — blocks validation.
    Error,
}

/// A source location with an explanatory label.
#[derive(Debug, Clone)]
pub struct LocationContext {
    /// Where in the source this context refers to.
    pub location: SourceLocation,
    /// Label explaining what this location represents.
    pub label: String,
}

impl fmt::Display for Diagnostic {
    /// Formats the diagnostic in rustc-style.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity_str = match self.severity {
            Severity::Hint => "hint",
            Severity::Warning => "warning",
            Severity::Error => "error",
        };

        writeln!(f, "{}[{}]: {}", severity_str, self.code, self.message)?;

        for loc_ctx in &self.locations {
            writeln!(f, "  --> {}", loc_ctx.location)?;
            writeln!(f, "   |")?;
            writeln!(f, "   | {}", loc_ctx.label)?;
        }

        for note in &self.notes {
            writeln!(f, "   = note: {}", note)?;
        }

        if let Some(ref help) = self.help {
            writeln!(f, "   = help: {}", help)?;
        }

        Ok(())
    }
}

/// The result of a validation operation.
///
/// Contains all diagnostics found during validation. Validation
/// reports ALL errors, not just the first one.
///
/// Invariants:
/// - `is_ok()` returns true only if no `Error`-severity diagnostics exist.
/// - `diagnostics` is sorted by file path, then line number.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    /// All diagnostics collected during validation.
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidationResult {
    /// Create an empty (passing) result.
    pub fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    /// Returns true if there are no error-severity diagnostics.
    pub fn is_ok(&self) -> bool {
        !self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }

    /// Returns only the error-severity diagnostics.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
    }

    /// Returns only the warning-severity diagnostics.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
    }

    /// Add a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Merge another result into this one.
    pub fn merge(&mut self, other: ValidationResult) {
        self.diagnostics.extend(other.diagnostics);
    }

    /// Sort diagnostics by file path, then line number.
    pub fn sort(&mut self) {
        self.diagnostics.sort_by(|a, b| {
            let loc_a = a.locations.first();
            let loc_b = b.locations.first();
            match (loc_a, loc_b) {
                (Some(a), Some(b)) => a
                    .location
                    .file
                    .cmp(&b.location.file)
                    .then(a.location.line.cmp(&b.location.line))
                    .then(a.location.column.cmp(&b.location.column)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    }

    /// Format all diagnostics as a single string (for CLI output).
    ///
    /// # COLD PATH
    pub fn format_all(&self) -> String {
        self.diagnostics
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Serialize diagnostics as JSON (for CI/CD integration).
    ///
    /// # COLD PATH
    pub fn to_json(&self) -> String {
        let mut parts = Vec::with_capacity(self.diagnostics.len());
        for d in &self.diagnostics {
            let severity = match d.severity {
                Severity::Hint => "hint",
                Severity::Warning => "warning",
                Severity::Error => "error",
            };
            let locations_json: Vec<String> = d
                .locations
                .iter()
                .map(|l| {
                    format!(
                        r#"{{"file":"{}","line":{},"column":{},"label":"{}"}}"#,
                        l.location.file.display(),
                        l.location.line,
                        l.location.column,
                        l.label.replace('"', "\\\"")
                    )
                })
                .collect();

            parts.push(format!(
                r#"{{"code":"{}","severity":"{}","message":"{}","locations":[{}]{}{}}}"#,
                d.code,
                severity,
                d.message.replace('"', "\\\""),
                locations_json.join(","),
                if d.notes.is_empty() {
                    String::new()
                } else {
                    format!(
                        r#","notes":[{}]"#,
                        d.notes
                            .iter()
                            .map(|n| format!(r#""{}""#, n.replace('"', "\\\"")))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                },
                match &d.help {
                    Some(h) => format!(r#","help":"{}""#, h.replace('"', "\\\"")),
                    None => String::new(),
                },
            ));
        }
        format!("[{}]", parts.join(","))
    }
}

/// Convenience builder for creating diagnostics.
///
/// # Example
///
/// ```rust
/// use torvyn_contracts::errors::*;
///
/// let diag = DiagnosticBuilder::error(ErrorCode::WitSyntaxError, "unexpected token")
///     .location("types.wit", 42, 10, "expected `}`")
///     .help("check for missing closing braces in the interface definition")
///     .build();
/// ```
pub struct DiagnosticBuilder {
    inner: Diagnostic,
}

impl DiagnosticBuilder {
    /// Create a new error diagnostic.
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            inner: Diagnostic {
                code,
                severity: Severity::Error,
                message: message.into(),
                locations: Vec::new(),
                notes: Vec::new(),
                help: None,
            },
        }
    }

    /// Create a new warning diagnostic.
    pub fn warning(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            inner: Diagnostic {
                code,
                severity: Severity::Warning,
                message: message.into(),
                locations: Vec::new(),
                notes: Vec::new(),
                help: None,
            },
        }
    }

    /// Add a source location with a label.
    pub fn location(
        mut self,
        file: impl Into<PathBuf>,
        line: u32,
        column: u32,
        label: impl Into<String>,
    ) -> Self {
        self.inner.locations.push(LocationContext {
            location: SourceLocation {
                file: file.into(),
                line,
                column,
            },
            label: label.into(),
        });
        self
    }

    /// Add a note.
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.inner.notes.push(note.into());
        self
    }

    /// Set the help text.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.inner.help = Some(help.into());
        self
    }

    /// Build the diagnostic.
    pub fn build(self) -> Diagnostic {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_formatting() {
        assert_eq!(ErrorCode::WitSyntaxError.as_str(), "E0100");
        assert_eq!(ErrorCode::ManifestParseError.as_str(), "E0110");
        assert_eq!(ErrorCode::IncompatibleMajorVersion.as_str(), "E0140");
        assert_eq!(ErrorCode::InvalidTopology.as_str(), "E0160");
    }

    #[test]
    fn test_error_code_display() {
        assert_eq!(format!("{}", ErrorCode::WitSyntaxError), "E0100");
    }

    #[test]
    fn test_source_location_display_with_column() {
        let loc = SourceLocation {
            file: PathBuf::from("types.wit"),
            line: 42,
            column: 10,
        };
        assert_eq!(loc.to_string(), "types.wit:42:10");
    }

    #[test]
    fn test_source_location_display_without_column() {
        let loc = SourceLocation {
            file: PathBuf::from("types.wit"),
            line: 42,
            column: 0,
        };
        assert_eq!(loc.to_string(), "types.wit:42");
    }

    #[test]
    fn test_validation_result_empty_is_ok() {
        let result = ValidationResult::new();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_result_with_warning_is_ok() {
        let mut result = ValidationResult::new();
        result.push(
            DiagnosticBuilder::warning(ErrorCode::CapabilityMismatch, "unused capability")
                .build(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_result_with_error_is_not_ok() {
        let mut result = ValidationResult::new();
        result.push(
            DiagnosticBuilder::error(ErrorCode::WitSyntaxError, "bad syntax")
                .location("types.wit", 1, 1, "here")
                .build(),
        );
        assert!(!result.is_ok());
    }

    #[test]
    fn test_diagnostic_builder_full() {
        let diag = DiagnosticBuilder::error(
            ErrorCode::IncompatibleMajorVersion,
            "incompatible contract versions",
        )
        .location(
            "pipeline.toml",
            12,
            5,
            "component-a exports torvyn:streaming@0.1.0",
        )
        .location(
            "pipeline.toml",
            13,
            5,
            "component-b requires torvyn:streaming@0.2.0",
        )
        .note("component-b uses `output-element.priority` (added in 0.2.0)")
        .help("upgrade component-a to a version compiled against torvyn:streaming@0.2.0 or later")
        .build();

        assert_eq!(diag.code, ErrorCode::IncompatibleMajorVersion);
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.locations.len(), 2);
        assert_eq!(diag.notes.len(), 1);
        assert!(diag.help.is_some());

        let formatted = diag.to_string();
        assert!(formatted.contains("E0140"));
        assert!(formatted.contains("incompatible contract versions"));
        assert!(formatted.contains("pipeline.toml:12:5"));
    }

    #[test]
    fn test_validation_result_merge() {
        let mut a = ValidationResult::new();
        a.push(DiagnosticBuilder::error(ErrorCode::WitSyntaxError, "error a").build());

        let mut b = ValidationResult::new();
        b.push(DiagnosticBuilder::error(ErrorCode::ManifestParseError, "error b").build());

        a.merge(b);
        assert_eq!(a.diagnostics.len(), 2);
    }

    #[test]
    fn test_validation_result_sort() {
        let mut result = ValidationResult::new();
        result.push(
            DiagnosticBuilder::error(ErrorCode::WitSyntaxError, "second")
                .location("b.wit", 10, 1, "here")
                .build(),
        );
        result.push(
            DiagnosticBuilder::error(ErrorCode::WitSyntaxError, "first")
                .location("a.wit", 5, 1, "here")
                .build(),
        );

        result.sort();
        assert_eq!(result.diagnostics[0].message, "first");
        assert_eq!(result.diagnostics[1].message, "second");
    }

    #[test]
    fn test_validation_result_to_json() {
        let mut result = ValidationResult::new();
        result.push(
            DiagnosticBuilder::error(ErrorCode::WitSyntaxError, "bad syntax")
                .location("types.wit", 1, 1, "unexpected token")
                .help("check syntax")
                .build(),
        );

        let json = result.to_json();
        assert!(json.contains("E0100"));
        assert!(json.contains("bad syntax"));
        assert!(json.contains("types.wit"));
    }

    #[test]
    fn test_errors_iterator() {
        let mut result = ValidationResult::new();
        result.push(
            DiagnosticBuilder::warning(ErrorCode::CapabilityMismatch, "warn").build(),
        );
        result.push(DiagnosticBuilder::error(ErrorCode::WitSyntaxError, "err").build());

        assert_eq!(result.errors().count(), 1);
        assert_eq!(result.warnings().count(), 1);
    }
}
