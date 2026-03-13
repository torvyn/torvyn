//! Diagnostic event types.
//!
//! Per HLI Doc 05 §5.1. Events are structured records emitted at
//! significant runtime state transitions.
//!
//! HLI DEVIATION: Doc 05 defines `Severity` with variants
//! `{Debug, Info, Warning, Error, Critical}`. The canonical `Severity`
//! in `torvyn-types` (lli_01) uses `{Trace, Debug, Info, Warn, Error}`.
//! We use the `torvyn-types` `Severity` as canonical. The HLI's
//! `Critical` maps to `Severity::Error` with an `is_critical` flag
//! in the event payload.

use serde::{Deserialize, Serialize};
use torvyn_types::{ComponentId, FlowId, Severity, StreamId, TraceContext};

/// A structured diagnostic event.
///
/// Per HLI Doc 05 §5.1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    /// Timestamp in nanoseconds since Unix epoch.
    pub timestamp_ns: u64,
    /// Event severity.
    pub severity: Severity,
    /// Event category.
    pub category: EventCategory,
    /// Associated flow (if any).
    pub flow_id: Option<FlowId>,
    /// Associated component (if any).
    pub component_id: Option<ComponentId>,
    /// Trace context at the time of the event.
    pub trace_context: Option<TraceContext>,
    /// Event-specific payload.
    pub payload: EventPayload,
    /// Whether this event is critical (maps to HLI's "Critical" severity).
    pub is_critical: bool,
}

/// Event category for filtering and routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventCategory {
    /// Flow and component lifecycle transitions.
    Lifecycle,
    /// Performance-related observations.
    Performance,
    /// Error conditions.
    Error,
    /// Security-related events.
    Security,
    /// Resource management events.
    Resource,
}

impl std::fmt::Display for EventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lifecycle => write!(f, "lifecycle"),
            Self::Performance => write!(f, "performance"),
            Self::Error => write!(f, "error"),
            Self::Security => write!(f, "security"),
            Self::Resource => write!(f, "resource"),
        }
    }
}

/// Event-specific payload.
///
/// Each variant carries the fields documented in HLI Doc 05 §5.2.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum EventPayload {
    // --- Lifecycle ---
    /// A new flow has been created.
    FlowCreated {
        /// Component IDs in the topology.
        topology: Vec<ComponentId>,
        /// Hash of the flow configuration.
        config_hash: u64,
    },
    /// A flow has started execution.
    FlowStarted {
        /// Number of components in the flow.
        component_count: u32,
    },
    /// A flow is draining.
    FlowDraining {
        /// Reason for draining.
        reason: String,
    },
    /// A flow has completed successfully.
    FlowCompleted {
        /// Total duration in nanoseconds.
        duration_ns: u64,
        /// Total elements processed.
        elements_total: u64,
    },
    /// A flow was cancelled.
    FlowCancelled {
        /// Reason for cancellation.
        reason: String,
        /// Duration before cancellation in nanoseconds.
        duration_ns: u64,
    },
    /// A flow has failed.
    FlowFailed {
        /// Error description.
        error: String,
        /// Component that caused the failure (if known).
        failing_component: Option<ComponentId>,
    },
    /// A component instance was created.
    ComponentInstantiated {
        /// WIT interface implemented.
        wit_interface: String,
        /// Memory limit in bytes.
        memory_limit: u64,
    },
    /// A component was linked.
    ComponentLinked {
        /// Number of imports satisfied.
        imports_satisfied: u32,
        /// Number of exports provided.
        exports_provided: u32,
    },
    /// A component was terminated.
    ComponentTerminated {
        /// Reason for termination.
        reason: String,
        /// Total invocations before termination.
        invocation_count: u64,
    },

    // --- Performance ---
    /// Backpressure was activated on a stream.
    BackpressureActivated {
        /// Stream where backpressure activated.
        stream_id: StreamId,
        /// Producing component.
        producer: ComponentId,
        /// Consuming component.
        consumer: ComponentId,
        /// Queue depth at activation.
        queue_depth: u32,
    },
    /// Backpressure was deactivated.
    BackpressureDeactivated {
        /// Stream where backpressure deactivated.
        stream_id: StreamId,
        /// Duration of backpressure in nanoseconds.
        duration_ns: u64,
    },
    /// A component invocation exceeded time threshold.
    ComponentSlow {
        /// Actual processing time in nanoseconds.
        processing_time_ns: u64,
        /// Configured threshold in nanoseconds.
        threshold_ns: u64,
    },
    /// Scheduler detected starvation of a component.
    SchedulerStarvation {
        /// Starved component.
        starved_component: ComponentId,
        /// Duration of starvation in nanoseconds.
        duration_ns: u64,
    },
    /// A flow element experienced a latency spike.
    FlowLatencySpike {
        /// Element sequence number.
        element_sequence: u64,
        /// Observed latency in nanoseconds.
        latency_ns: u64,
        /// Threshold in nanoseconds.
        threshold_ns: u64,
    },

    // --- Error ---
    /// A component returned an error.
    ComponentError {
        /// Error variant name.
        error_variant: String,
        /// Error message.
        error_message: String,
    },
    /// A component trapped (e.g., Wasm trap).
    ComponentTrap {
        /// Trap type.
        trap_type: String,
        /// Trap message.
        trap_message: String,
    },
    /// A component invocation timed out.
    ComponentTimeout {
        /// Configured timeout in nanoseconds.
        timeout_ns: u64,
        /// Actual processing time in nanoseconds.
        processing_time_ns: u64,
    },
    /// A component exhausted its fuel budget.
    ComponentFuelExhausted {
        /// Fuel limit that was exceeded.
        fuel_limit: u64,
    },
    /// A flow exceeded its error budget.
    FlowErrorBudgetExceeded {
        /// Current error count.
        error_count: u64,
        /// Configured error budget.
        error_budget: u64,
    },

    // --- Security ---
    /// A capability access was denied.
    CapabilityDenied {
        /// Capability that was denied.
        capability: String,
        /// Context of the denial.
        context: String,
    },
    /// A capability was exercised.
    CapabilityExercised {
        /// Capability that was used.
        capability: String,
    },
    /// A component signature was verified.
    SignatureVerified {
        /// Signer identity.
        signer: String,
        /// Signature algorithm.
        algorithm: String,
    },
    /// A component signature verification failed.
    SignatureFailed {
        /// Reason for failure.
        reason: String,
    },

    // --- Resource ---
    /// A resource was allocated from a pool.
    ResourceAllocated {
        /// Raw resource ID.
        resource_id_raw: u64,
        /// Pool tier.
        pool_id: u32,
        /// Allocated size in bytes.
        size_bytes: u64,
    },
    /// A resource was copied between components.
    ResourceCopied {
        /// Raw resource ID.
        resource_id_raw: u64,
        /// Source component.
        from: ComponentId,
        /// Destination component.
        to: ComponentId,
        /// Bytes copied.
        copy_bytes: u64,
        /// Reason for the copy.
        reason: String,
    },
    /// A resource was transferred between owners.
    ResourceTransferred {
        /// Raw resource ID.
        resource_id_raw: u64,
        /// Previous owner.
        from: String,
        /// New owner.
        to: String,
        /// Transfer type.
        transfer_type: String,
    },
    /// A resource was freed.
    ResourceFreed {
        /// Raw resource ID.
        resource_id_raw: u64,
        /// Pool it was returned to.
        pool_id: u32,
    },
    /// A pool was exhausted during allocation.
    PoolExhausted {
        /// Pool that was exhausted.
        pool_id: u32,
        /// Requested allocation size.
        requested_size: u64,
        /// Pool capacity.
        pool_capacity: u32,
    },
    /// A resource leak is suspected.
    ResourceLeakSuspected {
        /// Raw resource ID.
        resource_id_raw: u64,
        /// Current owner.
        owner: String,
        /// Age of the resource in nanoseconds.
        age_ns: u64,
    },
}

impl DiagnosticEvent {
    /// Create a new diagnostic event with the current timestamp.
    ///
    /// # WARM PATH for performance/resource events, COLD PATH for lifecycle.
    pub fn new(severity: Severity, category: EventCategory, payload: EventPayload) -> Self {
        Self {
            timestamp_ns: current_time_ns(),
            severity,
            category,
            flow_id: None,
            component_id: None,
            trace_context: None,
            payload,
            is_critical: false,
        }
    }

    /// Set the flow ID.
    pub fn with_flow(mut self, flow_id: FlowId) -> Self {
        self.flow_id = Some(flow_id);
        self
    }

    /// Set the component ID.
    pub fn with_component(mut self, component_id: ComponentId) -> Self {
        self.component_id = Some(component_id);
        self
    }

    /// Set the trace context.
    pub fn with_trace(mut self, ctx: TraceContext) -> Self {
        self.trace_context = Some(ctx);
        self
    }

    /// Mark as critical.
    pub fn critical(mut self) -> Self {
        self.is_critical = true;
        self
    }

    /// Event name for logging/export.
    pub fn event_name(&self) -> &'static str {
        match &self.payload {
            EventPayload::FlowCreated { .. } => "flow.created",
            EventPayload::FlowStarted { .. } => "flow.started",
            EventPayload::FlowDraining { .. } => "flow.draining",
            EventPayload::FlowCompleted { .. } => "flow.completed",
            EventPayload::FlowCancelled { .. } => "flow.cancelled",
            EventPayload::FlowFailed { .. } => "flow.failed",
            EventPayload::ComponentInstantiated { .. } => "component.instantiated",
            EventPayload::ComponentLinked { .. } => "component.linked",
            EventPayload::ComponentTerminated { .. } => "component.terminated",
            EventPayload::BackpressureActivated { .. } => "backpressure.activated",
            EventPayload::BackpressureDeactivated { .. } => "backpressure.deactivated",
            EventPayload::ComponentSlow { .. } => "component.slow",
            EventPayload::SchedulerStarvation { .. } => "scheduler.starvation",
            EventPayload::FlowLatencySpike { .. } => "flow.latency_spike",
            EventPayload::ComponentError { .. } => "component.error",
            EventPayload::ComponentTrap { .. } => "component.trap",
            EventPayload::ComponentTimeout { .. } => "component.timeout",
            EventPayload::ComponentFuelExhausted { .. } => "component.fuel_exhausted",
            EventPayload::FlowErrorBudgetExceeded { .. } => "flow.error_budget_exceeded",
            EventPayload::CapabilityDenied { .. } => "capability.denied",
            EventPayload::CapabilityExercised { .. } => "capability.exercised",
            EventPayload::SignatureVerified { .. } => "component.signature_verified",
            EventPayload::SignatureFailed { .. } => "component.signature_failed",
            EventPayload::ResourceAllocated { .. } => "resource.allocated",
            EventPayload::ResourceCopied { .. } => "resource.copied",
            EventPayload::ResourceTransferred { .. } => "resource.transferred",
            EventPayload::ResourceFreed { .. } => "resource.freed",
            EventPayload::PoolExhausted { .. } => "pool.exhausted",
            EventPayload::ResourceLeakSuspected { .. } => "resource.leak_suspected",
        }
    }
}

/// Get current time in nanoseconds since Unix epoch.
///
/// # HOT PATH — should be fast.
#[inline]
pub fn current_time_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_event_new() {
        let event = DiagnosticEvent::new(
            Severity::Info,
            EventCategory::Lifecycle,
            EventPayload::FlowStarted {
                component_count: 3,
            },
        );
        assert_eq!(event.severity, Severity::Info);
        assert_eq!(event.category, EventCategory::Lifecycle);
        assert!(event.timestamp_ns > 0);
        assert!(!event.is_critical);
    }

    #[test]
    fn test_diagnostic_event_builder() {
        let event = DiagnosticEvent::new(
            Severity::Error,
            EventCategory::Error,
            EventPayload::ComponentTrap {
                trap_type: "OOM".into(),
                trap_message: "out of memory".into(),
            },
        )
        .with_flow(FlowId::new(1))
        .with_component(ComponentId::new(2))
        .critical();

        assert_eq!(event.flow_id, Some(FlowId::new(1)));
        assert_eq!(event.component_id, Some(ComponentId::new(2)));
        assert!(event.is_critical);
    }

    #[test]
    fn test_event_name() {
        let event = DiagnosticEvent::new(
            Severity::Warn,
            EventCategory::Performance,
            EventPayload::BackpressureActivated {
                stream_id: StreamId::new(1),
                producer: ComponentId::new(1),
                consumer: ComponentId::new(2),
                queue_depth: 64,
            },
        );
        assert_eq!(event.event_name(), "backpressure.activated");
    }

    #[test]
    fn test_event_category_display() {
        assert_eq!(format!("{}", EventCategory::Lifecycle), "lifecycle");
        assert_eq!(format!("{}", EventCategory::Security), "security");
    }

    #[test]
    fn test_event_serde_roundtrip() {
        let event = DiagnosticEvent::new(
            Severity::Info,
            EventCategory::Resource,
            EventPayload::ResourceAllocated {
                resource_id_raw: 42,
                pool_id: 0,
                size_bytes: 4096,
            },
        );
        let json = serde_json::to_string(&event).unwrap();
        let parsed: DiagnosticEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_name(), "resource.allocated");
    }

    #[test]
    fn test_current_time_ns_positive() {
        let ts = current_time_ns();
        assert!(ts > 0);
    }
}
