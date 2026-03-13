//! Timestamp utilities for the Torvyn runtime.
//!
//! Provides consistent nanosecond-precision timestamps for element metadata,
//! transfer records, and observability events.

use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the current wall-clock time as nanoseconds since the Unix epoch.
///
/// This is used for `element-meta.timestamp_ns` and transfer records.
/// For latency measurement, prefer `std::time::Instant` instead.
///
/// # HOT PATH — called per stream element for timestamp assignment.
///
/// # Examples
/// ```
/// use torvyn_types::current_timestamp_ns;
///
/// let ts = current_timestamp_ns();
/// assert!(ts > 0);
/// ```
#[inline]
pub fn current_timestamp_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_timestamp_ns_is_positive() {
        let ts = current_timestamp_ns();
        assert!(ts > 0);
    }

    #[test]
    fn test_current_timestamp_ns_is_monotonically_nondecreasing() {
        let a = current_timestamp_ns();
        let b = current_timestamp_ns();
        assert!(b >= a);
    }

    #[test]
    fn test_current_timestamp_ns_is_plausible() {
        let ts = current_timestamp_ns();
        // Should be after 2024-01-01 (~1704067200 seconds = 1704067200_000_000_000 ns)
        assert!(ts > 1_704_067_200_000_000_000);
        // Should be before 2100-01-01
        assert!(ts < 4_102_444_800_000_000_000);
    }
}
