//! Reactor-specific error types.
//!
//! These extend the shared [`torvyn_types::ReactorError`] with internal
//! detail needed by the flow driver and coordinator.

use std::fmt;
use std::time::Duration;
use torvyn_types::{ComponentId, ProcessError};

// ---------------------------------------------------------------------------
// FlowCreationError
// ---------------------------------------------------------------------------

/// Errors that can occur when creating a new flow.
///
/// Returned by [`ReactorHandle::create_flow`](crate::ReactorHandle::create_flow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowCreationError {
    /// The pipeline topology is invalid (cycles, disconnected components, etc.).
    InvalidTopology(String),
    /// A referenced component does not exist or cannot be instantiated.
    ComponentNotFound(ComponentId),
    /// Contract incompatibility between connected components.
    ContractMismatch {
        /// Source component.
        from: ComponentId,
        /// Destination component.
        to: ComponentId,
        /// Description of the mismatch.
        detail: String,
    },
    /// Capability requirements not satisfied.
    CapabilityDenied {
        /// The component that was denied.
        component: ComponentId,
        /// The denied capability.
        capability: String,
    },
    /// The reactor is shutting down and cannot accept new flows.
    ReactorShuttingDown,
    /// An internal error occurred during flow creation.
    Internal(String),
}

impl fmt::Display for FlowCreationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowCreationError::InvalidTopology(reason) => {
                write!(
                    f,
                    "[E0410] Invalid flow topology: {reason}. \
                     Ensure the pipeline is a directed acyclic graph with valid connections."
                )
            }
            FlowCreationError::ComponentNotFound(id) => {
                write!(
                    f,
                    "[E0411] Component '{id}' not found. \
                     Verify the component is compiled and available to the host runtime."
                )
            }
            FlowCreationError::ContractMismatch { from, to, detail } => {
                write!(
                    f,
                    "[E0412] Contract mismatch between '{from}' and '{to}': {detail}. \
                     Run `torvyn link` to diagnose interface incompatibilities."
                )
            }
            FlowCreationError::CapabilityDenied {
                component,
                capability,
            } => {
                write!(
                    f,
                    "[E0413] Capability '{capability}' denied for '{component}'. \
                     Grant the capability in the pipeline configuration."
                )
            }
            FlowCreationError::ReactorShuttingDown => {
                write!(
                    f,
                    "[E0414] Reactor is shutting down; cannot create new flows. \
                     Wait for shutdown to complete before restarting."
                )
            }
            FlowCreationError::Internal(msg) => {
                write!(
                    f,
                    "[E0419] Internal reactor error during flow creation: {msg}"
                )
            }
        }
    }
}

impl std::error::Error for FlowCreationError {}

// ---------------------------------------------------------------------------
// FlowError
// ---------------------------------------------------------------------------

/// Errors that can occur during flow execution.
#[derive(Clone, Debug)]
pub enum FlowError {
    /// A component returned a processing error.
    ComponentError {
        /// The component that errored.
        component: ComponentId,
        /// The processing error.
        error: ProcessError,
    },
    /// The flow exceeded its wall-clock deadline.
    DeadlineExceeded {
        /// The configured deadline.
        deadline: Duration,
        /// The actual elapsed time.
        elapsed: Duration,
    },
    /// A single component invocation timed out.
    ComponentTimeout {
        /// The component that timed out.
        component: ComponentId,
        /// The configured timeout.
        timeout: Duration,
    },
    /// The flow's resource budget was exhausted.
    ResourceExhausted {
        /// Details about what was exhausted.
        detail: String,
    },
    /// The drain phase timed out after cancellation.
    DrainTimeout {
        /// The configured drain timeout.
        timeout: Duration,
    },
    /// An internal reactor error (bug).
    Internal(String),
}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowError::ComponentError { component, error } => {
                write!(f, "[E0420] Component '{component}' error: {error}")
            }
            FlowError::DeadlineExceeded { deadline, elapsed } => {
                write!(
                    f,
                    "[E0421] Flow deadline exceeded: allowed {deadline:?}, elapsed {elapsed:?}. \
                     Increase the flow deadline or optimize component processing."
                )
            }
            FlowError::ComponentTimeout { component, timeout } => {
                write!(
                    f,
                    "[E0422] Component '{component}' invocation timed out after {timeout:?}. \
                     Increase the per-component timeout or investigate the component's processing time."
                )
            }
            FlowError::ResourceExhausted { detail } => {
                write!(
                    f,
                    "[E0423] Resource exhaustion: {detail}. \
                     Release unused buffers or increase memory budgets."
                )
            }
            FlowError::DrainTimeout { timeout } => {
                write!(
                    f,
                    "[E0424] Drain phase timed out after {timeout:?}. \
                     Remaining elements were discarded. Increase drain timeout \
                     or investigate slow consumer components."
                )
            }
            FlowError::Internal(msg) => {
                write!(f, "[E0429] Internal reactor error: {msg}. This is a bug.")
            }
        }
    }
}

impl std::error::Error for FlowError {}

// ---------------------------------------------------------------------------
// ErrorPolicy
// ---------------------------------------------------------------------------

/// Policy for handling component errors during flow execution.
///
/// Per Doc 04, Section 14.2. Default is `FailFast`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorPolicy {
    /// Any component error immediately cancels the flow.
    FailFast,
    /// Retry the failed invocation up to N times with backoff.
    Retry {
        /// Maximum retry attempts.
        max_retries: u32,
        /// Delay between retries.
        backoff: Duration,
    },
    /// Skip the failed element and continue processing.
    SkipElement,
    /// Log the error and continue (ignoring the failure).
    LogAndContinue,
}

impl Default for ErrorPolicy {
    /// # COLD PATH — called during flow configuration.
    fn default() -> Self {
        ErrorPolicy::FailFast
    }
}

impl fmt::Display for ErrorPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorPolicy::FailFast => write!(f, "FailFast"),
            ErrorPolicy::Retry {
                max_retries,
                backoff,
            } => write!(f, "Retry(max={max_retries}, backoff={backoff:?})"),
            ErrorPolicy::SkipElement => write!(f, "SkipElement"),
            ErrorPolicy::LogAndContinue => write!(f, "LogAndContinue"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_creation_error_invalid_topology_display() {
        let err = FlowCreationError::InvalidTopology("cycle at stage 2".into());
        let msg = format!("{err}");
        assert!(msg.contains("E0410"));
        assert!(msg.contains("cycle at stage 2"));
    }

    #[test]
    fn test_flow_creation_error_component_not_found_display() {
        let err = FlowCreationError::ComponentNotFound(ComponentId::new(42));
        let msg = format!("{err}");
        assert!(msg.contains("E0411"));
        assert!(msg.contains("component-42"));
    }

    #[test]
    fn test_flow_creation_error_shutting_down_display() {
        let err = FlowCreationError::ReactorShuttingDown;
        let msg = format!("{err}");
        assert!(msg.contains("E0414"));
        assert!(msg.contains("shutting down"));
    }

    #[test]
    fn test_flow_error_deadline_display() {
        let err = FlowError::DeadlineExceeded {
            deadline: Duration::from_secs(30),
            elapsed: Duration::from_secs(31),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0421"));
        assert!(msg.contains("deadline"));
    }

    #[test]
    fn test_flow_error_component_timeout_display() {
        let err = FlowError::ComponentTimeout {
            component: ComponentId::new(5),
            timeout: Duration::from_secs(5),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0422"));
        assert!(msg.contains("component-5"));
    }

    #[test]
    fn test_flow_error_internal_display() {
        let err = FlowError::Internal("assertion failed".into());
        let msg = format!("{err}");
        assert!(msg.contains("E0429"));
        assert!(msg.contains("bug"));
    }

    #[test]
    fn test_error_policy_default_is_fail_fast() {
        assert!(matches!(ErrorPolicy::default(), ErrorPolicy::FailFast));
    }

    #[test]
    fn test_error_policy_display() {
        assert_eq!(format!("{}", ErrorPolicy::FailFast), "FailFast");
        assert_eq!(format!("{}", ErrorPolicy::SkipElement), "SkipElement");
        assert_eq!(format!("{}", ErrorPolicy::LogAndContinue), "LogAndContinue");
        let retry = ErrorPolicy::Retry {
            max_retries: 3,
            backoff: Duration::from_millis(100),
        };
        let msg = format!("{retry}");
        assert!(msg.contains("Retry"));
        assert!(msg.contains("max=3"));
    }
}
