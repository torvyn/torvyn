//! # torvyn-config
//!
//! Configuration parsing, validation, and schema definitions for the
//! Torvyn reactive streaming runtime.
//!
//! This crate implements the **two-configuration-context model** recommended
//! by the consolidated HLI review (Doc 10, Recommendation 3):
//!
//! 1. **Component Manifest** — `Torvyn.toml` in a component project root.
//!    Contains project metadata, component declarations, build/test config,
//!    and optional inline pipeline definitions.
//!
//! 2. **Pipeline Definition** — either `[flow.*]` tables inline in
//!    `Torvyn.toml` or a standalone `pipeline.toml`. Contains flow topology,
//!    per-component overrides, scheduling, backpressure, and resource limits.
//!
//! # Quick Start
//!
//! ```no_run
//! use torvyn_config::load_config;
//!
//! let config = load_config("./Torvyn.toml").unwrap();
//! println!("Project: {}", config.manifest.torvyn.name);
//! for (name, flow) in &config.flows {
//!     println!("  Flow: {name} ({} nodes)", flow.nodes.len());
//! }
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(clippy::pedantic)]
// Allow pedantic lints that conflict with the LLI design choices:
// - ConfigParseError is intentionally large for rich diagnostics (cold path only)
// - Merge functions use != default pattern for clarity
// - Constructor methods are always consumed immediately
#![allow(clippy::result_large_err)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::if_not_else)]
#![allow(clippy::module_name_repetitions)]

pub mod env;
pub mod error;
pub mod loader;
pub mod manifest;
pub mod merge;
pub mod pipeline;
pub mod runtime;
pub mod validate;

// Re-exports for convenience
pub use env::{collect_env_overrides, interpolate_env};
pub use error::{ConfigErrors, ConfigParseError};
pub use loader::{load_config, load_manifest, load_pipeline, ResolvedConfig};
pub use manifest::{
    BuildConfig, ComponentDecl, ComponentManifest, ProjectMetadata, RegistryConfig, TestConfig,
};
pub use merge::{
    merge_backpressure_config, merge_observability_config, merge_runtime_config,
    merge_scheduling_config, merge_security_config,
};
pub use pipeline::{EdgeDef, EdgeEndpoint, FlowDef, NodeDef, PipelineDefinition};
pub use runtime::{
    parse_memory_size, BackpressureConfig, CapabilityGrant, ObservabilityConfig, RuntimeConfig,
    SchedulingConfig, SecurityConfig,
};
pub use validate::{validate_manifest, validate_pipeline};
