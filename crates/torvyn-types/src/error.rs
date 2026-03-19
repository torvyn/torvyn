//! Shared error types for the Torvyn runtime.
//!
//! Error hierarchy:
//! - [`ProcessError`] — Rust mapping of the WIT `process-error` variant (5 variants).
//! - [`TorvynError`] — Top-level error enum aggregating all subsystem errors.
//! - Subsystem-specific errors: [`ContractError`], [`LinkError`], [`ResourceError`],
//!   [`ReactorError`], [`ConfigError`], [`SecurityError`], [`PackagingError`].
//!
//! All error types implement `Display` with actionable messages and `std::error::Error`.

use crate::{ComponentId, FlowId, ResourceId, StreamId};
use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ProcessError — WIT process-error mapping
// ---------------------------------------------------------------------------

/// Rust mapping of the WIT `process-error` variant.
///
/// This type has exactly 5 variants matching the WIT definition in
/// `torvyn:streaming@0.1.0` (Doc 01, Section 3.1). It is the error type
/// returned by component invocations (process, pull, push).
///
/// # Variants
/// - `InvalidInput` — input element was malformed or violated the contract.
/// - `Unavailable` — a required resource or service was unavailable.
/// - `Internal` — unexpected internal error within the component.
/// - `DeadlineExceeded` — the processing deadline has passed.
/// - `Fatal` — the component is permanently unable to process further elements.
///
/// # Examples
/// ```
/// use torvyn_types::ProcessError;
///
/// let err = ProcessError::InvalidInput("expected JSON, got binary".into());
/// assert!(format!("{}", err).contains("expected JSON"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ProcessError {
    /// The input element was malformed or violated the expected contract.
    InvalidInput(String),
    /// A resource or service required by the component was unavailable.
    Unavailable(String),
    /// An unexpected internal error occurred within the component.
    Internal(String),
    /// The component's processing deadline has been exceeded.
    DeadlineExceeded,
    /// The component is permanently unable to process further elements.
    /// This is a terminal error — the runtime will not send more elements.
    Fatal(String),
}

impl ProcessError {
    /// Returns `true` if this is a terminal error (the component should be removed).
    ///
    /// # HOT PATH — called per error to determine flow control action.
    #[inline]
    pub fn is_fatal(&self) -> bool {
        matches!(self, ProcessError::Fatal(_))
    }

    /// Returns the error kind as a static string for metrics and logging.
    ///
    /// # HOT PATH — called for observability categorization.
    #[inline]
    pub fn kind(&self) -> &'static str {
        match self {
            ProcessError::InvalidInput(_) => "invalid_input",
            ProcessError::Unavailable(_) => "unavailable",
            ProcessError::Internal(_) => "internal",
            ProcessError::DeadlineExceeded => "deadline_exceeded",
            ProcessError::Fatal(_) => "fatal",
        }
    }
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessError::InvalidInput(msg) => {
                write!(f, "process error: invalid input \u{2014} {msg}. \
                       Check that upstream components produce elements matching this component's expected schema.")
            }
            ProcessError::Unavailable(msg) => {
                write!(f, "process error: resource unavailable \u{2014} {msg}. \
                       Verify that required capabilities are granted and downstream services are reachable.")
            }
            ProcessError::Internal(msg) => {
                write!(f, "process error: internal error \u{2014} {msg}. \
                       This indicates a bug in the component. Check component logs and consider filing a bug report.")
            }
            ProcessError::DeadlineExceeded => {
                write!(f, "process error: deadline exceeded. \
                       The component did not complete within its allotted time. \
                       Consider increasing the timeout or optimizing the component's processing logic.")
            }
            ProcessError::Fatal(msg) => {
                write!(f, "process error: FATAL \u{2014} {msg}. \
                       The component has entered a terminal failure state and cannot process further elements. \
                       The flow will be terminated.")
            }
        }
    }
}

impl std::error::Error for ProcessError {}

// ---------------------------------------------------------------------------
// ProcessErrorKind — lightweight variant tag for observability
// ---------------------------------------------------------------------------

/// Lightweight variant tag for `ProcessError`, used in observability paths
/// where carrying the full error string is too expensive.
///
/// # HOT PATH — used in `EventSink::record_invocation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ProcessErrorKind {
    /// Corresponds to `ProcessError::InvalidInput`.
    InvalidInput,
    /// Corresponds to `ProcessError::Unavailable`.
    Unavailable,
    /// Corresponds to `ProcessError::Internal`.
    Internal,
    /// Corresponds to `ProcessError::DeadlineExceeded`.
    DeadlineExceeded,
    /// Corresponds to `ProcessError::Fatal`.
    Fatal,
}

impl From<&ProcessError> for ProcessErrorKind {
    #[inline]
    fn from(err: &ProcessError) -> Self {
        match err {
            ProcessError::InvalidInput(_) => ProcessErrorKind::InvalidInput,
            ProcessError::Unavailable(_) => ProcessErrorKind::Unavailable,
            ProcessError::Internal(_) => ProcessErrorKind::Internal,
            ProcessError::DeadlineExceeded => ProcessErrorKind::DeadlineExceeded,
            ProcessError::Fatal(_) => ProcessErrorKind::Fatal,
        }
    }
}

// ---------------------------------------------------------------------------
// Subsystem-specific error types
// ---------------------------------------------------------------------------

/// Contract validation errors.
///
/// Error code range: E0100–E0199 (per Doc 09, G-08).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractError {
    /// A WIT file could not be parsed.
    ParseError {
        /// The file that failed to parse.
        file: String,
        /// The line number where the error occurred.
        line: u32,
        /// A description of the parse error.
        message: String,
    },
    /// A required interface is missing from the component.
    MissingInterface {
        /// The component that is missing the interface.
        component: String,
        /// The name of the missing interface.
        interface_name: String,
    },
    /// A type in the component does not match the expected contract.
    TypeMismatch {
        /// The interface where the mismatch occurred.
        interface_name: String,
        /// The name of the mismatched type.
        type_name: String,
        /// The expected type.
        expected: String,
        /// The actual type found.
        actual: String,
    },
    /// The contract version is incompatible.
    VersionIncompatible {
        /// The required version.
        required: String,
        /// The provided version.
        provided: String,
    },
    /// A WIT package could not be resolved.
    PackageNotFound {
        /// The name of the missing package.
        package_name: String,
    },
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractError::ParseError {
                file,
                line,
                message,
            } => {
                write!(
                    f,
                    "[E0100] Contract parse error in '{file}' at line {line}: {message}. \
                       Fix the WIT syntax and re-run `torvyn check`."
                )
            }
            ContractError::MissingInterface {
                component,
                interface_name,
            } => {
                write!(f, "[E0101] Component '{component}' does not export required interface '{interface_name}'. \
                       Ensure the component's world definition includes `export {interface_name}`.")
            }
            ContractError::TypeMismatch {
                interface_name,
                type_name,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "[E0102] Type mismatch in interface '{interface_name}': type '{type_name}' \
                       expected '{expected}' but found '{actual}'. \
                       Recompile the component against the correct WIT package version."
                )
            }
            ContractError::VersionIncompatible { required, provided } => {
                write!(
                    f,
                    "[E0103] Contract version incompatible: required '{required}', \
                       provided '{provided}'. \
                       Update the component to target the required contract version."
                )
            }
            ContractError::PackageNotFound { package_name } => {
                write!(
                    f,
                    "[E0104] WIT package '{package_name}' not found. \
                       Run `torvyn init` to fetch dependencies or check the `wit/deps/` directory."
                )
            }
        }
    }
}

impl std::error::Error for ContractError {}

/// Component linking errors.
///
/// Error code range: E0200–E0299.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// A component's required import is not satisfied by any export.
    UnresolvedImport {
        /// The component with the unresolved import.
        component: String,
        /// The name of the unresolved import.
        import_name: String,
    },
    /// Two connected components have incompatible interfaces.
    InterfaceMismatch {
        /// The source component.
        from_component: String,
        /// The destination component.
        to_component: String,
        /// The mismatched interface.
        interface_name: String,
        /// Details about the mismatch.
        detail: String,
    },
    /// A capability required by the component was not granted.
    CapabilityDenied {
        /// The component that needs the capability.
        component: String,
        /// The denied capability.
        capability: String,
    },
    /// The pipeline topology contains a cycle.
    CyclicDependency {
        /// The components forming the cycle.
        cycle: Vec<String>,
    },
    /// A component could not be compiled.
    CompilationFailed {
        /// The component that failed to compile.
        component: String,
        /// The reason for compilation failure.
        reason: String,
    },
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::UnresolvedImport {
                component,
                import_name,
            } => {
                write!(
                    f,
                    "[E0200] Unresolved import: component '{component}' requires '{import_name}' \
                       but no connected component exports it. \
                       Check the pipeline topology and ensure all imports are satisfied."
                )
            }
            LinkError::InterfaceMismatch {
                from_component,
                to_component,
                interface_name,
                detail,
            } => {
                write!(
                    f,
                    "[E0201] Interface mismatch between '{from_component}' and '{to_component}' \
                       on interface '{interface_name}': {detail}. \
                       Ensure both components target the same contract version."
                )
            }
            LinkError::CapabilityDenied {
                component,
                capability,
            } => {
                write!(
                    f,
                    "[E0202] Capability denied: component '{component}' requires '{capability}' \
                       but it was not granted in the pipeline configuration. \
                       Add the capability to the component's grant list in Torvyn.toml."
                )
            }
            LinkError::CyclicDependency { cycle } => {
                write!(
                    f,
                    "[E0203] Cyclic dependency detected in pipeline topology: {}. \
                       Torvyn pipelines must be directed acyclic graphs (DAGs). \
                       Remove the cycle by restructuring the pipeline.",
                    cycle.join(" \u{2192} ")
                )
            }
            LinkError::CompilationFailed { component, reason } => {
                write!(
                    f,
                    "[E0204] Component '{component}' failed to compile: {reason}. \
                       Verify the .wasm file is a valid WebAssembly Component."
                )
            }
        }
    }
}

impl std::error::Error for LinkError {}

/// Resource management errors.
///
/// Error code range: E0300–E0399.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceError {
    /// The resource handle refers to a slot that has been freed and reused.
    StaleHandle {
        /// The stale handle.
        handle: ResourceId,
    },
    /// The caller is not the owner of the resource.
    NotOwner {
        /// The resource handle.
        handle: ResourceId,
        /// The expected owner.
        expected_owner: String,
        /// The actual caller.
        actual_caller: String,
    },
    /// The resource is not currently allocated (still in pool).
    NotAllocated {
        /// The resource handle.
        handle: ResourceId,
    },
    /// Outstanding borrows prevent the requested operation.
    BorrowsOutstanding {
        /// The resource handle.
        handle: ResourceId,
        /// Number of outstanding borrows.
        borrow_count: u32,
    },
    /// A mutable lease is active, preventing the requested operation.
    MutableLeaseActive {
        /// The resource handle.
        handle: ResourceId,
    },
    /// Read-only leases are active, preventing the requested operation.
    ReadOnlyLeasesActive {
        /// The resource handle.
        handle: ResourceId,
    },
    /// The component's memory budget would be exceeded.
    BudgetExceeded {
        /// The component exceeding its budget.
        component: ComponentId,
        /// Current memory usage in bytes.
        current_bytes: u64,
        /// Requested additional bytes.
        requested_bytes: u64,
        /// Total budget in bytes.
        budget_bytes: u64,
    },
    /// Buffer pool is exhausted and system allocator fallback failed.
    AllocationFailed {
        /// The requested buffer capacity.
        requested_capacity: u32,
        /// The reason for failure.
        reason: String,
    },
    /// The write would exceed the buffer's capacity.
    CapacityExceeded {
        /// The resource handle.
        handle: ResourceId,
        /// The buffer capacity.
        capacity: u32,
        /// The attempted write size.
        attempted_size: u64,
    },
    /// The offset is out of bounds.
    OutOfBounds {
        /// The resource handle.
        handle: ResourceId,
        /// The invalid offset.
        offset: u64,
        /// The buffer size.
        buffer_size: u64,
    },
}

impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceError::StaleHandle { handle } => {
                write!(
                    f,
                    "[E0300] Stale resource handle: {handle}. The slot has been freed and reused. \
                       This usually indicates a bug in the component or a lifecycle mismatch."
                )
            }
            ResourceError::NotOwner {
                handle,
                expected_owner,
                actual_caller,
            } => {
                write!(f, "[E0301] Not owner of resource {handle}: expected owner '{expected_owner}', \
                       caller is '{actual_caller}'. Only the owner can transfer, free, or mutate a resource.")
            }
            ResourceError::NotAllocated { handle } => {
                write!(
                    f,
                    "[E0302] Resource {handle} is not allocated (currently pooled). \
                       Allocate the resource before attempting to use it."
                )
            }
            ResourceError::BorrowsOutstanding {
                handle,
                borrow_count,
            } => {
                write!(f, "[E0303] Cannot modify resource {handle}: {borrow_count} borrow(s) outstanding. \
                       Wait for all borrows to be released before transferring or freeing.")
            }
            ResourceError::MutableLeaseActive { handle } => {
                write!(
                    f,
                    "[E0304] Cannot access resource {handle}: a mutable lease is active. \
                       Wait for the lease to expire or be released."
                )
            }
            ResourceError::ReadOnlyLeasesActive { handle } => {
                write!(
                    f,
                    "[E0305] Cannot mutate resource {handle}: read-only lease(s) active. \
                       Wait for all leases to expire or be released."
                )
            }
            ResourceError::BudgetExceeded {
                component,
                current_bytes,
                requested_bytes,
                budget_bytes,
            } => {
                write!(f, "[E0306] Memory budget exceeded for {component}: \
                       current={current_bytes}B, requested={requested_bytes}B, budget={budget_bytes}B. \
                       Release unused buffers or increase the component's memory budget in Torvyn.toml.")
            }
            ResourceError::AllocationFailed {
                requested_capacity,
                reason,
            } => {
                write!(
                    f,
                    "[E0307] Buffer allocation failed for {requested_capacity}B: {reason}. \
                       Consider increasing pool sizes or reducing concurrent buffer usage."
                )
            }
            ResourceError::CapacityExceeded {
                handle,
                capacity,
                attempted_size,
            } => {
                write!(
                    f,
                    "[E0308] Write would exceed buffer capacity for {handle}: \
                       capacity={capacity}B, attempted_size={attempted_size}B. \
                       Allocate a larger buffer or reduce write size."
                )
            }
            ResourceError::OutOfBounds {
                handle,
                offset,
                buffer_size,
            } => {
                write!(
                    f,
                    "[E0309] Out of bounds access on {handle}: \
                       offset={offset}, buffer_size={buffer_size}B. \
                       Verify offset is within the buffer's valid range."
                )
            }
        }
    }
}

impl std::error::Error for ResourceError {}

/// Reactor and scheduling errors.
///
/// Error code range: E0400–E0499.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReactorError {
    /// The specified flow was not found.
    FlowNotFound {
        /// The flow ID that was not found.
        flow_id: FlowId,
    },
    /// The flow is in a state that does not permit the requested operation.
    InvalidFlowState {
        /// The flow ID.
        flow_id: FlowId,
        /// The current state of the flow.
        current_state: String,
        /// The operation that was attempted.
        attempted_operation: String,
    },
    /// The flow topology is invalid.
    InvalidTopology {
        /// The reason the topology is invalid.
        reason: String,
    },
    /// A timeout occurred.
    Timeout {
        /// The flow ID.
        flow_id: FlowId,
        /// The operation that timed out.
        operation: String,
        /// The duration in milliseconds before timeout.
        duration_ms: u64,
    },
    /// The reactor is shutting down and cannot accept new flows.
    ShuttingDown,
    /// A stream queue operation failed.
    QueueError {
        /// The stream ID.
        stream_id: StreamId,
        /// The reason for the queue error.
        reason: String,
    },
}

impl fmt::Display for ReactorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReactorError::FlowNotFound { flow_id } => {
                write!(
                    f,
                    "[E0400] Flow '{flow_id}' not found. \
                       Verify the flow ID is correct and the flow has not been terminated."
                )
            }
            ReactorError::InvalidFlowState {
                flow_id,
                current_state,
                attempted_operation,
            } => {
                write!(
                    f,
                    "[E0401] Cannot {attempted_operation} on flow '{flow_id}': \
                       current state is '{current_state}'. \
                       Check the flow lifecycle documentation for valid state transitions."
                )
            }
            ReactorError::InvalidTopology { reason } => {
                write!(
                    f,
                    "[E0402] Invalid flow topology: {reason}. \
                       Ensure the pipeline is a directed acyclic graph with valid connections."
                )
            }
            ReactorError::Timeout {
                flow_id,
                operation,
                duration_ms,
            } => {
                write!(f, "[E0403] Timeout on flow '{flow_id}' during '{operation}' \
                       after {duration_ms}ms. Consider increasing timeouts in the flow configuration.")
            }
            ReactorError::ShuttingDown => {
                write!(
                    f,
                    "[E0404] Reactor is shutting down and cannot accept new flows. \
                       Wait for shutdown to complete before restarting."
                )
            }
            ReactorError::QueueError { stream_id, reason } => {
                write!(f, "[E0405] Stream queue error on '{stream_id}': {reason}.")
            }
        }
    }
}

impl std::error::Error for ReactorError {}

/// Configuration errors.
///
/// Error code range: E0700–E0799.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The configuration file could not be read.
    FileNotFound {
        /// The path that was not found.
        path: String,
    },
    /// The configuration file is not valid TOML.
    ParseError {
        /// The path of the invalid file.
        path: String,
        /// The parse error message.
        message: String,
    },
    /// A required configuration field is missing.
    MissingField {
        /// The missing field name.
        field: String,
        /// The context where the field is expected.
        context: String,
    },
    /// A configuration value is invalid.
    InvalidValue {
        /// The field with the invalid value.
        field: String,
        /// The invalid value.
        value: String,
        /// The reason the value is invalid.
        reason: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::FileNotFound { path } => {
                write!(
                    f,
                    "[E0700] Configuration file not found: '{path}'. \
                       Create a Torvyn.toml file or specify a different path with --config."
                )
            }
            ConfigError::ParseError { path, message } => {
                write!(
                    f,
                    "[E0701] Failed to parse configuration '{path}': {message}. \
                       Validate the TOML syntax."
                )
            }
            ConfigError::MissingField { field, context } => {
                write!(
                    f,
                    "[E0702] Missing required field '{field}' in {context}. \
                       Add the field to your configuration file."
                )
            }
            ConfigError::InvalidValue {
                field,
                value,
                reason,
            } => {
                write!(
                    f,
                    "[E0703] Invalid value for '{field}': '{value}' \u{2014} {reason}."
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Security and capability errors.
///
/// Error code range: E0500–E0599.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecurityError {
    /// A capability was not granted.
    CapabilityDenied {
        /// The component that was denied.
        component: ComponentId,
        /// The denied capability.
        capability: String,
    },
    /// The sandbox configuration is invalid.
    InvalidSandboxConfig {
        /// The component with the invalid config.
        component: ComponentId,
        /// The reason the config is invalid.
        reason: String,
    },
    /// A security policy violation was detected at runtime.
    PolicyViolation {
        /// The component that violated the policy.
        component: ComponentId,
        /// Details about the violation.
        detail: String,
    },
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecurityError::CapabilityDenied {
                component,
                capability,
            } => {
                write!(
                    f,
                    "[E0500] Capability '{capability}' denied for {component}. \
                       Grant the capability in the pipeline configuration or component manifest."
                )
            }
            SecurityError::InvalidSandboxConfig { component, reason } => {
                write!(
                    f,
                    "[E0501] Invalid sandbox configuration for {component}: {reason}."
                )
            }
            SecurityError::PolicyViolation { component, detail } => {
                write!(
                    f,
                    "[E0502] Security policy violation by {component}: {detail}. \
                       This event has been logged for auditing."
                )
            }
        }
    }
}

impl std::error::Error for SecurityError {}

/// Packaging and distribution errors.
///
/// Error code range: E0600–E0699.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackagingError {
    /// The artifact format is invalid.
    InvalidArtifact {
        /// The path of the invalid artifact.
        path: String,
        /// The reason the artifact is invalid.
        reason: String,
    },
    /// An OCI registry operation failed.
    RegistryError {
        /// The registry URL.
        registry: String,
        /// The reason for the error.
        reason: String,
    },
    /// Signature verification failed.
    SignatureInvalid {
        /// The artifact whose signature is invalid.
        artifact: String,
        /// The reason the signature is invalid.
        reason: String,
    },
    /// A required metadata field is missing from the artifact.
    MissingMetadata {
        /// The artifact missing metadata.
        artifact: String,
        /// The missing field.
        field: String,
    },
}

impl fmt::Display for PackagingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackagingError::InvalidArtifact { path, reason } => {
                write!(
                    f,
                    "[E0600] Invalid artifact '{path}': {reason}. \
                       Re-pack with `torvyn pack` to create a valid artifact."
                )
            }
            PackagingError::RegistryError { registry, reason } => {
                write!(
                    f,
                    "[E0601] Registry error for '{registry}': {reason}. \
                       Check network connectivity and registry credentials."
                )
            }
            PackagingError::SignatureInvalid { artifact, reason } => {
                write!(
                    f,
                    "[E0602] Signature verification failed for '{artifact}': {reason}. \
                       Re-sign with `torvyn pack --sign` or verify the signing key."
                )
            }
            PackagingError::MissingMetadata { artifact, field } => {
                write!(
                    f,
                    "[E0603] Artifact '{artifact}' is missing required metadata field '{field}'. \
                       Re-pack with a complete Torvyn.toml manifest."
                )
            }
        }
    }
}

impl std::error::Error for PackagingError {}

// ---------------------------------------------------------------------------
// EngineError
// ---------------------------------------------------------------------------

/// Engine errors for Wasm compilation and instantiation.
///
/// Error code range: E0800–E0899.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// A WebAssembly module failed to compile.
    CompilationFailed {
        /// The path or identifier of the module.
        module: String,
        /// The reason for failure.
        reason: String,
    },
    /// A WebAssembly module failed to instantiate.
    InstantiationFailed {
        /// The path or identifier of the module.
        module: String,
        /// The reason for failure.
        reason: String,
    },
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::CompilationFailed { module, reason } => {
                write!(
                    f,
                    "[E0800] Wasm compilation failed for '{module}': {reason}. \
                       Verify the .wasm file is a valid WebAssembly Component."
                )
            }
            EngineError::InstantiationFailed { module, reason } => {
                write!(
                    f,
                    "[E0801] Wasm instantiation failed for '{module}': {reason}. \
                       Check that all imports are satisfied and the module is compatible."
                )
            }
        }
    }
}

impl std::error::Error for EngineError {}

// ---------------------------------------------------------------------------
// TorvynError — top-level error enum
// ---------------------------------------------------------------------------

/// Top-level error type aggregating all subsystem errors.
///
/// This is the primary error type returned by public Torvyn APIs.
/// It provides a uniform error handling surface for the CLI and host binary.
///
/// # Examples
/// ```
/// use torvyn_types::{TorvynError, ProcessError};
///
/// let err: TorvynError = ProcessError::Fatal("disk full".into()).into();
/// assert!(format!("{}", err).contains("FATAL"));
/// ```
#[derive(Debug)]
pub enum TorvynError {
    /// A stream processing error from a component.
    Process(ProcessError),
    /// A contract validation error.
    Contract(ContractError),
    /// A component linking error.
    Link(LinkError),
    /// A resource management error.
    Resource(ResourceError),
    /// A reactor/scheduling error.
    Reactor(ReactorError),
    /// A configuration error.
    Config(ConfigError),
    /// A security/capability error.
    Security(SecurityError),
    /// A packaging/distribution error.
    Packaging(PackagingError),
    /// An engine error.
    Engine(EngineError),
    /// A generic I/O error.
    Io(std::io::Error),
}

impl fmt::Display for TorvynError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TorvynError::Process(e) => write!(f, "{e}"),
            TorvynError::Contract(e) => write!(f, "{e}"),
            TorvynError::Link(e) => write!(f, "{e}"),
            TorvynError::Resource(e) => write!(f, "{e}"),
            TorvynError::Reactor(e) => write!(f, "{e}"),
            TorvynError::Config(e) => write!(f, "{e}"),
            TorvynError::Security(e) => write!(f, "{e}"),
            TorvynError::Packaging(e) => write!(f, "{e}"),
            TorvynError::Engine(e) => write!(f, "{e}"),
            TorvynError::Io(e) => write!(f, "[E0001] I/O error: {e}"),
        }
    }
}

impl std::error::Error for TorvynError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TorvynError::Process(e) => Some(e),
            TorvynError::Contract(e) => Some(e),
            TorvynError::Link(e) => Some(e),
            TorvynError::Resource(e) => Some(e),
            TorvynError::Reactor(e) => Some(e),
            TorvynError::Config(e) => Some(e),
            TorvynError::Security(e) => Some(e),
            TorvynError::Packaging(e) => Some(e),
            TorvynError::Engine(e) => Some(e),
            TorvynError::Io(e) => Some(e),
        }
    }
}

impl From<ProcessError> for TorvynError {
    fn from(e: ProcessError) -> Self {
        TorvynError::Process(e)
    }
}

impl From<ContractError> for TorvynError {
    fn from(e: ContractError) -> Self {
        TorvynError::Contract(e)
    }
}

impl From<LinkError> for TorvynError {
    fn from(e: LinkError) -> Self {
        TorvynError::Link(e)
    }
}

impl From<ResourceError> for TorvynError {
    fn from(e: ResourceError) -> Self {
        TorvynError::Resource(e)
    }
}

impl From<ReactorError> for TorvynError {
    fn from(e: ReactorError) -> Self {
        TorvynError::Reactor(e)
    }
}

impl From<ConfigError> for TorvynError {
    fn from(e: ConfigError) -> Self {
        TorvynError::Config(e)
    }
}

impl From<SecurityError> for TorvynError {
    fn from(e: SecurityError) -> Self {
        TorvynError::Security(e)
    }
}

impl From<PackagingError> for TorvynError {
    fn from(e: PackagingError) -> Self {
        TorvynError::Packaging(e)
    }
}

impl From<EngineError> for TorvynError {
    fn from(e: EngineError) -> Self {
        TorvynError::Engine(e)
    }
}

impl From<std::io::Error> for TorvynError {
    fn from(e: std::io::Error) -> Self {
        TorvynError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- ProcessError ---

    #[test]
    fn test_process_error_invalid_input_display_is_actionable() {
        let err = ProcessError::InvalidInput("expected JSON".into());
        let msg = format!("{err}");
        assert!(
            msg.contains("invalid input"),
            "should contain error category"
        );
        assert!(msg.contains("expected JSON"), "should contain the detail");
        assert!(
            msg.contains("Check that upstream"),
            "should contain remediation"
        );
    }

    #[test]
    fn test_process_error_unavailable_display_is_actionable() {
        let err = ProcessError::Unavailable("database offline".into());
        let msg = format!("{err}");
        assert!(msg.contains("unavailable"));
        assert!(msg.contains("database offline"));
        assert!(msg.contains("capabilities"));
    }

    #[test]
    fn test_process_error_internal_display_is_actionable() {
        let err = ProcessError::Internal("null pointer".into());
        let msg = format!("{err}");
        assert!(msg.contains("internal error"));
        assert!(msg.contains("null pointer"));
        assert!(msg.contains("bug"));
    }

    #[test]
    fn test_process_error_deadline_display_is_actionable() {
        let err = ProcessError::DeadlineExceeded;
        let msg = format!("{err}");
        assert!(msg.contains("deadline exceeded"));
        assert!(msg.contains("timeout"));
    }

    #[test]
    fn test_process_error_fatal_display_is_actionable() {
        let err = ProcessError::Fatal("disk full".into());
        let msg = format!("{err}");
        assert!(msg.contains("FATAL"));
        assert!(msg.contains("disk full"));
        assert!(msg.contains("terminated"));
    }

    #[test]
    fn test_process_error_is_fatal() {
        assert!(!ProcessError::InvalidInput("x".into()).is_fatal());
        assert!(!ProcessError::Unavailable("x".into()).is_fatal());
        assert!(!ProcessError::Internal("x".into()).is_fatal());
        assert!(!ProcessError::DeadlineExceeded.is_fatal());
        assert!(ProcessError::Fatal("x".into()).is_fatal());
    }

    #[test]
    fn test_process_error_kind() {
        assert_eq!(
            ProcessError::InvalidInput("".into()).kind(),
            "invalid_input"
        );
        assert_eq!(ProcessError::Unavailable("".into()).kind(), "unavailable");
        assert_eq!(ProcessError::Internal("".into()).kind(), "internal");
        assert_eq!(ProcessError::DeadlineExceeded.kind(), "deadline_exceeded");
        assert_eq!(ProcessError::Fatal("".into()).kind(), "fatal");
    }

    #[test]
    fn test_process_error_kind_conversion() {
        let err = ProcessError::Fatal("x".into());
        let kind: ProcessErrorKind = (&err).into();
        assert_eq!(kind, ProcessErrorKind::Fatal);
    }

    // --- ContractError ---

    #[test]
    fn test_contract_error_parse_display() {
        let err = ContractError::ParseError {
            file: "types.wit".into(),
            line: 42,
            message: "unexpected token".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0100"));
        assert!(msg.contains("types.wit"));
        assert!(msg.contains("line 42"));
        assert!(msg.contains("torvyn check"));
    }

    #[test]
    fn test_contract_error_missing_interface_display() {
        let err = ContractError::MissingInterface {
            component: "my-processor".into(),
            interface_name: "torvyn:streaming/processor".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0101"));
        assert!(msg.contains("my-processor"));
    }

    // --- LinkError ---

    #[test]
    fn test_link_error_capability_denied_display() {
        let err = LinkError::CapabilityDenied {
            component: "secret-reader".into(),
            capability: "filesystem-read".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0202"));
        assert!(msg.contains("filesystem-read"));
        assert!(msg.contains("Torvyn.toml"));
    }

    #[test]
    fn test_link_error_cyclic_dependency_display() {
        let err = LinkError::CyclicDependency {
            cycle: vec!["A".into(), "B".into(), "A".into()],
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0203"));
        assert!(msg.contains("A \u{2192} B \u{2192} A"));
    }

    // --- ResourceError ---

    #[test]
    fn test_resource_error_stale_handle_display() {
        let err = ResourceError::StaleHandle {
            handle: ResourceId::new(5, 0),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0300"));
        assert!(msg.contains("resource-5:g0"));
    }

    #[test]
    fn test_resource_error_budget_exceeded_display() {
        let err = ResourceError::BudgetExceeded {
            component: ComponentId::new(1),
            current_bytes: 1000,
            requested_bytes: 500,
            budget_bytes: 1024,
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0306"));
        assert!(msg.contains("Torvyn.toml"));
    }

    // --- ReactorError ---

    #[test]
    fn test_reactor_error_flow_not_found_display() {
        let err = ReactorError::FlowNotFound {
            flow_id: FlowId::new(99),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0400"));
        assert!(msg.contains("flow-99"));
    }

    // --- SecurityError ---

    #[test]
    fn test_security_error_capability_denied_display() {
        let err = SecurityError::CapabilityDenied {
            component: ComponentId::new(3),
            capability: "network".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0500"));
        assert!(msg.contains("network"));
    }

    // --- ConfigError ---

    #[test]
    fn test_config_error_file_not_found_display() {
        let err = ConfigError::FileNotFound {
            path: "/app/Torvyn.toml".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0700"));
        assert!(msg.contains("--config"));
    }

    // --- PackagingError ---

    #[test]
    fn test_packaging_error_invalid_artifact_display() {
        let err = PackagingError::InvalidArtifact {
            path: "my-component.tar".into(),
            reason: "missing manifest layer".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0600"));
        assert!(msg.contains("torvyn pack"));
    }

    // --- EngineError ---

    #[test]
    fn test_engine_error_compilation_failed_display() {
        let err = EngineError::CompilationFailed {
            module: "my-module.wasm".into(),
            reason: "invalid magic bytes".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0800"));
        assert!(msg.contains("my-module.wasm"));
    }

    // --- TorvynError conversions ---

    #[test]
    fn test_torvyn_error_from_process_error() {
        let err: TorvynError = ProcessError::Fatal("bad".into()).into();
        assert!(matches!(err, TorvynError::Process(ProcessError::Fatal(_))));
    }

    #[test]
    fn test_torvyn_error_from_contract_error() {
        let err: TorvynError = ContractError::PackageNotFound {
            package_name: "x".into(),
        }
        .into();
        assert!(matches!(err, TorvynError::Contract(_)));
    }

    #[test]
    fn test_torvyn_error_from_resource_error() {
        let err: TorvynError = ResourceError::StaleHandle {
            handle: ResourceId::new(0, 0),
        }
        .into();
        assert!(matches!(err, TorvynError::Resource(_)));
    }

    #[test]
    fn test_torvyn_error_display_preserves_inner() {
        let inner = ProcessError::DeadlineExceeded;
        let inner_msg = format!("{inner}");
        let outer: TorvynError = inner.into();
        let outer_msg = format!("{outer}");
        assert_eq!(inner_msg, outer_msg);
    }

    #[test]
    fn test_torvyn_error_source_chain() {
        use std::error::Error;
        let err: TorvynError = ProcessError::Internal("oops".into()).into();
        assert!(err.source().is_some());
    }
}
