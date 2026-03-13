//! Backpressure state machine with watermark hysteresis.
//!
//! Per Doc 04 §5.5: backpressure activates at full capacity and deactivates
//! at the low watermark. This prevents oscillation.

use std::time::Instant;

// ---------------------------------------------------------------------------
// BackpressureState
// ---------------------------------------------------------------------------

/// Backpressure state for a stream.
///
/// Two states with hysteresis: Normal → Active when queue is full,
/// Active → Normal when queue drops below low watermark.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum BackpressureState {
    /// Queue depth is below high watermark. Normal operation.
    #[default]
    Normal,
    /// Queue is at or near capacity. Producer is suspended.
    Active {
        /// Timestamp when backpressure was activated.
        since: Instant,
    },
}

impl BackpressureState {
    /// Returns `true` if backpressure is currently active.
    ///
    /// # HOT PATH — checked per scheduling decision.
    #[inline(always)]
    pub fn is_active(&self) -> bool {
        matches!(self, BackpressureState::Active { .. })
    }

    /// Attempt to activate backpressure.
    ///
    /// Returns `true` if the transition occurred (was Normal, now Active).
    /// Returns `false` if already active (no-op).
    ///
    /// # WARM PATH — called when queue reaches capacity.
    pub fn try_activate(&mut self) -> bool {
        match self {
            BackpressureState::Normal => {
                *self = BackpressureState::Active {
                    since: Instant::now(),
                };
                true
            }
            BackpressureState::Active { .. } => false,
        }
    }

    /// Attempt to deactivate backpressure.
    ///
    /// Returns `Some(duration)` if the transition occurred, where
    /// `duration` is how long backpressure was active. Returns `None`
    /// if already normal (no-op).
    ///
    /// # WARM PATH — called when queue drops below low watermark.
    pub fn try_deactivate(&mut self) -> Option<std::time::Duration> {
        match self {
            BackpressureState::Active { since } => {
                let duration = since.elapsed();
                *self = BackpressureState::Normal;
                Some(duration)
            }
            BackpressureState::Normal => None,
        }
    }
}

// LLI DEVIATION: Default derived via #[derive(Default)] + #[default] instead of manual impl per clippy.

// ---------------------------------------------------------------------------
// check_backpressure — stateless decision function
// ---------------------------------------------------------------------------

/// Determine whether to activate or deactivate backpressure based on
/// queue depth and watermarks.
///
/// # HOT PATH — called per element to check backpressure transitions.
///
/// # Returns
/// - `Some(true)` → activate backpressure (queue full).
/// - `Some(false)` → deactivate backpressure (below low watermark).
/// - `None` → no change needed.
#[inline]
pub fn check_backpressure_transition(
    queue_len: usize,
    queue_capacity: usize,
    low_watermark_depth: usize,
    currently_active: bool,
) -> Option<bool> {
    if !currently_active && queue_len >= queue_capacity {
        Some(true) // activate
    } else if currently_active && queue_len <= low_watermark_depth {
        Some(false) // deactivate
    } else {
        None // no change
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_backpressure_initial_state() {
        let state = BackpressureState::default();
        assert!(!state.is_active());
    }

    #[test]
    fn test_backpressure_activate() {
        let mut state = BackpressureState::Normal;
        assert!(state.try_activate());
        assert!(state.is_active());
    }

    #[test]
    fn test_backpressure_activate_twice_is_noop() {
        let mut state = BackpressureState::Normal;
        assert!(state.try_activate());
        assert!(!state.try_activate()); // second call is no-op
    }

    #[test]
    fn test_backpressure_deactivate() {
        let mut state = BackpressureState::Normal;
        state.try_activate();
        thread::sleep(Duration::from_millis(1));
        let duration = state.try_deactivate().unwrap();
        assert!(duration >= Duration::from_millis(1));
        assert!(!state.is_active());
    }

    #[test]
    fn test_backpressure_deactivate_when_normal_is_none() {
        let mut state = BackpressureState::Normal;
        assert!(state.try_deactivate().is_none());
    }

    #[test]
    fn test_check_backpressure_activate_at_capacity() {
        // Not active, queue at capacity → activate
        assert_eq!(check_backpressure_transition(64, 64, 32, false), Some(true));
    }

    #[test]
    fn test_check_backpressure_no_change_below_capacity() {
        // Not active, queue below capacity → no change
        assert_eq!(check_backpressure_transition(50, 64, 32, false), None);
    }

    #[test]
    fn test_check_backpressure_deactivate_below_watermark() {
        // Active, queue below low watermark → deactivate
        assert_eq!(check_backpressure_transition(30, 64, 32, true), Some(false));
    }

    #[test]
    fn test_check_backpressure_no_change_above_watermark() {
        // Active, queue above low watermark → no change (hysteresis)
        assert_eq!(check_backpressure_transition(40, 64, 32, true), None);
    }

    #[test]
    fn test_check_backpressure_deactivate_at_watermark() {
        // Active, queue exactly at low watermark → deactivate
        assert_eq!(check_backpressure_transition(32, 64, 32, true), Some(false));
    }
}
