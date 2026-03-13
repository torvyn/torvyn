//! # torvyn-types
//!
//! Shared foundation types for the Torvyn reactive streaming runtime.
//!
//! This crate is the universal leaf dependency — every other Torvyn crate
//! imports from it. It contains identity types, error types, domain enums,
//! state machines, shared records, traits, and constants.
//!
//! ## Design principles
//! - **Zero internal dependencies**: this crate depends only on `std` and `serde`.
//! - **Zero unsafe code**: `#![forbid(unsafe_code)]`.
//! - **Complete documentation**: `#![deny(missing_docs)]`.
//! - **Minimal footprint**: compiles in under 2 seconds.
//!
//! ## Quick reference
//!
//! | Category | Key types |
//! |----------|-----------|
//! | Identity | [`ComponentTypeId`], [`ComponentInstanceId`], [`ComponentId`], [`FlowId`], [`StreamId`], [`ResourceId`], [`BufferHandle`], [`TraceId`], [`SpanId`] |
//! | Errors | [`ProcessError`], [`TorvynError`], [`ContractError`], [`LinkError`], [`ResourceError`], [`ReactorError`], [`ConfigError`], [`SecurityError`], [`PackagingError`] |
//! | Enums | [`ComponentRole`], [`BackpressureSignal`], [`BackpressurePolicy`], [`ObservabilityLevel`], [`Severity`], [`CopyReason`] |
//! | State machines | [`FlowState`], [`ResourceState`], [`InvalidTransition`] |
//! | Records | [`ElementMeta`], [`TransferRecord`], [`TraceContext`] |
//! | Traits | [`EventSink`], [`NoopEventSink`] |

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

// Module declarations
mod constants;
mod enums;
mod error;
mod identity;
mod records;
mod state;
mod timestamp;
mod traits;

// --- Identity types ---
pub use identity::{
    BufferHandle, ComponentId, ComponentInstanceId, ComponentTypeId, FlowId, ResourceId, SpanId,
    StreamId, TraceId,
};

// --- Error types ---
pub use error::{
    ConfigError, ContractError, EngineError, LinkError, PackagingError, ProcessError,
    ProcessErrorKind, ReactorError, ResourceError, SecurityError, TorvynError,
};

// --- Domain enums ---
pub use enums::{
    BackpressurePolicy, BackpressureSignal, ComponentRole, CopyReason, ObservabilityLevel, Severity,
};

// --- State machines ---
pub use state::{FlowState, InvalidTransition, ResourceState};

// --- Records ---
pub use records::{ElementMeta, TraceContext, TransferRecord};

// --- Traits ---
pub use traits::{EventSink, InvocationStatus, NoopEventSink};

// --- Constants ---
pub use constants::*;

// --- Timestamp utilities ---
pub use timestamp::current_timestamp_ns;
