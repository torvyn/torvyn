//! Fixed-bucket histogram for distribution metrics.
//!
//! Pre-allocated bucket array with atomic counts. Recording is O(log n)
//! via binary search over bucket boundaries, followed by one atomic increment.
//!
//! Per HLI Doc 05 §3.4 and MR-12: fixed-bucket for v0.1.

use std::sync::atomic::{AtomicU64, Ordering};

/// Pre-defined histogram bucket boundaries for latency (nanoseconds).
///
/// Covers 100ns to 10s with logarithmic distribution (24 buckets + overflow).
/// Per HLI Doc 05 §3.4.
pub const LATENCY_BUCKETS_NS: &[u64] = &[
    100,
    250,
    500,
    1_000,
    2_500,
    5_000,
    10_000,
    25_000,
    50_000,
    100_000,
    250_000,
    500_000,
    1_000_000,
    2_500_000,
    5_000_000,
    10_000_000,
    25_000_000,
    50_000_000,
    100_000_000,
    250_000_000,
    500_000_000,
    1_000_000_000,
    2_500_000_000,
    5_000_000_000,
    10_000_000_000,
];

/// Pre-defined histogram bucket boundaries for sizes (bytes).
///
/// Covers 64B to 16 MiB.
pub const SIZE_BUCKETS_BYTES: &[u64] = &[
    64,
    256,
    1_024,
    4_096,
    16_384,
    65_536,
    262_144,
    1_048_576,
    4_194_304,
    16_777_216,
];

/// A fixed-bucket histogram with atomic bucket counts.
///
/// # Invariants
/// - `buckets.len() == boundaries.len() + 1` (last bucket is overflow).
/// - `boundaries` is sorted ascending.
/// - `count == sum of all bucket values`.
///
/// # Performance
/// - `record`: ~5 comparisons (binary search over 24 buckets) + 2 atomic
///   increments (`sum` + bucket). Target: < 50ns.
///
/// # Examples
/// ```
/// use torvyn_observability::metrics::Histogram;
/// use torvyn_observability::metrics::LATENCY_BUCKETS_NS;
///
/// let h = Histogram::new(LATENCY_BUCKETS_NS);
/// h.record(500);
/// h.record(1_000_000);
/// assert_eq!(h.count(), 2);
/// ```
pub struct Histogram {
    /// One count per bucket, plus an overflow bucket at the end.
    buckets: Box<[AtomicU64]>,
    /// Sum of all recorded values for mean computation.
    sum: AtomicU64,
    /// Total observation count.
    count: AtomicU64,
    /// Minimum observed value.
    min: AtomicU64,
    /// Maximum observed value.
    max: AtomicU64,
    /// Bucket upper boundaries (exclusive). Values > last boundary go to overflow.
    boundaries: &'static [u64],
}

impl Histogram {
    /// Create a new histogram with the given bucket boundaries.
    ///
    /// # Preconditions
    /// - `boundaries` must be sorted ascending.
    /// - `boundaries` must not be empty.
    ///
    /// # COLD PATH — called during flow registration.
    pub fn new(boundaries: &'static [u64]) -> Self {
        debug_assert!(
            !boundaries.is_empty(),
            "histogram boundaries must not be empty"
        );
        debug_assert!(
            boundaries.windows(2).all(|w| w[0] < w[1]),
            "histogram boundaries must be sorted ascending"
        );

        let bucket_count = boundaries.len() + 1; // +1 for overflow
        let mut buckets = Vec::with_capacity(bucket_count);
        for _ in 0..bucket_count {
            buckets.push(AtomicU64::new(0));
        }

        Self {
            buckets: buckets.into_boxed_slice(),
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
            min: AtomicU64::new(u64::MAX),
            max: AtomicU64::new(0),
            boundaries,
        }
    }

    /// Record a value into the histogram.
    ///
    /// Uses binary search to find the correct bucket, then atomically
    /// increments the bucket count, sum, and total count.
    ///
    /// # HOT PATH — zero-alloc, lock-free.
    ///
    /// # Postconditions
    /// - Exactly one bucket incremented by 1.
    /// - `sum` incremented by `value`.
    /// - `count` incremented by 1.
    #[inline]
    pub fn record(&self, value: u64) {
        // Binary search: find first boundary > value.
        let bucket_idx = match self.boundaries.binary_search(&value) {
            // Exact match: value equals boundary[i], goes into bucket i.
            Ok(i) => i,
            Err(i) => i,
        };

        self.buckets[bucket_idx].fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        // Update min/max with CAS loops.
        let mut current_min = self.min.load(Ordering::Relaxed);
        while value < current_min {
            match self.min.compare_exchange_weak(
                current_min,
                value,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_min = actual,
            }
        }

        let mut current_max = self.max.load(Ordering::Relaxed);
        while value > current_max {
            match self.max.compare_exchange_weak(
                current_max,
                value,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_max = actual,
            }
        }
    }

    /// Read the total observation count.
    ///
    /// # WARM PATH
    #[inline]
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Read the sum of all observed values.
    ///
    /// # WARM PATH
    #[inline]
    pub fn sum(&self) -> u64 {
        self.sum.load(Ordering::Relaxed)
    }

    /// Read the minimum observed value. Returns `u64::MAX` if no values recorded.
    ///
    /// # WARM PATH
    #[inline]
    pub fn min(&self) -> u64 {
        self.min.load(Ordering::Relaxed)
    }

    /// Read the maximum observed value. Returns 0 if no values recorded.
    ///
    /// # WARM PATH
    #[inline]
    pub fn max(&self) -> u64 {
        self.max.load(Ordering::Relaxed)
    }

    /// Compute the mean of all observed values.
    ///
    /// Returns 0.0 if no values have been recorded.
    ///
    /// # COLD PATH
    pub fn mean(&self) -> f64 {
        let c = self.count();
        if c == 0 {
            return 0.0;
        }
        self.sum() as f64 / c as f64
    }

    /// Take a snapshot of all bucket counts for percentile computation.
    ///
    /// Returns `(boundaries, bucket_counts)` where `bucket_counts.len() ==
    /// boundaries.len() + 1`.
    ///
    /// # COLD PATH — allocates a Vec.
    pub fn snapshot(&self) -> HistogramSnapshot {
        let counts: Vec<u64> = self
            .buckets
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect();

        HistogramSnapshot {
            boundaries: self.boundaries,
            counts,
            total_count: self.count(),
            sum: self.sum(),
            min: self.min(),
            max: self.max(),
        }
    }

    /// Reset all buckets, sum, count, min, and max to zero.
    ///
    /// # COLD PATH
    pub fn reset(&self) {
        for bucket in self.buckets.iter() {
            bucket.store(0, Ordering::Relaxed);
        }
        self.sum.store(0, Ordering::Relaxed);
        self.count.store(0, Ordering::Relaxed);
        self.min.store(u64::MAX, Ordering::Relaxed);
        self.max.store(0, Ordering::Relaxed);
    }

    /// Returns the bucket boundaries.
    pub fn boundaries(&self) -> &'static [u64] {
        self.boundaries
    }
}

impl std::fmt::Debug for Histogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Histogram")
            .field("count", &self.count())
            .field("sum", &self.sum())
            .field("min", &self.min())
            .field("max", &self.max())
            .field("buckets", &self.boundaries.len())
            .finish()
    }
}

/// Point-in-time snapshot of a histogram for percentile computation.
#[derive(Clone, Debug)]
pub struct HistogramSnapshot {
    /// Bucket upper boundaries.
    pub boundaries: &'static [u64],
    /// Count per bucket (len = boundaries.len() + 1).
    pub counts: Vec<u64>,
    /// Total count at snapshot time.
    pub total_count: u64,
    /// Sum at snapshot time.
    pub sum: u64,
    /// Min at snapshot time.
    pub min: u64,
    /// Max at snapshot time.
    pub max: u64,
}

impl HistogramSnapshot {
    /// Compute a percentile value using linear interpolation within buckets.
    ///
    /// # Preconditions
    /// - `percentile` must be in `[0.0, 100.0]`.
    ///
    /// # Returns
    /// The estimated value at the given percentile, or 0 if no data.
    ///
    /// # COLD PATH — called during report generation.
    pub fn percentile(&self, percentile: f64) -> u64 {
        debug_assert!(
            (0.0..=100.0).contains(&percentile),
            "percentile must be in [0.0, 100.0], got {percentile}"
        );

        if self.total_count == 0 {
            return 0;
        }

        let target = (percentile / 100.0 * self.total_count as f64).ceil() as u64;
        let target = target.max(1);

        let mut cumulative: u64 = 0;
        for (i, &count) in self.counts.iter().enumerate() {
            cumulative += count;
            if cumulative >= target {
                // Found the bucket. Interpolate within it.
                let lower = if i == 0 { 0 } else { self.boundaries[i - 1] };
                let upper = if i < self.boundaries.len() {
                    self.boundaries[i]
                } else {
                    // Overflow bucket: use max observed value.
                    self.max
                };

                if count == 0 {
                    return upper;
                }

                // Linear interpolation within bucket.
                let within_bucket = cumulative - target;
                let fraction = 1.0 - (within_bucket as f64 / count as f64);
                let value = lower as f64 + fraction * (upper - lower) as f64;
                return value as u64;
            }
        }

        // Should not reach here if total_count > 0.
        self.max
    }

    /// Compute mean from snapshot.
    pub fn mean(&self) -> f64 {
        if self.total_count == 0 {
            0.0
        } else {
            self.sum as f64 / self.total_count as f64
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Small bucket set for testing.
    static TEST_BUCKETS: &[u64] = &[10, 50, 100, 500, 1000];

    #[test]
    fn test_histogram_new() {
        let h = Histogram::new(TEST_BUCKETS);
        assert_eq!(h.count(), 0);
        assert_eq!(h.sum(), 0);
        assert_eq!(h.min(), u64::MAX);
        assert_eq!(h.max(), 0);
    }

    #[test]
    fn test_histogram_record_single() {
        let h = Histogram::new(TEST_BUCKETS);
        h.record(25);
        assert_eq!(h.count(), 1);
        assert_eq!(h.sum(), 25);
        assert_eq!(h.min(), 25);
        assert_eq!(h.max(), 25);
    }

    #[test]
    fn test_histogram_record_into_first_bucket() {
        let h = Histogram::new(TEST_BUCKETS);
        h.record(5);
        let snap = h.snapshot();
        assert_eq!(snap.counts[0], 1); // bucket [0, 10)
    }

    #[test]
    fn test_histogram_record_at_boundary() {
        let h = Histogram::new(TEST_BUCKETS);
        h.record(10); // exactly at boundary
        let snap = h.snapshot();
        // binary_search returns Ok(0) for value 10, so bucket 0
        assert_eq!(snap.counts[0], 1);
    }

    #[test]
    fn test_histogram_record_into_overflow() {
        let h = Histogram::new(TEST_BUCKETS);
        h.record(5000); // beyond all boundaries
        let snap = h.snapshot();
        let overflow_idx = TEST_BUCKETS.len();
        assert_eq!(snap.counts[overflow_idx], 1);
    }

    #[test]
    fn test_histogram_multiple_records() {
        let h = Histogram::new(TEST_BUCKETS);
        for v in [5, 25, 75, 250, 750, 5000] {
            h.record(v);
        }
        assert_eq!(h.count(), 6);
        assert_eq!(h.sum(), 5 + 25 + 75 + 250 + 750 + 5000);
        assert_eq!(h.min(), 5);
        assert_eq!(h.max(), 5000);
    }

    #[test]
    fn test_histogram_reset() {
        let h = Histogram::new(TEST_BUCKETS);
        h.record(100);
        h.record(200);
        h.reset();
        assert_eq!(h.count(), 0);
        assert_eq!(h.sum(), 0);
    }

    #[test]
    fn test_histogram_mean() {
        let h = Histogram::new(TEST_BUCKETS);
        h.record(100);
        h.record(200);
        h.record(300);
        assert!((h.mean() - 200.0).abs() < 0.01);
    }

    #[test]
    fn test_histogram_mean_empty() {
        let h = Histogram::new(TEST_BUCKETS);
        assert_eq!(h.mean(), 0.0);
    }

    #[test]
    fn test_histogram_snapshot_percentile_p50() {
        let h = Histogram::new(TEST_BUCKETS);
        // Record 100 values: 1..=100
        for v in 1..=100 {
            h.record(v);
        }
        let snap = h.snapshot();
        let p50 = snap.percentile(50.0);
        // Values 1..=100, p50 should be around 50.
        // With bucket boundaries [10, 50, 100, 500, 1000],
        // bucket [10,50) contains values 11..50 (40 values).
        // p50 of 100 values = value at rank 50.
        assert!(p50 > 0 && p50 <= 100, "p50 was {p50}");
    }

    #[test]
    fn test_histogram_snapshot_percentile_empty() {
        let h = Histogram::new(TEST_BUCKETS);
        let snap = h.snapshot();
        assert_eq!(snap.percentile(50.0), 0);
    }

    #[test]
    fn test_histogram_concurrent_records() {
        use std::sync::Arc;
        use std::thread;

        let h = Arc::new(Histogram::new(TEST_BUCKETS));
        let mut handles = Vec::new();

        for _ in 0..8 {
            let h = Arc::clone(&h);
            handles.push(thread::spawn(move || {
                for v in 0..1000 {
                    h.record(v);
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(h.count(), 8000);
    }

    #[test]
    fn test_latency_buckets_sorted() {
        assert!(LATENCY_BUCKETS_NS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn test_size_buckets_sorted() {
        assert!(SIZE_BUCKETS_BYTES.windows(2).all(|w| w[0] < w[1]));
    }
}
