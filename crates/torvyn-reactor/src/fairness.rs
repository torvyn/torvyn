//! Flow priority and yield control for inter-flow fairness.
//!
//! Per Doc 04 §3.4 and §7: priority levels adjust yield frequency.
//! Higher priority flows yield less often, getting more CPU time.

use std::time::Instant;
use torvyn_types::MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD;

// ---------------------------------------------------------------------------
// FlowPriority
// ---------------------------------------------------------------------------

/// Priority level for a flow.
///
/// Affects yield frequency (elements processed per yield to Tokio).
///
/// | Priority    | Elements per yield |
/// |-------------|-------------------|
/// | Critical    | 128               |
/// | High        | 64                |
/// | Normal      | 32                |
/// | Low         | 16                |
/// | Background  | 8                 |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlowPriority {
    /// Lowest priority. Yields most frequently.
    Background,
    /// Below-normal priority.
    Low,
    /// Default priority.
    #[default]
    Normal,
    /// Above-normal priority.
    High,
    /// Highest priority. Yields least frequently.
    Critical,
}

impl FlowPriority {
    /// Returns the default elements-per-yield for this priority level.
    ///
    /// # HOT PATH — called to determine yield quantum.
    #[inline]
    pub const fn elements_per_yield(&self) -> u64 {
        match self {
            FlowPriority::Critical => 128,
            FlowPriority::High => 64,
            FlowPriority::Normal => 32,
            FlowPriority::Low => 16,
            FlowPriority::Background => 8,
        }
    }
}

// LLI DEVIATION: Default derived via #[derive(Default)] + #[default] instead of manual impl per clippy.

impl std::fmt::Display for FlowPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowPriority::Background => write!(f, "Background"),
            FlowPriority::Low => write!(f, "Low"),
            FlowPriority::Normal => write!(f, "Normal"),
            FlowPriority::High => write!(f, "High"),
            FlowPriority::Critical => write!(f, "Critical"),
        }
    }
}

// ---------------------------------------------------------------------------
// YieldController
// ---------------------------------------------------------------------------

/// Tracks whether the flow driver should yield to Tokio.
///
/// Yields after `elements_per_yield` elements OR after `time_quantum`
/// elapses, whichever comes first. Hard ceiling at
/// `MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD`.
///
/// # HOT PATH — checked after every element.
pub struct YieldController {
    /// Elements processed since last yield.
    elements_since_yield: u64,
    /// Timestamp of last yield (or flow start).
    last_yield_at: Instant,
    /// Elements before yield (from priority config).
    elements_per_yield: u64,
    /// Maximum time before forced yield.
    time_quantum: std::time::Duration,
}

impl YieldController {
    /// Create a new `YieldController`.
    ///
    /// # COLD PATH — called once per flow.
    pub fn new(elements_per_yield: u64, time_quantum: std::time::Duration) -> Self {
        let clamped = elements_per_yield.min(MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD);
        Self {
            elements_since_yield: 0,
            last_yield_at: Instant::now(),
            elements_per_yield: clamped,
            time_quantum,
        }
    }

    /// Record that one element was processed.
    ///
    /// # HOT PATH — zero-alloc.
    #[inline(always)]
    pub fn record_element(&mut self) {
        self.elements_since_yield += 1;
    }

    /// Returns `true` if the flow driver should yield now.
    ///
    /// # HOT PATH — checked after every element.
    #[inline]
    pub fn should_yield(&self) -> bool {
        self.elements_since_yield >= self.elements_per_yield
            || self.last_yield_at.elapsed() >= self.time_quantum
    }

    /// Reset counters after a yield.
    ///
    /// # HOT PATH — called after each yield.
    #[inline]
    pub fn reset(&mut self) {
        self.elements_since_yield = 0;
        self.last_yield_at = Instant::now();
    }

    /// Returns elements since last yield (for observability).
    #[inline]
    pub fn elements_since_yield(&self) -> u64 {
        self.elements_since_yield
    }
}

// ---------------------------------------------------------------------------
// FairnessMonitor
// ---------------------------------------------------------------------------

/// Tracks per-flow CPU time for inter-flow fairness enforcement.
///
/// The monitor accumulates wall-clock processing time per flow and can
/// report whether a flow is exceeding its fair share. Integration with
/// Wasmtime fuel is handled externally: the reactor sets fuel budgets
/// on component instances before invocation.
///
/// # WARM PATH — updated per element.
pub struct FairnessMonitor {
    /// Cumulative CPU (wall-clock) time spent processing this flow.
    total_cpu_time: std::time::Duration,
    /// Total elements processed by this flow.
    total_elements: u64,
    /// Total yields performed by this flow.
    total_yields: u64,
    /// Wasmtime fuel consumed (if fuel metering is enabled).
    total_fuel_consumed: u64,
    /// Priority-adjusted elements-per-yield for this flow.
    elements_per_yield: u64,
}

impl FairnessMonitor {
    /// Create a new `FairnessMonitor` for a flow with the given priority.
    ///
    /// # COLD PATH — called once per flow.
    pub fn new(priority: FlowPriority) -> Self {
        Self {
            total_cpu_time: std::time::Duration::ZERO,
            total_elements: 0,
            total_yields: 0,
            total_fuel_consumed: 0,
            elements_per_yield: priority.elements_per_yield(),
        }
    }

    /// Record processing time for an element.
    ///
    /// # HOT PATH — zero-alloc.
    #[inline]
    pub fn record_processing(&mut self, duration: std::time::Duration) {
        self.total_cpu_time += duration;
        self.total_elements += 1;
    }

    /// Record fuel consumed by a component invocation.
    ///
    /// # HOT PATH — zero-alloc.
    #[inline]
    pub fn record_fuel_consumed(&mut self, fuel: u64) {
        self.total_fuel_consumed += fuel;
    }

    /// Record that a yield occurred.
    #[inline]
    pub fn record_yield(&mut self) {
        self.total_yields += 1;
    }

    /// Returns the total CPU time spent on this flow.
    #[inline]
    pub fn total_cpu_time(&self) -> std::time::Duration {
        self.total_cpu_time
    }

    /// Returns the total elements processed.
    #[inline]
    pub fn total_elements(&self) -> u64 {
        self.total_elements
    }

    /// Returns the total yields performed.
    #[inline]
    pub fn total_yields(&self) -> u64 {
        self.total_yields
    }

    /// Returns the total Wasmtime fuel consumed.
    #[inline]
    pub fn total_fuel_consumed(&self) -> u64 {
        self.total_fuel_consumed
    }

    /// Returns the configured elements-per-yield quantum.
    #[inline]
    pub fn elements_per_yield(&self) -> u64 {
        self.elements_per_yield
    }

    /// Returns the average processing time per element, or `None` if
    /// no elements have been processed.
    #[inline]
    pub fn avg_processing_time(&self) -> Option<std::time::Duration> {
        if self.total_elements == 0 {
            None
        } else {
            Some(self.total_cpu_time / self.total_elements as u32)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_flow_priority_elements_per_yield() {
        assert_eq!(FlowPriority::Critical.elements_per_yield(), 128);
        assert_eq!(FlowPriority::High.elements_per_yield(), 64);
        assert_eq!(FlowPriority::Normal.elements_per_yield(), 32);
        assert_eq!(FlowPriority::Low.elements_per_yield(), 16);
        assert_eq!(FlowPriority::Background.elements_per_yield(), 8);
    }

    #[test]
    fn test_flow_priority_ordering() {
        assert!(FlowPriority::Background < FlowPriority::Low);
        assert!(FlowPriority::Low < FlowPriority::Normal);
        assert!(FlowPriority::Normal < FlowPriority::High);
        assert!(FlowPriority::High < FlowPriority::Critical);
    }

    #[test]
    fn test_flow_priority_default() {
        assert_eq!(FlowPriority::default(), FlowPriority::Normal);
    }

    #[test]
    fn test_yield_controller_element_count() {
        let mut ctrl = YieldController::new(4, Duration::from_secs(10));
        assert!(!ctrl.should_yield());
        ctrl.record_element();
        ctrl.record_element();
        ctrl.record_element();
        assert!(!ctrl.should_yield());
        ctrl.record_element();
        assert!(ctrl.should_yield());
    }

    #[test]
    fn test_yield_controller_reset() {
        let mut ctrl = YieldController::new(2, Duration::from_secs(10));
        ctrl.record_element();
        ctrl.record_element();
        assert!(ctrl.should_yield());
        ctrl.reset();
        assert!(!ctrl.should_yield());
        assert_eq!(ctrl.elements_since_yield(), 0);
    }

    #[test]
    fn test_yield_controller_clamps_to_ceiling() {
        let ctrl = YieldController::new(999, Duration::from_secs(10));
        // Should be clamped to MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD
        assert!(ctrl.elements_per_yield <= MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD);
    }

    #[test]
    fn test_fairness_monitor_initial_state() {
        let mon = FairnessMonitor::new(FlowPriority::Normal);
        assert_eq!(mon.total_elements(), 0);
        assert_eq!(mon.total_yields(), 0);
        assert_eq!(mon.total_fuel_consumed(), 0);
        assert_eq!(mon.total_cpu_time(), Duration::ZERO);
        assert!(mon.avg_processing_time().is_none());
        assert_eq!(mon.elements_per_yield(), 32);
    }

    #[test]
    fn test_fairness_monitor_record_processing() {
        let mut mon = FairnessMonitor::new(FlowPriority::High);
        mon.record_processing(Duration::from_micros(100));
        mon.record_processing(Duration::from_micros(200));
        assert_eq!(mon.total_elements(), 2);
        assert_eq!(mon.total_cpu_time(), Duration::from_micros(300));
        assert_eq!(mon.avg_processing_time(), Some(Duration::from_micros(150)));
    }

    #[test]
    fn test_fairness_monitor_fuel_tracking() {
        let mut mon = FairnessMonitor::new(FlowPriority::Normal);
        mon.record_fuel_consumed(1000);
        mon.record_fuel_consumed(500);
        assert_eq!(mon.total_fuel_consumed(), 1500);
    }

    #[test]
    fn test_fairness_monitor_yield_tracking() {
        let mut mon = FairnessMonitor::new(FlowPriority::Low);
        mon.record_yield();
        mon.record_yield();
        mon.record_yield();
        assert_eq!(mon.total_yields(), 3);
        assert_eq!(mon.elements_per_yield(), 16);
    }
}
