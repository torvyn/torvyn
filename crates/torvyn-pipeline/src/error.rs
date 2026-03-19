//! Pipeline error types for topology construction, validation,
//! instantiation, and shutdown.
//!
//! Error code range: E0900–E0999.

use std::fmt;
use std::time::Duration;

use torvyn_types::{ComponentId, FlowId};

// ---------------------------------------------------------------------------
// PipelineError
// ---------------------------------------------------------------------------

/// Errors originating from the pipeline crate.
///
/// Covers topology construction, validation, instantiation, and shutdown.
///
/// # Examples
/// ```
/// use torvyn_pipeline::PipelineError;
///
/// let err = PipelineError::EmptyTopology {
///     flow_name: "my-flow".into(),
/// };
/// let msg = format!("{err}");
/// assert!(msg.contains("E0900"));
/// assert!(msg.contains("my-flow"));
/// ```
#[derive(Debug)]
pub enum PipelineError {
    // --- Topology construction errors (E0900–E0919) ---
    /// The topology has no nodes.
    EmptyTopology {
        /// The flow name.
        flow_name: String,
    },

    /// A node name is duplicated within the topology.
    DuplicateNode {
        /// The flow name.
        flow_name: String,
        /// The duplicate node name.
        node_name: String,
    },

    /// An edge references a node that does not exist.
    EdgeReferencesUnknownNode {
        /// The flow name.
        flow_name: String,
        /// The edge index.
        edge_index: usize,
        /// The unknown node name.
        node_name: String,
        /// Which endpoint: "from" or "to".
        endpoint: &'static str,
    },

    /// A self-loop edge (from and to are the same node).
    SelfLoop {
        /// The flow name.
        flow_name: String,
        /// The node with the self-loop.
        node_name: String,
    },

    // --- Topology validation errors (E0920–E0939) ---
    /// The topology contains a cycle.
    CycleDetected {
        /// The flow name.
        flow_name: String,
        /// Node names forming the cycle.
        cycle: Vec<String>,
    },

    /// A node is not reachable from any source.
    DisconnectedNode {
        /// The flow name.
        flow_name: String,
        /// The disconnected node name.
        node_name: String,
    },

    /// A source node has incoming edges.
    SourceHasIncoming {
        /// The flow name.
        flow_name: String,
        /// The source node name.
        node_name: String,
    },

    /// A sink node has outgoing edges.
    SinkHasOutgoing {
        /// The flow name.
        flow_name: String,
        /// The sink node name.
        node_name: String,
    },

    /// A processor/filter/router node is missing incoming or outgoing edges.
    ProcessorMissingEdges {
        /// The flow name.
        flow_name: String,
        /// The node name.
        node_name: String,
        /// The role of the node (e.g. "Processor").
        role: String,
        /// Whether the node has incoming edges.
        has_incoming: bool,
        /// Whether the node has outgoing edges.
        has_outgoing: bool,
    },

    /// Fan-out or fan-in limit exceeded.
    FanLimitExceeded {
        /// The flow name.
        flow_name: String,
        /// The node name.
        node_name: String,
        /// The port name.
        port: String,
        /// The actual connection count.
        count: usize,
        /// The configured limit.
        limit: usize,
        /// "fan-out" or "fan-in".
        direction: &'static str,
    },

    /// No source nodes found in the topology.
    NoSourceNodes {
        /// The flow name.
        flow_name: String,
    },

    /// No sink nodes found in the topology.
    NoSinkNodes {
        /// The flow name.
        flow_name: String,
    },

    // --- Instantiation errors (E0940–E0969) ---
    /// Component compilation failed.
    CompilationFailed {
        /// The flow name.
        flow_name: String,
        /// The node name.
        node_name: String,
        /// The failure reason.
        reason: String,
    },

    /// Component instantiation failed.
    InstantiationFailed {
        /// The flow name.
        flow_name: String,
        /// The node name.
        node_name: String,
        /// The failure reason.
        reason: String,
    },

    /// Component initialization (`lifecycle.init`) failed.
    InitializationFailed {
        /// The flow name.
        flow_name: String,
        /// The node name.
        node_name: String,
        /// The failure reason.
        reason: String,
    },

    /// Flow registration with the reactor failed.
    FlowRegistrationFailed {
        /// The flow name.
        flow_name: String,
        /// The failure reason.
        reason: String,
    },

    /// Security sandbox configuration failed.
    SandboxConfigFailed {
        /// The flow name.
        flow_name: String,
        /// The node name.
        node_name: String,
        /// The failure reason.
        reason: String,
    },

    // --- Shutdown errors (E0970–E0989) ---
    /// Graceful shutdown timed out.
    ShutdownTimeout {
        /// The flow ID.
        flow_id: FlowId,
        /// The configured timeout.
        timeout: Duration,
        /// Number of components that did not complete draining.
        components_remaining: usize,
    },

    /// Component teardown failed.
    TeardownFailed {
        /// The flow ID.
        flow_id: FlowId,
        /// The component ID.
        component_id: ComponentId,
        /// The failure reason.
        reason: String,
    },

    /// Resource cleanup failed.
    ResourceCleanupFailed {
        /// The flow ID.
        flow_id: FlowId,
        /// The failure reason.
        reason: String,
    },

    // --- General (E0990–E0999) ---
    /// An error from a dependent subsystem.
    Subsystem {
        /// The subsystem name.
        subsystem: &'static str,
        /// The failure reason.
        reason: String,
    },
}

impl fmt::Display for PipelineError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // --- Topology construction ---
            PipelineError::EmptyTopology { flow_name } => {
                write!(
                    f,
                    "[E0900] Pipeline '{flow_name}' has no nodes. \
                     Add at least a source and a sink node to the flow definition."
                )
            }
            PipelineError::DuplicateNode {
                flow_name,
                node_name,
            } => {
                write!(
                    f,
                    "[E0901] Duplicate node name '{node_name}' in pipeline '{flow_name}'. \
                     Each node name must be unique within a flow."
                )
            }
            PipelineError::EdgeReferencesUnknownNode {
                flow_name,
                edge_index,
                node_name,
                endpoint,
            } => {
                write!(
                    f,
                    "[E0902] Edge {edge_index} in pipeline '{flow_name}' references \
                     unknown {endpoint} node '{node_name}'. \
                     Check that all edge endpoints reference nodes defined in the flow."
                )
            }
            PipelineError::SelfLoop {
                flow_name,
                node_name,
            } => {
                write!(
                    f,
                    "[E0903] Self-loop detected on node '{node_name}' in pipeline \
                     '{flow_name}'. A node cannot connect to itself."
                )
            }

            // --- Topology validation ---
            PipelineError::CycleDetected { flow_name, cycle } => {
                let cycle_str = cycle.join(" → ");
                write!(
                    f,
                    "[E0920] Cycle detected in pipeline '{flow_name}': {cycle_str}. \
                     Torvyn v1 requires pipeline topologies to be directed acyclic graphs (DAGs). \
                     Remove the cycle or implement the feedback loop inside a single component."
                )
            }
            PipelineError::DisconnectedNode {
                flow_name,
                node_name,
            } => {
                write!(
                    f,
                    "[E0921] Node '{node_name}' in pipeline '{flow_name}' is not reachable \
                     from any source. Connect it to the pipeline graph or remove it."
                )
            }
            PipelineError::SourceHasIncoming {
                flow_name,
                node_name,
            } => {
                write!(
                    f,
                    "[E0922] Source node '{node_name}' in pipeline '{flow_name}' has \
                     incoming edges. Source nodes must not have incoming edges. \
                     Change the node's role or remove the incoming edges."
                )
            }
            PipelineError::SinkHasOutgoing {
                flow_name,
                node_name,
            } => {
                write!(
                    f,
                    "[E0923] Sink node '{node_name}' in pipeline '{flow_name}' has \
                     outgoing edges. Sink nodes must not have outgoing edges. \
                     Change the node's role or remove the outgoing edges."
                )
            }
            PipelineError::ProcessorMissingEdges {
                flow_name,
                node_name,
                role,
                has_incoming,
                has_outgoing,
            } => {
                let missing = match (*has_incoming, *has_outgoing) {
                    (false, false) => "incoming and outgoing edges",
                    (false, true) => "incoming edges",
                    (true, false) => "outgoing edges",
                    (true, true) => unreachable!(),
                };
                write!(
                    f,
                    "[E0924] {role} node '{node_name}' in pipeline '{flow_name}' is \
                     missing {missing}. {role} nodes must have both incoming and outgoing edges."
                )
            }
            PipelineError::FanLimitExceeded {
                flow_name,
                node_name,
                port,
                count,
                limit,
                direction,
            } => {
                write!(
                    f,
                    "[E0925] {direction} limit exceeded for node '{node_name}' port \
                     '{port}' in pipeline '{flow_name}': {count} connections, limit is {limit}. \
                     Reduce the number of connections or increase the fan limit in configuration."
                )
            }
            PipelineError::NoSourceNodes { flow_name } => {
                write!(
                    f,
                    "[E0926] Pipeline '{flow_name}' has no source nodes. \
                     Every pipeline must have at least one source node."
                )
            }
            PipelineError::NoSinkNodes { flow_name } => {
                write!(
                    f,
                    "[E0927] Pipeline '{flow_name}' has no sink nodes. \
                     Every pipeline must have at least one sink node."
                )
            }

            // --- Instantiation ---
            PipelineError::CompilationFailed {
                flow_name,
                node_name,
                reason,
            } => {
                write!(
                    f,
                    "[E0940] Component compilation failed for node '{node_name}' in \
                     pipeline '{flow_name}': {reason}. \
                     Verify the component binary is a valid WebAssembly Component."
                )
            }
            PipelineError::InstantiationFailed {
                flow_name,
                node_name,
                reason,
            } => {
                write!(
                    f,
                    "[E0941] Component instantiation failed for node '{node_name}' in \
                     pipeline '{flow_name}': {reason}."
                )
            }
            PipelineError::InitializationFailed {
                flow_name,
                node_name,
                reason,
            } => {
                write!(
                    f,
                    "[E0942] Component initialization (lifecycle.init) failed for node \
                     '{node_name}' in pipeline '{flow_name}': {reason}. \
                     Check the component's init function and its configuration."
                )
            }
            PipelineError::FlowRegistrationFailed { flow_name, reason } => {
                write!(
                    f,
                    "[E0943] Failed to register flow for pipeline '{flow_name}' with \
                     the reactor: {reason}."
                )
            }
            PipelineError::SandboxConfigFailed {
                flow_name,
                node_name,
                reason,
            } => {
                write!(
                    f,
                    "[E0944] Security sandbox configuration failed for node '{node_name}' \
                     in pipeline '{flow_name}': {reason}. \
                     Check the component's capability grants in the pipeline configuration."
                )
            }

            // --- Shutdown ---
            PipelineError::ShutdownTimeout {
                flow_id,
                timeout,
                components_remaining,
            } => {
                write!(
                    f,
                    "[E0970] Graceful shutdown of {flow_id} timed out after {timeout:?}. \
                     {components_remaining} component(s) did not complete draining. \
                     They will be forcefully terminated."
                )
            }
            PipelineError::TeardownFailed {
                flow_id,
                component_id,
                reason,
            } => {
                write!(
                    f,
                    "[E0971] Teardown of {component_id} in {flow_id} failed: {reason}. \
                     Resources will be forcefully reclaimed."
                )
            }
            PipelineError::ResourceCleanupFailed { flow_id, reason } => {
                write!(
                    f,
                    "[E0972] Resource cleanup for {flow_id} failed: {reason}. \
                     Some resources may have leaked. Check the resource manager diagnostics."
                )
            }

            // --- General ---
            PipelineError::Subsystem { subsystem, reason } => {
                write!(
                    f,
                    "[E0990] Pipeline error from subsystem '{subsystem}': {reason}."
                )
            }
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<PipelineError> for torvyn_types::TorvynError {
    fn from(e: PipelineError) -> Self {
        // LLI DEVIATION: ReactorError::FlowCreationFailed does not exist.
        // Map pipeline errors into ReactorError::InvalidTopology instead,
        // since pipelines are the unit of scheduling.
        torvyn_types::ReactorError::InvalidTopology {
            reason: e.to_string(),
        }
        .into()
    }
}

// ---------------------------------------------------------------------------
// ValidationReport
// ---------------------------------------------------------------------------

/// Collects multiple validation errors for a single topology.
///
/// Validation is not fail-fast — all errors are collected so the developer
/// sees the complete list of problems in one check.
///
/// # Examples
/// ```
/// use torvyn_pipeline::{PipelineError, ValidationReport};
///
/// let mut report = ValidationReport::new("my-flow");
/// report.push(PipelineError::NoSourceNodes {
///     flow_name: "my-flow".into(),
/// });
/// assert!(!report.is_ok());
/// assert_eq!(report.errors().len(), 1);
/// ```
#[derive(Debug)]
pub struct ValidationReport {
    flow_name: String,
    errors: Vec<PipelineError>,
}

impl ValidationReport {
    /// Create a new empty report for the given flow.
    ///
    /// # COLD PATH — called once per validation.
    pub fn new(flow_name: &str) -> Self {
        Self {
            flow_name: flow_name.to_owned(),
            errors: Vec::new(),
        }
    }

    /// Add an error to the report.
    pub fn push(&mut self, error: PipelineError) {
        self.errors.push(error);
    }

    /// Returns `true` if no errors were found.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns a slice of all collected errors.
    pub fn errors(&self) -> &[PipelineError] {
        &self.errors
    }

    /// Consume the report, returning `Ok(())` or `Err(Vec<PipelineError>)`.
    ///
    /// # Errors
    /// Returns `Err` containing all collected errors if any were found.
    pub fn into_result(self) -> Result<(), Vec<PipelineError>> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }

    /// The flow name this report is for.
    pub fn flow_name(&self) -> &str {
        &self.flow_name
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_topology_error_display() {
        let err = PipelineError::EmptyTopology {
            flow_name: "test-flow".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0900"));
        assert!(msg.contains("test-flow"));
        assert!(msg.contains("no nodes"));
    }

    #[test]
    fn test_cycle_detected_error_display() {
        let err = PipelineError::CycleDetected {
            flow_name: "loop-flow".into(),
            cycle: vec!["A".into(), "B".into(), "C".into(), "A".into()],
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0920"));
        assert!(msg.contains("A → B → C → A"));
        assert!(msg.contains("DAG"));
    }

    #[test]
    fn test_fan_limit_error_display() {
        let err = PipelineError::FanLimitExceeded {
            flow_name: "wide-flow".into(),
            node_name: "router-1".into(),
            port: "output".into(),
            count: 20,
            limit: 16,
            direction: "fan-out",
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0925"));
        assert!(msg.contains("20"));
        assert!(msg.contains("16"));
    }

    #[test]
    fn test_shutdown_timeout_error_display() {
        let err = PipelineError::ShutdownTimeout {
            flow_id: torvyn_types::FlowId::new(42),
            timeout: Duration::from_secs(30),
            components_remaining: 3,
        };
        let msg = format!("{err}");
        assert!(msg.contains("E0970"));
        assert!(msg.contains("flow-42"));
        assert!(msg.contains("3 component(s)"));
    }

    #[test]
    fn test_validation_report_collects_errors() {
        let mut report = ValidationReport::new("test");
        assert!(report.is_ok());

        report.push(PipelineError::NoSourceNodes {
            flow_name: "test".into(),
        });
        report.push(PipelineError::NoSinkNodes {
            flow_name: "test".into(),
        });

        assert!(!report.is_ok());
        assert_eq!(report.errors().len(), 2);
    }

    #[test]
    fn test_validation_report_into_result_ok() {
        let report = ValidationReport::new("ok");
        assert!(report.into_result().is_ok());
    }

    #[test]
    fn test_validation_report_into_result_err() {
        let mut report = ValidationReport::new("bad");
        report.push(PipelineError::EmptyTopology {
            flow_name: "bad".into(),
        });
        let result = report.into_result();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().len(), 1);
    }

    #[test]
    fn test_pipeline_error_into_torvyn_error() {
        let err = PipelineError::EmptyTopology {
            flow_name: "x".into(),
        };
        let te: torvyn_types::TorvynError = err.into();
        let msg = format!("{te}");
        assert!(msg.contains("E0900"));
    }
}
