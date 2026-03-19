//! # torvyn-contracts
//!
//! WIT contracts and validation for the Torvyn streaming runtime.
//!
//! This crate provides:
//! - The canonical WIT files for `torvyn:streaming@0.1.0`
//! - Contract validation for `torvyn check` (single-component)
//! - Version compatibility checking
//! - Link validation for `torvyn link` (multi-component composition)
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use std::path::Path;
//! use torvyn_contracts::{validate_component, WitParserImpl};
//!
//! let parser = WitParserImpl::new();
//! let result = validate_component(Path::new("my-component/"), &parser);
//! if result.is_ok() {
//!     println!("Component is valid!");
//! } else {
//!     eprintln!("{}", result.format_all());
//! }
//! ```
//!
//! ## Error Codes
//!
//! All error codes are in the range E0100–E0199:
//! - E0100–E0109: Parse errors
//! - E0110–E0119: Manifest errors
//! - E0120–E0139: Semantic validation errors
//! - E0140–E0159: Compatibility errors
//! - E0160–E0179: Link errors

#![deny(missing_docs)]

pub mod compatibility;
pub mod errors;
pub mod linker;
pub mod parser;
pub mod validator;

// Re-export key types for convenience
pub use compatibility::{check_compatibility, CompatibilityReport, CompatibilityVerdict};
pub use errors::{
    Diagnostic, DiagnosticBuilder, ErrorCode, Severity, SourceLocation, ValidationResult,
};
pub use linker::{validate_pipeline, PipelineComponent, PipelineConnection, PipelineDefinition};
pub use parser::{ParsedInterface, ParsedPackage, ParsedWorld, WitParser};
pub use validator::{parse_manifest, validate_component, ComponentManifest};

#[cfg(feature = "wit-parser-backend")]
pub use parser::WitParserImpl;

/// Returns the path to the bundled Torvyn streaming WIT files.
///
/// These are the canonical WIT files committed to the repository.
///
/// # COLD PATH
pub fn wit_streaming_path() -> String {
    format!("{}/wit/torvyn-streaming", env!("CARGO_MANIFEST_DIR"))
}

/// Returns the path to the bundled Torvyn filtering WIT files.
///
/// # COLD PATH
pub fn wit_filtering_path() -> String {
    format!("{}/wit-ext/torvyn-filtering", env!("CARGO_MANIFEST_DIR"))
}

/// Returns the path to the bundled Torvyn aggregation WIT files.
///
/// # COLD PATH
pub fn wit_aggregation_path() -> String {
    format!("{}/wit-ext/torvyn-aggregation", env!("CARGO_MANIFEST_DIR"))
}

/// Returns the path to the bundled Torvyn capabilities WIT files.
///
/// # COLD PATH
pub fn wit_capabilities_path() -> String {
    format!("{}/wit-ext/torvyn-capabilities", env!("CARGO_MANIFEST_DIR"))
}
