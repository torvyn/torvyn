//! Project-wide constants for the Torvyn runtime.
//!
//! Includes version information, default capacity values, and performance
//! budget constants.

/// The current Torvyn runtime version (semver).
pub const TORVYN_VERSION: &str = "0.1.0";

/// The minimum compatible Torvyn version for artifacts.
/// Artifacts produced by versions older than this cannot be loaded.
pub const MIN_COMPATIBLE_VERSION: &str = "0.1.0";

/// Default stream queue depth.
///
/// Per consolidated review (C02-2): changed from 1024 to 64.
/// This is the number of stream elements a queue can hold before
/// backpressure activates.
pub const DEFAULT_QUEUE_DEPTH: usize = 64;

/// Default buffer pool size per tier (number of buffers).
pub const DEFAULT_BUFFER_POOL_SIZE: usize = 256;

/// Maximum allowed per-element host-side overhead in nanoseconds.
///
/// Per Doc 04, Section 11.1: the reactor targets < 5us overhead.
/// This constant represents the total budget across all subsystems.
pub const MAX_HOT_PATH_NS: u64 = 5_000;

/// Maximum allowed observability overhead at Production level, in nanoseconds.
///
/// Per Doc 05, Section 8.1: < 500ns per element at Production level.
pub const MAX_OBSERVABILITY_PRODUCTION_NS: u64 = 500;

/// Maximum allowed observability overhead at Diagnostic level, in nanoseconds.
///
/// Per Doc 05, Section 8.1: < 2us per element at Diagnostic level.
pub const MAX_OBSERVABILITY_DIAGNOSTIC_NS: u64 = 2_000;

/// Default low watermark ratio for backpressure recovery.
///
/// When a queue's depth drops below `capacity * LOW_WATERMARK_RATIO`,
/// backpressure is deactivated.
pub const DEFAULT_LOW_WATERMARK_RATIO: f64 = 0.5;

/// Maximum buffer size in bytes (global cap).
///
/// Per Doc 03, Section 4.3: 16 MiB global maximum.
pub const MAX_BUFFER_SIZE_BYTES: u32 = 16 * 1024 * 1024;

/// Default component invocation timeout in milliseconds.
///
/// Per Doc 04, Section 6.5: 5 seconds.
pub const DEFAULT_COMPONENT_TIMEOUT_MS: u64 = 5_000;

/// Default drain timeout in milliseconds.
///
/// Per Doc 04, Section 6.5: 5 seconds.
pub const DEFAULT_DRAIN_TIMEOUT_MS: u64 = 5_000;

/// Default cooperative cancellation timeout in milliseconds.
///
/// Per Doc 04, Section 6.5: 1 second.
pub const DEFAULT_COOPERATIVE_CANCEL_TIMEOUT_MS: u64 = 1_000;

/// Default maximum consecutive elements before a flow driver must yield.
///
/// Per Doc 04, Section 7.3: 256 elements hard ceiling.
pub const MAX_CONSECUTIVE_ELEMENTS_BEFORE_YIELD: u64 = 256;

/// Default elements per yield at Normal priority.
///
/// Per Doc 04, Section 7.2: 32 elements.
pub const DEFAULT_ELEMENTS_PER_YIELD: u64 = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_is_semver() {
        let parts: Vec<&str> = TORVYN_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "version must be semver (major.minor.patch)");
        for part in parts {
            part.parse::<u32>().expect("each version component must be a number");
        }
    }

    #[test]
    fn test_default_queue_depth_is_power_of_two_minus_one_or_reasonable() {
        assert!(DEFAULT_QUEUE_DEPTH > 0);
        assert!(DEFAULT_QUEUE_DEPTH <= 1024);
    }

    #[test]
    fn test_max_buffer_size_is_16_mib() {
        assert_eq!(MAX_BUFFER_SIZE_BYTES, 16 * 1024 * 1024);
    }

    #[test]
    fn test_hot_path_budget_is_5_microseconds() {
        assert_eq!(MAX_HOT_PATH_NS, 5_000);
    }

    #[test]
    fn test_low_watermark_ratio_is_valid() {
        assert!(DEFAULT_LOW_WATERMARK_RATIO > 0.0);
        assert!(DEFAULT_LOW_WATERMARK_RATIO < 1.0);
    }
}
