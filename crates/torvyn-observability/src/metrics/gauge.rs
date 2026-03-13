//! Atomic gauge for current-value metrics.
//!
//! Unlike counters, gauges can go up and down. Uses `AtomicU64` for unsigned
//! values and `AtomicI64` for signed values.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// An unsigned atomic gauge.
///
/// # Invariants
/// - Represents a point-in-time value that can increase or decrease.
///
/// # Performance
/// - `set`, `read`: ~1ns each.
///
/// # Examples
/// ```
/// use torvyn_observability::metrics::Gauge;
///
/// let g = Gauge::new();
/// g.set(42);
/// assert_eq!(g.read(), 42);
/// g.increment(8);
/// assert_eq!(g.read(), 50);
/// ```
pub struct Gauge {
    value: AtomicU64,
}

impl Gauge {
    /// Create a new gauge initialized to zero.
    ///
    /// # COLD PATH
    #[inline]
    pub const fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    /// Set the gauge to an absolute value.
    ///
    /// # HOT PATH — zero-alloc, lock-free.
    #[inline]
    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
    }

    /// Increment the gauge by `delta`.
    ///
    /// # HOT PATH
    #[inline]
    pub fn increment(&self, delta: u64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
    }

    /// Decrement the gauge by `delta`, saturating at zero.
    ///
    /// # HOT PATH
    #[inline]
    pub fn decrement(&self, delta: u64) {
        // Saturating subtract via CAS loop.
        let mut current = self.value.load(Ordering::Relaxed);
        loop {
            let new = current.saturating_sub(delta);
            match self.value.compare_exchange_weak(
                current,
                new,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// Read the current gauge value.
    ///
    /// # WARM PATH
    #[inline]
    pub fn read(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Update the gauge to `value` if `value` is greater than the current value.
    /// Used for tracking high-water marks (peak values).
    ///
    /// # HOT PATH
    #[inline]
    pub fn update_max(&self, value: u64) {
        let mut current = self.value.load(Ordering::Relaxed);
        while value > current {
            match self.value.compare_exchange_weak(
                current,
                value,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Gauge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gauge")
            .field("value", &self.read())
            .finish()
    }
}

/// A signed atomic gauge for values that can be negative.
pub struct SignedGauge {
    value: AtomicI64,
}

impl SignedGauge {
    /// Create a new signed gauge initialized to zero.
    #[inline]
    pub const fn new() -> Self {
        Self {
            value: AtomicI64::new(0),
        }
    }

    /// Set the gauge to an absolute value.
    #[inline]
    pub fn set(&self, value: i64) {
        self.value.store(value, Ordering::Relaxed);
    }

    /// Add a delta (can be negative).
    #[inline]
    pub fn add(&self, delta: i64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
    }

    /// Read the current gauge value.
    #[inline]
    pub fn read(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }
}

impl Default for SignedGauge {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gauge_new_is_zero() {
        let g = Gauge::new();
        assert_eq!(g.read(), 0);
    }

    #[test]
    fn test_gauge_set() {
        let g = Gauge::new();
        g.set(100);
        assert_eq!(g.read(), 100);
    }

    #[test]
    fn test_gauge_increment() {
        let g = Gauge::new();
        g.increment(10);
        g.increment(20);
        assert_eq!(g.read(), 30);
    }

    #[test]
    fn test_gauge_decrement() {
        let g = Gauge::new();
        g.set(50);
        g.decrement(20);
        assert_eq!(g.read(), 30);
    }

    #[test]
    fn test_gauge_decrement_saturates_at_zero() {
        let g = Gauge::new();
        g.set(5);
        g.decrement(100);
        assert_eq!(g.read(), 0);
    }

    #[test]
    fn test_gauge_update_max() {
        let g = Gauge::new();
        g.update_max(10);
        assert_eq!(g.read(), 10);
        g.update_max(5); // no change
        assert_eq!(g.read(), 10);
        g.update_max(20);
        assert_eq!(g.read(), 20);
    }

    #[test]
    fn test_gauge_concurrent() {
        use std::sync::Arc;
        use std::thread;

        let g = Arc::new(Gauge::new());
        let mut handles = Vec::new();

        for _ in 0..10 {
            let g = Arc::clone(&g);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    g.increment(1);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(g.read(), 1000);
    }

    #[test]
    fn test_signed_gauge() {
        let g = SignedGauge::new();
        g.add(10);
        g.add(-3);
        assert_eq!(g.read(), 7);
    }
}
