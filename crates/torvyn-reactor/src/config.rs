//! Configuration types for the Torvyn reactor.
//!
//! All configuration is immutable after flow creation. Defaults are drawn
//! from [`torvyn_types`] constants.

use std::time::Duration;

use torvyn_types::{
    BackpressurePolicy, TraceContext, DEFAULT_COMPONENT_TIMEOUT_MS,
    DEFAULT_COOPERATIVE_CANCEL_TIMEOUT_MS, DEFAULT_DRAIN_TIMEOUT_MS, DEFAULT_ELEMENTS_PER_YIELD,
    DEFAULT_LOW_WATERMARK_RATIO, DEFAULT_QUEUE_DEPTH, MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD,
};

use crate::error::ErrorPolicy;
use crate::fairness::FlowPriority;
use crate::topology::FlowTopology;

// ---------------------------------------------------------------------------
// FlowConfig
// ---------------------------------------------------------------------------

/// Configuration for creating a new flow.
///
/// Passed to [`ReactorHandle::create_flow`](crate::ReactorHandle::create_flow).
/// All fields have sensible defaults.
#[derive(Clone, Debug)]
pub struct FlowConfig {
    /// The pipeline topology (stages and connections).
    pub topology: FlowTopology,
    /// Scheduling priority for this flow.
    pub priority: FlowPriority,
    /// Timeout configuration.
    pub timeouts: TimeoutConfig,
    /// Default backpressure policy for all streams in this flow.
    pub default_backpressure_policy: BackpressurePolicy,
    /// Default stream queue capacity.
    pub default_queue_capacity: usize,
    /// Default low watermark ratio for backpressure recovery.
    pub default_low_watermark_ratio: f64,
    /// Yield configuration for cooperative scheduling.
    pub yield_config: YieldConfig,
    /// Error handling policy.
    pub error_policy: ErrorPolicy,
    /// Trace context to inherit for distributed tracing.
    pub trace_context: Option<TraceContext>,
}

impl FlowConfig {
    /// Create a `FlowConfig` with defaults for the given topology.
    ///
    /// # COLD PATH — called during flow creation.
    pub fn default_with_topology(topology: FlowTopology) -> Self {
        Self {
            topology,
            priority: FlowPriority::Normal,
            timeouts: TimeoutConfig::default(),
            default_backpressure_policy: BackpressurePolicy::default(),
            default_queue_capacity: DEFAULT_QUEUE_DEPTH,
            default_low_watermark_ratio: DEFAULT_LOW_WATERMARK_RATIO,
            yield_config: YieldConfig::default(),
            error_policy: ErrorPolicy::default(),
            trace_context: None,
        }
    }
}

// ---------------------------------------------------------------------------
// StreamConfig
// ---------------------------------------------------------------------------

/// Per-stream configuration overrides.
///
/// If a field is `None`, the flow-level default is used.
#[derive(Clone, Debug, Default)]
pub struct StreamConfig {
    /// Queue capacity override. Default: flow's `default_queue_capacity`.
    pub capacity: Option<usize>,
    /// Backpressure policy override. Default: flow's `default_backpressure_policy`.
    pub backpressure_policy: Option<BackpressurePolicy>,
    /// Low watermark ratio override. Default: flow's `default_low_watermark_ratio`.
    pub low_watermark_ratio: Option<f64>,
}

// ---------------------------------------------------------------------------
// TimeoutConfig
// ---------------------------------------------------------------------------

/// Timeout configuration for a flow.
///
/// Per Doc 04, Section 6.5. All timeouts are wall-clock time.
#[derive(Clone, Debug)]
pub struct TimeoutConfig {
    /// Maximum wall-clock duration for the entire flow.
    /// `None` means no flow-level deadline.
    pub flow_deadline: Option<Duration>,
    /// Maximum wall-clock duration for a single component invocation.
    pub component_invocation_timeout: Duration,
    /// Maximum time for the draining phase after cancellation.
    pub drain_timeout: Duration,
    /// Maximum time to wait for cooperative cancellation before
    /// escalating to fuel exhaustion.
    pub cooperative_cancel_timeout: Duration,
}

impl Default for TimeoutConfig {
    /// # COLD PATH — called during flow configuration.
    fn default() -> Self {
        Self {
            flow_deadline: None,
            component_invocation_timeout: Duration::from_millis(DEFAULT_COMPONENT_TIMEOUT_MS),
            drain_timeout: Duration::from_millis(DEFAULT_DRAIN_TIMEOUT_MS),
            cooperative_cancel_timeout: Duration::from_millis(
                DEFAULT_COOPERATIVE_CANCEL_TIMEOUT_MS,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// YieldConfig
// ---------------------------------------------------------------------------

/// Yield configuration for cooperative scheduling within a flow.
///
/// Per Doc 04, Section 3.5 and 7.2.
///
/// # Invariants
/// - `elements_per_yield` must be > 0 and <= `MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD`.
/// - `time_quantum` must be > 0.
#[derive(Clone, Debug)]
pub struct YieldConfig {
    /// Number of elements to process before yielding to Tokio.
    pub elements_per_yield: u64,
    /// Maximum wall-clock time per yield quantum.
    pub time_quantum: Duration,
}

impl Default for YieldConfig {
    /// # COLD PATH — called during flow configuration.
    fn default() -> Self {
        Self {
            elements_per_yield: DEFAULT_ELEMENTS_PER_YIELD,
            time_quantum: Duration::from_micros(100),
        }
    }
}

impl YieldConfig {
    /// Validate the yield configuration.
    ///
    /// # Errors
    /// Returns an error string if any invariant is violated.
    ///
    /// # COLD PATH — called during flow creation.
    pub fn validate(&self) -> Result<(), String> {
        if self.elements_per_yield == 0 {
            return Err("elements_per_yield must be > 0".into());
        }
        if self.elements_per_yield > MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD {
            return Err(format!(
                "elements_per_yield ({}) exceeds hard ceiling ({})",
                self.elements_per_yield, MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD
            ));
        }
        if self.time_quantum.is_zero() {
            return Err("time_quantum must be > 0".into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_config_defaults() {
        let cfg = TimeoutConfig::default();
        assert_eq!(cfg.component_invocation_timeout, Duration::from_secs(5));
        assert_eq!(cfg.drain_timeout, Duration::from_secs(5));
        assert_eq!(cfg.cooperative_cancel_timeout, Duration::from_secs(1));
        assert!(cfg.flow_deadline.is_none());
    }

    #[test]
    fn test_yield_config_defaults() {
        let cfg = YieldConfig::default();
        assert_eq!(cfg.elements_per_yield, 32);
        assert_eq!(cfg.time_quantum, Duration::from_micros(100));
    }

    #[test]
    fn test_yield_config_validate_ok() {
        let cfg = YieldConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_yield_config_validate_zero_elements() {
        let cfg = YieldConfig {
            elements_per_yield: 0,
            ..YieldConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_yield_config_validate_exceeds_ceiling() {
        let cfg = YieldConfig {
            elements_per_yield: MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD + 1,
            ..YieldConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("exceeds hard ceiling"));
    }

    #[test]
    fn test_yield_config_validate_zero_quantum() {
        let cfg = YieldConfig {
            time_quantum: Duration::ZERO,
            ..YieldConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_stream_config_default_is_all_none() {
        let cfg = StreamConfig::default();
        assert!(cfg.capacity.is_none());
        assert!(cfg.backpressure_policy.is_none());
        assert!(cfg.low_watermark_ratio.is_none());
    }
}
