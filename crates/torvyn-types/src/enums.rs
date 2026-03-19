//! Domain enumerations for the Torvyn runtime.
//!
//! Lightweight, `Copy` types for component roles, backpressure signals/policies,
//! observability levels, severity levels, and copy reasons.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ComponentRole
// ---------------------------------------------------------------------------

/// The role of a component within a pipeline topology.
///
/// Per consolidated review (Doc 10, C02-7, C04-4): `ComponentRole` is the
/// canonical name, replacing `NodeRole` (Doc 02) and `StageRole` (Doc 04).
///
/// # Examples
/// ```
/// use torvyn_types::ComponentRole;
///
/// let role = ComponentRole::Processor;
/// assert_eq!(format!("{}", role), "Processor");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ComponentRole {
    /// Produces stream elements by pulling from an external data source.
    Source,
    /// Transforms stream elements (1:1 input-to-output mapping).
    Processor,
    /// Consumes stream elements by writing to an external destination.
    Sink,
    /// Evaluates whether elements pass a predicate (accept/reject, no new buffer).
    Filter,
    /// Routes elements to one or more named output ports.
    Router,
}

impl ComponentRole {
    /// Returns `true` if this role produces stream elements.
    ///
    /// # COLD PATH — called during topology validation.
    #[inline]
    pub const fn is_producer(&self) -> bool {
        matches!(
            self,
            ComponentRole::Source | ComponentRole::Processor | ComponentRole::Router
        )
    }

    /// Returns `true` if this role consumes stream elements.
    ///
    /// # COLD PATH — called during topology validation.
    #[inline]
    pub const fn is_consumer(&self) -> bool {
        matches!(
            self,
            ComponentRole::Processor
                | ComponentRole::Sink
                | ComponentRole::Filter
                | ComponentRole::Router
        )
    }
}

impl fmt::Display for ComponentRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComponentRole::Source => write!(f, "Source"),
            ComponentRole::Processor => write!(f, "Processor"),
            ComponentRole::Sink => write!(f, "Sink"),
            ComponentRole::Filter => write!(f, "Filter"),
            ComponentRole::Router => write!(f, "Router"),
        }
    }
}

// ---------------------------------------------------------------------------
// BackpressureSignal
// ---------------------------------------------------------------------------

/// Backpressure signal from a consumer to the runtime.
///
/// Maps directly to the WIT `backpressure-signal` enum defined in
/// `torvyn:streaming@0.1.0` (Doc 01, Section 3.1).
///
/// # Examples
/// ```
/// use torvyn_types::BackpressureSignal;
///
/// let signal = BackpressureSignal::Pause;
/// assert!(signal.is_paused());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum BackpressureSignal {
    /// Consumer is ready to accept more data. Normal operation.
    Ready,
    /// Consumer requests the producer to pause until further notice.
    Pause,
}

impl BackpressureSignal {
    /// Returns `true` if the consumer is requesting a pause.
    ///
    /// # HOT PATH — called per element after sink invocation.
    #[inline]
    pub const fn is_paused(&self) -> bool {
        matches!(self, BackpressureSignal::Pause)
    }
}

impl fmt::Display for BackpressureSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackpressureSignal::Ready => write!(f, "Ready"),
            BackpressureSignal::Pause => write!(f, "Pause"),
        }
    }
}

// ---------------------------------------------------------------------------
// BackpressurePolicy
// ---------------------------------------------------------------------------

/// Policy governing what happens when a stream queue is full.
///
/// Configured per-stream or per-flow. The reactor enforces the chosen policy.
///
/// # Examples
/// ```
/// use torvyn_types::BackpressurePolicy;
///
/// let policy = BackpressurePolicy::BlockProducer;
/// assert_eq!(format!("{}", policy), "BlockProducer");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum BackpressurePolicy {
    /// Block the producer until space is available (default).
    /// This is the safest option and prevents data loss.
    #[default]
    BlockProducer,
    /// Drop the oldest element in the queue to make room for the new one.
    /// Useful for real-time systems where freshness matters more than completeness.
    DropOldest,
    /// Drop the newest element (the one being produced) when the queue is full.
    /// The producer continues without blocking; the new element is discarded.
    DropNewest,
    /// Return an error to the producer when the queue is full.
    /// The producer must handle the error (retry, skip, or fail).
    Error,
}

impl BackpressurePolicy {
    /// Returns `true` if this policy can cause data loss.
    ///
    /// # COLD PATH — called during configuration validation.
    #[inline]
    pub const fn may_lose_data(&self) -> bool {
        matches!(
            self,
            BackpressurePolicy::DropOldest | BackpressurePolicy::DropNewest
        )
    }
}

// LLI DEVIATION: Default derived via #[derive(Default)] + #[default] instead of manual impl per clippy.

impl fmt::Display for BackpressurePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackpressurePolicy::BlockProducer => write!(f, "BlockProducer"),
            BackpressurePolicy::DropOldest => write!(f, "DropOldest"),
            BackpressurePolicy::DropNewest => write!(f, "DropNewest"),
            BackpressurePolicy::Error => write!(f, "Error"),
        }
    }
}

// ---------------------------------------------------------------------------
// ObservabilityLevel
// ---------------------------------------------------------------------------

/// Observability collection level, configurable at runtime.
///
/// Per Doc 05, Section 1.4: three levels with explicit overhead budgets.
/// Transitions are atomic (via `AtomicU8` in the collector).
///
/// # Examples
/// ```
/// use torvyn_types::ObservabilityLevel;
///
/// let level = ObservabilityLevel::Production;
/// assert!(level.is_enabled());
/// assert_eq!(level as u8, 1);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(u8)]
pub enum ObservabilityLevel {
    /// Nothing collected. For bare-metal benchmarks. Overhead: 0%.
    Off = 0,
    /// Flow-level counters, latency histograms, error counts. Default.
    /// Overhead: < 1% throughput, < 500ns per element.
    #[default]
    Production = 1,
    /// All of Production + per-element spans, per-copy accounting,
    /// per-backpressure events, queue depth snapshots.
    /// Overhead: < 5% throughput, < 2us per element.
    Diagnostic = 2,
}

impl ObservabilityLevel {
    /// Returns `true` if any collection is enabled.
    ///
    /// # HOT PATH — checked per element to skip recording.
    #[inline]
    pub const fn is_enabled(&self) -> bool {
        !matches!(self, ObservabilityLevel::Off)
    }

    /// Returns `true` if diagnostic-level detail is enabled.
    ///
    /// # HOT PATH — checked per element for detailed recording.
    #[inline]
    pub const fn is_diagnostic(&self) -> bool {
        matches!(self, ObservabilityLevel::Diagnostic)
    }

    /// Convert from a raw `u8` value, returning `None` for invalid values.
    #[inline]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(ObservabilityLevel::Off),
            1 => Some(ObservabilityLevel::Production),
            2 => Some(ObservabilityLevel::Diagnostic),
            _ => None,
        }
    }
}

// LLI DEVIATION: Default derived via #[derive(Default)] + #[default] instead of manual impl per clippy.

impl fmt::Display for ObservabilityLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObservabilityLevel::Off => write!(f, "Off"),
            ObservabilityLevel::Production => write!(f, "Production"),
            ObservabilityLevel::Diagnostic => write!(f, "Diagnostic"),
        }
    }
}

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

/// Log severity level for diagnostic events.
///
/// # Examples
/// ```
/// use torvyn_types::Severity;
///
/// let s = Severity::Warn;
/// assert!(s >= Severity::Info);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(u8)]
pub enum Severity {
    /// Fine-grained debugging information.
    Trace = 0,
    /// Developer-oriented debugging information.
    Debug = 1,
    /// Informational messages about normal operation.
    Info = 2,
    /// Potentially harmful situations that deserve attention.
    Warn = 3,
    /// Error events that may still allow the system to continue.
    Error = 4,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Trace => write!(f, "TRACE"),
            Severity::Debug => write!(f, "DEBUG"),
            Severity::Info => write!(f, "INFO"),
            Severity::Warn => write!(f, "WARN"),
            Severity::Error => write!(f, "ERROR"),
        }
    }
}

// ---------------------------------------------------------------------------
// CopyReason
// ---------------------------------------------------------------------------

/// The reason a data copy occurred, for copy accounting and observability.
///
/// Per Doc 05, Section 9.1 (`EventSink::record_copy`). This is the
/// `torvyn-types` version; Doc 05 defines additional observability-specific
/// variants. This crate provides the shared subset.
///
/// # Examples
/// ```
/// use torvyn_types::CopyReason;
///
/// let reason = CopyReason::HostToComponent;
/// assert_eq!(format!("{}", reason), "HostToComponent");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CopyReason {
    /// Data must enter component linear memory for processing.
    HostToComponent,
    /// Data is extracted from component linear memory after processing.
    ComponentToHost,
    /// Data is transferred between components (involves a host intermediary).
    CrossComponent,
    /// Buffer contents are copied when returning to the pool.
    PoolReturn,
}

impl fmt::Display for CopyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CopyReason::HostToComponent => write!(f, "HostToComponent"),
            CopyReason::ComponentToHost => write!(f, "ComponentToHost"),
            CopyReason::CrossComponent => write!(f, "CrossComponent"),
            CopyReason::PoolReturn => write!(f, "PoolReturn"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- ComponentRole ---

    #[test]
    fn test_component_role_display() {
        assert_eq!(format!("{}", ComponentRole::Source), "Source");
        assert_eq!(format!("{}", ComponentRole::Processor), "Processor");
        assert_eq!(format!("{}", ComponentRole::Sink), "Sink");
        assert_eq!(format!("{}", ComponentRole::Filter), "Filter");
        assert_eq!(format!("{}", ComponentRole::Router), "Router");
    }

    #[test]
    fn test_component_role_producer_consumer() {
        assert!(ComponentRole::Source.is_producer());
        assert!(!ComponentRole::Source.is_consumer());
        assert!(ComponentRole::Processor.is_producer());
        assert!(ComponentRole::Processor.is_consumer());
        assert!(!ComponentRole::Sink.is_producer());
        assert!(ComponentRole::Sink.is_consumer());
        assert!(!ComponentRole::Filter.is_producer());
        assert!(ComponentRole::Filter.is_consumer());
        assert!(ComponentRole::Router.is_producer());
        assert!(ComponentRole::Router.is_consumer());
    }

    // --- BackpressureSignal ---

    #[test]
    fn test_backpressure_signal_is_paused() {
        assert!(!BackpressureSignal::Ready.is_paused());
        assert!(BackpressureSignal::Pause.is_paused());
    }

    // --- BackpressurePolicy ---

    #[test]
    fn test_backpressure_policy_default() {
        assert_eq!(
            BackpressurePolicy::default(),
            BackpressurePolicy::BlockProducer
        );
    }

    #[test]
    fn test_backpressure_policy_may_lose_data() {
        assert!(!BackpressurePolicy::BlockProducer.may_lose_data());
        assert!(BackpressurePolicy::DropOldest.may_lose_data());
        assert!(BackpressurePolicy::DropNewest.may_lose_data());
        assert!(!BackpressurePolicy::Error.may_lose_data());
    }

    // --- ObservabilityLevel ---

    #[test]
    fn test_observability_level_ordering() {
        assert!(ObservabilityLevel::Off < ObservabilityLevel::Production);
        assert!(ObservabilityLevel::Production < ObservabilityLevel::Diagnostic);
    }

    #[test]
    fn test_observability_level_is_enabled() {
        assert!(!ObservabilityLevel::Off.is_enabled());
        assert!(ObservabilityLevel::Production.is_enabled());
        assert!(ObservabilityLevel::Diagnostic.is_enabled());
    }

    #[test]
    fn test_observability_level_is_diagnostic() {
        assert!(!ObservabilityLevel::Off.is_diagnostic());
        assert!(!ObservabilityLevel::Production.is_diagnostic());
        assert!(ObservabilityLevel::Diagnostic.is_diagnostic());
    }

    #[test]
    fn test_observability_level_from_u8() {
        assert_eq!(
            ObservabilityLevel::from_u8(0),
            Some(ObservabilityLevel::Off)
        );
        assert_eq!(
            ObservabilityLevel::from_u8(1),
            Some(ObservabilityLevel::Production)
        );
        assert_eq!(
            ObservabilityLevel::from_u8(2),
            Some(ObservabilityLevel::Diagnostic)
        );
        assert_eq!(ObservabilityLevel::from_u8(3), None);
    }

    #[test]
    fn test_observability_level_default() {
        assert_eq!(
            ObservabilityLevel::default(),
            ObservabilityLevel::Production
        );
    }

    #[test]
    fn test_observability_level_repr_u8() {
        assert_eq!(ObservabilityLevel::Off as u8, 0);
        assert_eq!(ObservabilityLevel::Production as u8, 1);
        assert_eq!(ObservabilityLevel::Diagnostic as u8, 2);
    }

    // --- Severity ---

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Trace < Severity::Debug);
        assert!(Severity::Debug < Severity::Info);
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Error);
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(format!("{}", Severity::Trace), "TRACE");
        assert_eq!(format!("{}", Severity::Error), "ERROR");
    }

    // --- CopyReason ---

    #[test]
    fn test_copy_reason_display() {
        assert_eq!(
            format!("{}", CopyReason::HostToComponent),
            "HostToComponent"
        );
        assert_eq!(
            format!("{}", CopyReason::ComponentToHost),
            "ComponentToHost"
        );
        assert_eq!(format!("{}", CopyReason::CrossComponent), "CrossComponent");
        assert_eq!(format!("{}", CopyReason::PoolReturn), "PoolReturn");
    }
}
