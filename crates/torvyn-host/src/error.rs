//! Host-level error types.
//!
//! The host is the top-level error boundary. Every subsystem error
//! surfaces here with additional context about which flow and pipeline
//! stage produced it.
//!
//! Error code range: E0900–E0999.

use std::fmt;
use std::time::Duration;

use torvyn_types::FlowId;

// ---------------------------------------------------------------------------
// HostError — top-level error for the host runtime
// ---------------------------------------------------------------------------

/// Top-level error for the Torvyn host runtime.
///
/// Every public host API returns `Result<_, HostError>`. Subsystem errors
/// are wrapped with host-level context.
///
/// # Examples
/// ```
/// use torvyn_host::HostError;
///
/// let err = HostError::config("Torvyn.toml not found".into());
/// assert!(format!("{err}").contains("E0900"));
/// ```
#[derive(Debug)]
pub enum HostError {
    /// Configuration parsing or validation failed.
    Config(ConfigContext),

    /// The startup sequence failed at a specific stage.
    Startup(StartupError),

    /// A flow operation failed.
    Flow(FlowError),

    /// Shutdown did not complete within the allowed time.
    ShutdownTimeout {
        /// The configured timeout duration.
        timeout: Duration,
        /// Number of flows still active when timeout expired.
        flows_remaining: usize,
    },

    /// An internal error that should not happen. Indicates a bug.
    Internal(String),
}

impl HostError {
    /// Create a configuration error with context.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn config(detail: String) -> Self {
        HostError::Config(ConfigContext { detail })
    }

    /// Create a flow-not-found error.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn flow_not_found(flow_id: FlowId) -> Self {
        HostError::Flow(FlowError::NotFound { flow_id })
    }

    /// Returns the error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            HostError::Config(_) => "E0900",
            HostError::Startup(s) => s.code(),
            HostError::Flow(f) => f.code(),
            HostError::ShutdownTimeout { .. } => "E0910",
            HostError::Internal(_) => "E0999",
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Config(ctx) => {
                write!(
                    f,
                    "[E0900] Configuration error: {}. \
                     Check your Torvyn.toml and run `torvyn check` to validate.",
                    ctx.detail
                )
            }
            HostError::Startup(err) => write!(f, "{err}"),
            HostError::Flow(err) => write!(f, "{err}"),
            HostError::ShutdownTimeout {
                timeout,
                flows_remaining,
            } => {
                write!(
                    f,
                    "[E0910] Graceful shutdown timed out after {timeout:?} with \
                     {flows_remaining} flow(s) still active. \
                     Force-terminated remaining flows. \
                     Consider increasing `shutdown_timeout` in Torvyn.toml."
                )
            }
            HostError::Internal(msg) => {
                write!(
                    f,
                    "[E0999] Internal host error: {msg}. \
                     This is likely a bug — please report it."
                )
            }
        }
    }
}

impl std::error::Error for HostError {}

// ---------------------------------------------------------------------------
// ConfigContext
// ---------------------------------------------------------------------------

/// Context for a configuration error.
#[derive(Debug)]
pub struct ConfigContext {
    /// Human-readable detail about what went wrong.
    pub detail: String,
}

// ---------------------------------------------------------------------------
// StartupError — errors during the startup sequence
// ---------------------------------------------------------------------------

/// Errors that occur during the host startup sequence.
///
/// Each variant maps to a specific startup stage, making it clear
/// exactly where the startup failed.
#[derive(Debug)]
pub enum StartupError {
    /// Wasm engine initialization failed.
    EngineInit {
        /// The underlying reason for the failure.
        reason: String,
    },

    /// Observability system initialization failed.
    ObservabilityInit {
        /// The underlying reason for the failure.
        reason: String,
    },

    /// Resource manager initialization failed.
    ResourceInit {
        /// The underlying reason for the failure.
        reason: String,
    },

    /// Security manager initialization failed.
    SecurityInit {
        /// The underlying reason for the failure.
        reason: String,
    },

    /// A flow failed during the pipeline startup sequence.
    FlowStartup {
        /// Name of the flow that failed.
        flow_name: String,
        /// Which stage of the startup sequence failed.
        stage: StartupStage,
        /// The underlying reason for the failure.
        reason: String,
    },
}

impl StartupError {
    /// Returns the error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            StartupError::EngineInit { .. } => "E0901",
            StartupError::ObservabilityInit { .. } => "E0902",
            StartupError::ResourceInit { .. } => "E0903",
            StartupError::SecurityInit { .. } => "E0904",
            StartupError::FlowStartup { .. } => "E0905",
        }
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartupError::EngineInit { reason } => {
                write!(
                    f,
                    "[E0901] Wasm engine initialization failed: {reason}. \
                     Check Wasmtime version compatibility and system resources."
                )
            }
            StartupError::ObservabilityInit { reason } => {
                write!(
                    f,
                    "[E0902] Observability initialization failed: {reason}. \
                     Verify the tracing exporter endpoint is reachable."
                )
            }
            StartupError::ResourceInit { reason } => {
                write!(
                    f,
                    "[E0903] Resource manager initialization failed: {reason}. \
                     Check memory availability and pool configuration."
                )
            }
            StartupError::SecurityInit { reason } => {
                write!(
                    f,
                    "[E0904] Security manager initialization failed: {reason}. \
                     Verify capability policies in Torvyn.toml."
                )
            }
            StartupError::FlowStartup {
                flow_name,
                stage,
                reason,
            } => {
                write!(
                    f,
                    "[E0905] Flow '{flow_name}' failed during {stage}: {reason}."
                )
            }
        }
    }
}

impl std::error::Error for StartupError {}

// ---------------------------------------------------------------------------
// StartupStage
// ---------------------------------------------------------------------------

/// Which stage of the startup sequence failed.
///
/// Provides precise context in startup error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupStage {
    /// Parsing and converting the flow definition.
    TopologyConstruction,
    /// Validating the pipeline topology (DAG check, port compatibility).
    TopologyValidation,
    /// Validating component contracts against the topology.
    ContractValidation,
    /// Linking components (resolving imports/exports, capabilities).
    Linking,
    /// Compiling Wasm components to native code.
    Compilation,
    /// Instantiating Wasm component instances.
    Instantiation,
    /// Calling `lifecycle.init()` on components.
    ComponentInit,
    /// Registering the flow with the reactor.
    ReactorRegistration,
}

impl fmt::Display for StartupStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartupStage::TopologyConstruction => write!(f, "topology construction"),
            StartupStage::TopologyValidation => write!(f, "topology validation"),
            StartupStage::ContractValidation => write!(f, "contract validation"),
            StartupStage::Linking => write!(f, "component linking"),
            StartupStage::Compilation => write!(f, "Wasm compilation"),
            StartupStage::Instantiation => write!(f, "component instantiation"),
            StartupStage::ComponentInit => {
                write!(f, "component initialization (lifecycle.init)")
            }
            StartupStage::ReactorRegistration => write!(f, "reactor registration"),
        }
    }
}

// ---------------------------------------------------------------------------
// FlowError
// ---------------------------------------------------------------------------

/// Errors for flow-level operations (create, cancel, inspect).
#[derive(Debug)]
pub enum FlowError {
    /// The specified flow was not found.
    NotFound {
        /// The flow ID that was not found.
        flow_id: FlowId,
    },

    /// The flow is in a state that does not permit the requested operation.
    InvalidState {
        /// The flow ID.
        flow_id: FlowId,
        /// The current state of the flow.
        current_state: String,
        /// The operation that was attempted.
        operation: String,
    },

    /// A reactor-level error occurred.
    Reactor {
        /// Description of the reactor error.
        detail: String,
    },
}

impl FlowError {
    /// Returns the error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            FlowError::NotFound { .. } => "E0920",
            FlowError::InvalidState { .. } => "E0921",
            FlowError::Reactor { .. } => "E0922",
        }
    }
}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowError::NotFound { flow_id } => {
                write!(
                    f,
                    "[E0920] Flow {flow_id} not found. \
                     Use `torvyn inspect flows` to list active flows."
                )
            }
            FlowError::InvalidState {
                flow_id,
                current_state,
                operation,
            } => {
                write!(
                    f,
                    "[E0921] Cannot {operation} flow {flow_id}: flow is in \
                     '{current_state}' state."
                )
            }
            FlowError::Reactor { detail } => {
                write!(f, "[E0922] Reactor error: {detail}.")
            }
        }
    }
}

impl std::error::Error for FlowError {}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl From<StartupError> for HostError {
    fn from(e: StartupError) -> Self {
        HostError::Startup(e)
    }
}

impl From<FlowError> for HostError {
    fn from(e: FlowError) -> Self {
        HostError::Flow(e)
    }
}

impl From<torvyn_types::ConfigError> for HostError {
    fn from(e: torvyn_types::ConfigError) -> Self {
        HostError::Config(ConfigContext {
            detail: e.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_error_config_display() {
        let err = HostError::config("missing [runtime] section".into());
        let msg = format!("{err}");
        assert!(msg.contains("E0900"));
        assert!(msg.contains("missing [runtime]"));
        assert!(msg.contains("torvyn check"));
    }

    #[test]
    fn test_startup_error_engine_display() {
        let err = StartupError::EngineInit {
            reason: "Cranelift unavailable".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0901"));
        assert!(msg.contains("Cranelift"));
    }

    #[test]
    fn test_startup_error_observability_display() {
        let err = StartupError::ObservabilityInit {
            reason: "OTLP endpoint unreachable".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0902"));
        assert!(msg.contains("OTLP"));
    }

    #[test]
    fn test_startup_error_resource_display() {
        let err = StartupError::ResourceInit {
            reason: "out of memory".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0903"));
        assert!(msg.contains("out of memory"));
    }

    #[test]
    fn test_startup_error_security_display() {
        let err = StartupError::SecurityInit {
            reason: "invalid policy".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0904"));
        assert!(msg.contains("invalid policy"));
    }

    #[test]
    fn test_startup_error_flow_display() {
        let err = StartupError::FlowStartup {
            flow_name: "my-pipeline".into(),
            stage: StartupStage::Linking,
            reason: "unresolved import".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0905"));
        assert!(msg.contains("my-pipeline"));
        assert!(msg.contains("component linking"));
    }

    #[test]
    fn test_shutdown_timeout_display() {
        let err = HostError::ShutdownTimeout {
            timeout: Duration::from_secs(30),
            flows_remaining: 2,
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0910"));
        assert!(msg.contains("30s"));
        assert!(msg.contains("2 flow(s)"));
        assert!(msg.contains("shutdown_timeout"));
    }

    #[test]
    fn test_internal_error_display() {
        let err = HostError::Internal("unexpected state".into());
        let msg = format!("{err}");
        assert!(msg.contains("E0999"));
        assert!(msg.contains("unexpected state"));
        assert!(msg.contains("bug"));
    }

    #[test]
    fn test_flow_error_not_found_display() {
        let err = FlowError::NotFound {
            flow_id: FlowId::new(99),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0920"));
        assert!(msg.contains("flow-99"));
        assert!(msg.contains("torvyn inspect"));
    }

    #[test]
    fn test_flow_error_invalid_state_display() {
        let err = FlowError::InvalidState {
            flow_id: FlowId::new(1),
            current_state: "Draining".into(),
            operation: "pause".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0921"));
        assert!(msg.contains("Draining"));
        assert!(msg.contains("pause"));
    }

    #[test]
    fn test_flow_error_reactor_display() {
        let err = FlowError::Reactor {
            detail: "queue full".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0922"));
        assert!(msg.contains("queue full"));
    }

    #[test]
    fn test_startup_stage_display() {
        assert_eq!(
            format!("{}", StartupStage::ContractValidation),
            "contract validation"
        );
        assert_eq!(
            format!("{}", StartupStage::ReactorRegistration),
            "reactor registration"
        );
    }

    #[test]
    fn test_host_error_codes_unique() {
        let codes = vec![
            HostError::config(String::new()).code(),
            HostError::Startup(StartupError::EngineInit {
                reason: String::new(),
            })
            .code(),
            HostError::Startup(StartupError::ObservabilityInit {
                reason: String::new(),
            })
            .code(),
            HostError::Startup(StartupError::ResourceInit {
                reason: String::new(),
            })
            .code(),
            HostError::Startup(StartupError::SecurityInit {
                reason: String::new(),
            })
            .code(),
            HostError::Startup(StartupError::FlowStartup {
                flow_name: String::new(),
                stage: StartupStage::Linking,
                reason: String::new(),
            })
            .code(),
            HostError::ShutdownTimeout {
                timeout: Duration::from_secs(0),
                flows_remaining: 0,
            }
            .code(),
            HostError::flow_not_found(FlowId::new(0)).code(),
            HostError::Flow(FlowError::InvalidState {
                flow_id: FlowId::new(0),
                current_state: String::new(),
                operation: String::new(),
            })
            .code(),
            HostError::Flow(FlowError::Reactor {
                detail: String::new(),
            })
            .code(),
            HostError::Internal(String::new()).code(),
        ];
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len(), "all error codes must be unique");
    }

    #[test]
    fn test_conversion_from_startup_error() {
        let err: HostError = StartupError::EngineInit { reason: "x".into() }.into();
        assert!(matches!(err, HostError::Startup(_)));
    }

    #[test]
    fn test_conversion_from_flow_error() {
        let err: HostError = FlowError::NotFound {
            flow_id: FlowId::new(1),
        }
        .into();
        assert!(matches!(err, HostError::Flow(_)));
    }

    #[test]
    fn test_conversion_from_config_error() {
        let err: HostError = torvyn_types::ConfigError::FileNotFound {
            path: "test.toml".into(),
        }
        .into();
        assert!(matches!(err, HostError::Config(_)));
    }
}
