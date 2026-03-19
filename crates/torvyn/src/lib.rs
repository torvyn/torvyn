//! # Torvyn
//!
//! **Ownership-aware reactive streaming runtime for WebAssembly components.**
//!
//! Torvyn composes sandboxed WebAssembly components into low-latency,
//! single-node streaming pipelines with contract-first composition,
//! host-managed resource ownership, and reactive backpressure.
//!
//! This is the umbrella crate — it re-exports the entire Torvyn public API.
//! For finer-grained dependencies, use the individual `torvyn-*` crates.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use torvyn::prelude::*;
//!
//! let host = HostBuilder::new()
//!     .with_config(HostConfig::default())
//!     .build()?;
//! ```
//!
//! ## Crate Organization
//!
//! | Module | Crate | Description |
//! |--------|-------|-------------|
//! | [`types`] | `torvyn-types` | Identity types, errors, enums, state machines |
//! | [`config`] | `torvyn-config` | Configuration parsing and validation |
//! | [`contracts`] | `torvyn-contracts` | WIT contract loading and validation |
//! | [`engine`] | `torvyn-engine` | Wasm engine abstraction and invocation |
//! | [`resources`] | `torvyn-resources` | Buffer pools and ownership tracking |
//! | [`security`] | `torvyn-security` | Capability model and sandboxing |
//! | [`observability`] | `torvyn-observability` | Metrics, tracing, and export |
//! | [`reactor`] | `torvyn-reactor` | Stream scheduling and flow lifecycle |
//! | [`linker`] | `torvyn-linker` | Component linking and composition |
//! | [`pipeline`] | `torvyn-pipeline` | Pipeline topology construction |
//! | [`packaging`] | `torvyn-packaging` | OCI artifact assembly and distribution |
//! | [`host`] | `torvyn-host` | Runtime orchestration |
//!
//! ## Feature Flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `cli` | Yes | Includes the `torvyn` binary. Disable for library-only usage. |

#![forbid(unsafe_code)]
#![deny(missing_docs)]

// ---------------------------------------------------------------------------
// Subsystem crate re-exports
// ---------------------------------------------------------------------------

/// Core identity types, error enums, state machines, and shared traits.
pub use torvyn_types as types;

/// Configuration parsing, validation, and schema definitions.
pub use torvyn_config as config;

/// WIT contract loading, validation, and compatibility checking.
pub use torvyn_contracts as contracts;

/// Wasm engine abstraction and component invocation.
pub use torvyn_engine as engine;

/// Buffer pools, ownership tracking, and copy accounting.
pub use torvyn_resources as resources;

/// Capability model, sandboxing, and audit logging.
pub use torvyn_security as security;

/// Metrics, tracing, OTLP export, and benchmark reporting.
pub use torvyn_observability as observability;

/// Stream scheduling, backpressure, and flow lifecycle.
pub use torvyn_reactor as reactor;

/// Component linking and pipeline composition.
pub use torvyn_linker as linker;

/// Pipeline topology construction, validation, and instantiation.
pub use torvyn_pipeline as pipeline;

/// OCI artifact assembly, signing, and distribution.
pub use torvyn_packaging as packaging;

/// Runtime orchestration — the main entry point for running Torvyn.
pub use torvyn_host as host;

// ---------------------------------------------------------------------------
// Prelude — commonly used types for convenient glob imports
// ---------------------------------------------------------------------------

/// Commonly used types and traits for convenient glob imports.
///
/// ```rust,ignore
/// use torvyn::prelude::*;
/// ```
pub mod prelude {
    // Identity types
    pub use torvyn_types::{
        BufferHandle, ComponentInstanceId, ComponentTypeId, FlowId, ResourceId, SpanId, StreamId,
        TraceId,
    };

    // Core enums
    pub use torvyn_types::{
        BackpressurePolicy, BackpressureSignal, ComponentRole, FlowState, ObservabilityLevel,
        ResourceState,
    };

    // Core error
    pub use torvyn_types::{ProcessError, TorvynError};

    // Core trait
    pub use torvyn_types::EventSink;

    // Host runtime
    pub use torvyn_host::{HostBuilder, HostConfig, HostStatus, TorvynHost};

    // Engine traits
    pub use torvyn_engine::{ComponentInvoker, WasmEngine};

    // Configuration
    pub use torvyn_config::{ComponentManifest, PipelineDefinition, RuntimeConfig};
}
