//! # torvyn-security
//!
//! Capabilities, sandboxing, and audit logging for the Torvyn runtime.
//!
//! This crate implements the security model defined in Doc 06 of the Torvyn
//! High-Level Implementation design. It provides:
//!
//! - **Capability taxonomy** — a typed enum of all WASI and Torvyn-specific
//!   permissions, with scoping types for filesystem, network, and resource pools.
//! - **Capability resolution** — the algorithm that intersects component-declared
//!   capabilities with operator grants to produce a `ResolvedCapabilitySet`.
//! - **Runtime enforcement** — `CapabilityGuard` for cold-path checks and
//!   `HotPathCapabilities` for zero-overhead per-element checks.
//! - **Sandbox configuration** — `SandboxConfig` and `SandboxConfigurator` that
//!   produce the complete security configuration consumed by the host runtime.
//! - **Audit logging** — structured `AuditEvent` records emitted to pluggable
//!   `AuditSink` backends.
//! - **Multi-tenant hooks** — `TenantId` type for Phase 2 isolation readiness.
//!
//! # Security Principles
//!
//! - **Deny-all by default.** A component with no grants can do nothing beyond
//!   pure computation.
//! - **Fail closed.** Ambiguous states result in denial.
//! - **Audit everything.** Every exercise and denial is recorded.
//!
//! # Example
//!
//! ```
//! use torvyn_security::{
//!     Capability, ComponentCapabilities, OperatorGrants,
//!     DefaultCapabilityResolver, DefaultSandboxConfigurator,
//!     SandboxConfigurator, CpuBudget, ResourceBudget,
//!     TenantId, AuditSinkHandle,
//! };
//! use torvyn_types::ComponentId;
//!
//! // Component declares its needs
//! let caps = ComponentCapabilities::new(
//!     vec![Capability::WallClock, Capability::MonotonicClock],
//!     vec![Capability::Stderr],
//! );
//!
//! // Operator grants permissions
//! let grants = OperatorGrants::new(vec![
//!     Capability::WallClock,
//!     Capability::MonotonicClock,
//! ]);
//!
//! // Resolve -> only wall and mono clocks; stderr is optional-not-granted
//! let result = DefaultCapabilityResolver::resolve(&caps, &grants).unwrap();
//! assert_eq!(result.resolved.len(), 2);
//! assert_eq!(result.warnings.len(), 1);
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod audit;
pub mod capability;
pub mod error;
pub mod guard;
pub mod manifest;
pub mod resolver;
pub mod sandbox;
pub mod tenant;

// Re-exports for ergonomic use
#[cfg(feature = "audit-file")]
pub use audit::FileAuditSink;
pub use audit::{AuditEvent, AuditEventKind, AuditSeverity, AuditSink, AuditSinkHandle};
pub use audit::{NoopAuditSink, StdoutAuditSink};
pub use capability::{Capability, CapabilityParseError, NetScope, PathScope, PoolScope, PortRange};
pub use error::{AuditError, CapabilityResolutionError, ResolutionWarning, SandboxError};
pub use guard::{CapabilityDenied, CapabilityGuard, HotPathCapabilities};
pub use manifest::{ComponentCapabilities, OperatorGrants};
pub use resolver::{DefaultCapabilityResolver, ResolutionResult, ResolvedCapabilitySet};
pub use sandbox::{
    CpuBudget, DefaultSandboxConfigurator, PreopenedDir, ResourceBudget, SandboxConfig,
    SandboxConfigurator, WasiConfiguration,
};
pub use tenant::TenantId;
