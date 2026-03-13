//! Engine error types for the Torvyn runtime.
//!
//! Error code range: E0800–E0899.
//!
//! These errors cover Wasm compilation, instantiation, invocation,
//! fuel exhaustion, memory limits, and type mismatches.

use thiserror::Error;
use torvyn_types::ComponentId;

/// Errors originating from the Wasm engine layer.
///
/// This is the primary error type returned by [`WasmEngine`](crate::WasmEngine)
/// and [`ComponentInvoker`](crate::ComponentInvoker) operations.
///
/// All variants include actionable error messages that tell the user
/// what went wrong, where, and how to fix it.
///
/// # Error Code Range
/// E0800–E0899
///
/// # Examples
/// ```
/// use torvyn_engine::EngineError;
///
/// let err = EngineError::CompilationFailed {
///     reason: "invalid magic number".into(),
///     source_hint: Some("my-component.wasm".into()),
/// };
/// assert!(format!("{}", err).contains("E0800"));
/// ```
#[derive(Debug, Error)]
pub enum EngineError {
    /// The Wasm component binary could not be compiled.
    ///
    /// Causes: invalid binary format, unsupported Wasm features,
    /// compiler internal error.
    #[error(
        "[E0800] Compilation failed{}: {reason}. \
         Verify the .wasm file is a valid WebAssembly Component \
         (not a core module). Re-compile with a supported toolchain.",
        match source_hint {
            Some(s) => format!(" for '{s}'"),
            None => String::new(),
        }
    )]
    CompilationFailed {
        /// The reason compilation failed.
        reason: String,
        /// Optional hint about the source file.
        source_hint: Option<String>,
    },

    /// A previously serialized (cached) compiled component could not
    /// be deserialized.
    ///
    /// Causes: cache corruption, engine version mismatch, config mismatch.
    #[error(
        "[E0801] Deserialization failed: {reason}. \
         The compilation cache may be stale. \
         Delete the cache directory and retry."
    )]
    DeserializationFailed {
        /// The reason deserialization failed.
        reason: String,
    },

    /// Component instantiation failed.
    ///
    /// Causes: unresolved imports, resource limit exceeded during
    /// instantiation, initialization trap.
    #[error(
        "[E0802] Instantiation failed for component {component_id}: {reason}. \
         Check that all imports are satisfied and resource limits are sufficient."
    )]
    InstantiationFailed {
        /// The component that failed to instantiate.
        component_id: ComponentId,
        /// The reason instantiation failed.
        reason: String,
    },

    /// A component import could not be resolved during linking.
    #[error(
        "[E0803] Unresolved import '{import_name}' for component {component_id}. \
         Ensure the pipeline topology provides all required interfaces."
    )]
    UnresolvedImport {
        /// The component with the unresolved import.
        component_id: ComponentId,
        /// The name of the unresolved import.
        import_name: String,
    },

    /// The component trapped during execution.
    ///
    /// A trap is an unrecoverable error within the Wasm execution
    /// (e.g., unreachable instruction, division by zero, out-of-bounds
    /// memory access).
    #[error(
        "[E0804] Component {component_id} trapped: {trap_code}. \
         This indicates a bug in the component. \
         Check component logs and consider filing a bug report."
    )]
    Trap {
        /// The component that trapped.
        component_id: ComponentId,
        /// Description of the trap.
        trap_code: String,
    },

    /// The component exhausted its fuel budget.
    ///
    /// The component consumed more CPU than its allocated fuel allows.
    /// This is a safety mechanism to prevent infinite loops and
    /// CPU-intensive components from starving others.
    #[error(
        "[E0805] Fuel exhausted for component {component_id}. \
         The component exceeded its CPU budget of {fuel_limit} fuel units. \
         Consider increasing the fuel budget in the pipeline configuration \
         or optimizing the component."
    )]
    FuelExhausted {
        /// The component that ran out of fuel.
        component_id: ComponentId,
        /// The fuel budget that was exceeded.
        fuel_limit: u64,
    },

    /// The component exceeded its memory limit.
    #[error(
        "[E0806] Memory limit exceeded for component {component_id}: \
         attempted to grow to {attempted_bytes} bytes, limit is {limit_bytes} bytes. \
         Increase the component's memory limit in the pipeline configuration."
    )]
    MemoryLimitExceeded {
        /// The component that exceeded the limit.
        component_id: ComponentId,
        /// How many bytes the component tried to use.
        attempted_bytes: usize,
        /// The configured limit.
        limit_bytes: usize,
    },

    /// A type mismatch occurred during invocation.
    ///
    /// The arguments or return values did not match the expected
    /// Component Model types.
    #[error(
        "[E0807] Type mismatch during invocation of '{function_name}' on \
         component {component_id}: {detail}. \
         Verify the component was compiled against the correct WIT contract."
    )]
    TypeMismatch {
        /// The component with the type mismatch.
        component_id: ComponentId,
        /// The function being invoked.
        function_name: String,
        /// Details about the mismatch.
        detail: String,
    },

    /// The requested export function was not found on the component.
    #[error(
        "[E0808] Export '{function_name}' not found on component {component_id}. \
         Verify the component exports the required Torvyn interface \
         (e.g., `torvyn:streaming/processor`)."
    )]
    ExportNotFound {
        /// The component missing the export.
        component_id: ComponentId,
        /// The function that was not found.
        function_name: String,
    },

    /// An internal engine error that should not occur under normal operation.
    #[error(
        "[E0809] Internal engine error: {reason}. \
         This may indicate a Torvyn bug. Please report this issue."
    )]
    Internal {
        /// Description of the internal error.
        reason: String,
    },

    /// WASI configuration failed.
    #[error(
        "[E0810] WASI configuration failed for component {component_id}: {reason}. \
         Check the component's capability grants and sandbox configuration."
    )]
    WasiConfigError {
        /// The component with the WASI config problem.
        component_id: ComponentId,
        /// The reason WASI config failed.
        reason: String,
    },

    /// Timeout waiting for a component invocation to complete.
    #[error(
        "[E0811] Invocation of '{function_name}' on component {component_id} \
         timed out after {timeout_ms}ms. \
         Consider increasing the invocation timeout."
    )]
    InvocationTimeout {
        /// The component that timed out.
        component_id: ComponentId,
        /// The function that timed out.
        function_name: String,
        /// How long we waited (ms).
        timeout_ms: u64,
    },
}

impl EngineError {
    /// Returns `true` if this error represents a fatal, unrecoverable
    /// condition for the component (trap, fuel exhaustion).
    ///
    /// # WARM PATH — called per error to determine component fate.
    #[inline]
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            EngineError::Trap { .. }
                | EngineError::FuelExhausted { .. }
                | EngineError::MemoryLimitExceeded { .. }
        )
    }

    /// Returns `true` if this error is transient and the operation
    /// might succeed on retry.
    ///
    /// # WARM PATH
    #[inline]
    pub fn is_retryable(&self) -> bool {
        matches!(self, EngineError::InvocationTimeout { .. })
    }

    /// Returns the error code as a static string for metrics labels.
    ///
    /// # WARM PATH
    #[inline]
    pub fn code(&self) -> &'static str {
        match self {
            EngineError::CompilationFailed { .. } => "E0800",
            EngineError::DeserializationFailed { .. } => "E0801",
            EngineError::InstantiationFailed { .. } => "E0802",
            EngineError::UnresolvedImport { .. } => "E0803",
            EngineError::Trap { .. } => "E0804",
            EngineError::FuelExhausted { .. } => "E0805",
            EngineError::MemoryLimitExceeded { .. } => "E0806",
            EngineError::TypeMismatch { .. } => "E0807",
            EngineError::ExportNotFound { .. } => "E0808",
            EngineError::Internal { .. } => "E0809",
            EngineError::WasiConfigError { .. } => "E0810",
            EngineError::InvocationTimeout { .. } => "E0811",
        }
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

// LLI DEVIATION: The LLI doc maps EngineError to LinkError::CompilationFailed,
// but torvyn-types now has a proper TorvynError::Engine(EngineError) variant.
// We map to that via the torvyn_types::EngineError intermediate type.
impl From<EngineError> for torvyn_types::TorvynError {
    fn from(e: EngineError) -> Self {
        let types_err = match &e {
            EngineError::CompilationFailed { source_hint, reason } => {
                torvyn_types::EngineError::CompilationFailed {
                    module: source_hint.clone().unwrap_or_default(),
                    reason: reason.clone(),
                }
            }
            EngineError::InstantiationFailed {
                component_id,
                reason,
            } => torvyn_types::EngineError::InstantiationFailed {
                module: component_id.to_string(),
                reason: reason.clone(),
            },
            other => torvyn_types::EngineError::CompilationFailed {
                module: String::new(),
                reason: other.to_string(),
            },
        };
        torvyn_types::TorvynError::Engine(types_err)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::ComponentId;

    #[test]
    fn test_compilation_failed_display_actionable() {
        let err = EngineError::CompilationFailed {
            reason: "invalid magic number".into(),
            source_hint: Some("my-component.wasm".into()),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0800"), "should contain error code");
        assert!(
            msg.contains("my-component.wasm"),
            "should contain source hint"
        );
        assert!(
            msg.contains("valid WebAssembly Component"),
            "should contain remediation"
        );
    }

    #[test]
    fn test_compilation_failed_no_source_hint() {
        let err = EngineError::CompilationFailed {
            reason: "bad binary".into(),
            source_hint: None,
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0800"));
        assert!(!msg.contains("for ''"), "should not have empty for clause");
    }

    #[test]
    fn test_trap_display_actionable() {
        let err = EngineError::Trap {
            component_id: ComponentId::new(42),
            trap_code: "unreachable instruction".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0804"));
        assert!(msg.contains("component-42"));
        assert!(msg.contains("bug in the component"));
    }

    #[test]
    fn test_fuel_exhausted_display_actionable() {
        let err = EngineError::FuelExhausted {
            component_id: ComponentId::new(7),
            fuel_limit: 1_000_000,
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0805"));
        assert!(msg.contains("1000000"));
        assert!(msg.contains("fuel budget"));
    }

    #[test]
    fn test_memory_limit_exceeded_display() {
        let err = EngineError::MemoryLimitExceeded {
            component_id: ComponentId::new(1),
            attempted_bytes: 32 * 1024 * 1024,
            limit_bytes: 16 * 1024 * 1024,
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0806"));
    }

    #[test]
    fn test_export_not_found_display() {
        let err = EngineError::ExportNotFound {
            component_id: ComponentId::new(3),
            function_name: "process".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0808"));
        assert!(msg.contains("process"));
    }

    #[test]
    fn test_is_fatal() {
        assert!(EngineError::Trap {
            component_id: ComponentId::new(1),
            trap_code: "x".into(),
        }
        .is_fatal());
        assert!(EngineError::FuelExhausted {
            component_id: ComponentId::new(1),
            fuel_limit: 0,
        }
        .is_fatal());
        assert!(EngineError::MemoryLimitExceeded {
            component_id: ComponentId::new(1),
            attempted_bytes: 0,
            limit_bytes: 0,
        }
        .is_fatal());
        assert!(!EngineError::CompilationFailed {
            reason: "x".into(),
            source_hint: None,
        }
        .is_fatal());
    }

    #[test]
    fn test_is_retryable() {
        assert!(EngineError::InvocationTimeout {
            component_id: ComponentId::new(1),
            function_name: "process".into(),
            timeout_ms: 5000,
        }
        .is_retryable());
        assert!(!EngineError::Trap {
            component_id: ComponentId::new(1),
            trap_code: "x".into(),
        }
        .is_retryable());
    }

    #[test]
    fn test_error_codes_unique() {
        let codes = vec![
            EngineError::CompilationFailed {
                reason: String::new(),
                source_hint: None,
            }
            .code(),
            EngineError::DeserializationFailed {
                reason: String::new(),
            }
            .code(),
            EngineError::InstantiationFailed {
                component_id: ComponentId::new(0),
                reason: String::new(),
            }
            .code(),
            EngineError::UnresolvedImport {
                component_id: ComponentId::new(0),
                import_name: String::new(),
            }
            .code(),
            EngineError::Trap {
                component_id: ComponentId::new(0),
                trap_code: String::new(),
            }
            .code(),
            EngineError::FuelExhausted {
                component_id: ComponentId::new(0),
                fuel_limit: 0,
            }
            .code(),
            EngineError::MemoryLimitExceeded {
                component_id: ComponentId::new(0),
                attempted_bytes: 0,
                limit_bytes: 0,
            }
            .code(),
            EngineError::TypeMismatch {
                component_id: ComponentId::new(0),
                function_name: String::new(),
                detail: String::new(),
            }
            .code(),
            EngineError::ExportNotFound {
                component_id: ComponentId::new(0),
                function_name: String::new(),
            }
            .code(),
            EngineError::Internal {
                reason: String::new(),
            }
            .code(),
            EngineError::WasiConfigError {
                component_id: ComponentId::new(0),
                reason: String::new(),
            }
            .code(),
            EngineError::InvocationTimeout {
                component_id: ComponentId::new(0),
                function_name: String::new(),
                timeout_ms: 0,
            }
            .code(),
        ];
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len(), "all error codes must be unique");
    }

    #[test]
    fn test_conversion_to_torvyn_error() {
        let err = EngineError::CompilationFailed {
            reason: "bad binary".into(),
            source_hint: Some("test.wasm".into()),
        };
        let torvyn_err: torvyn_types::TorvynError = err.into();
        let msg = format!("{torvyn_err}");
        assert!(msg.contains("bad binary"));
    }
}
