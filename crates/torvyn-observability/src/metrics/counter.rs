//! Atomic monotonic counter for hot-path metrics.
//!
//! Uses `Relaxed` ordering: callers accept momentary staleness on reads
//! in exchange for zero contention on the hot path.

use std::sync::atomic::{AtomicU64, Ordering};

/// A monotonically increasing atomic counter.
///
/// # Invariants
/// - Value only increases (or resets to zero).
/// - Thread-safe without locks.
///
/// # Performance
/// - `increment`: ~1ns (single `fetch_add` with `Relaxed`).
/// - `read`: ~1ns (single `load` with `Relaxed`).
///
/// # Examples
/// ```
/// use torvyn_observability::metrics::Counter;
///
/// let counter = Counter::new();
/// counter.increment(5);
/// assert_eq!(counter.read(), 5);
/// ```
pub struct Counter {
    value: AtomicU64,
}

impl Counter {
    /// Create a new counter initialized to zero.
    ///
    /// # COLD PATH — called during flow registration.
    #[inline]
    pub const fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    /// Increment the counter by `delta`.
    ///
    /// # HOT PATH — zero-alloc, lock-free.
    ///
    /// # Postconditions
    /// - Counter value increased by exactly `delta`.
    #[inline]
    pub fn increment(&self, delta: u64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
    }

    /// Read the current counter value.
    ///
    /// # WARM PATH — called by export and snapshot.
    ///
    /// The returned value may be momentarily stale relative to concurrent
    /// `increment` calls. This is acceptable for metrics export.
    #[inline]
    pub fn read(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Reset the counter to zero and return the previous value.
    ///
    /// # COLD PATH — called during flow deregistration.
    #[inline]
    pub fn reset(&self) -> u64 {
        self.value.swap(0, Ordering::Relaxed)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Counter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Counter")
            .field("value", &self.read())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_new_is_zero() {
        let c = Counter::new();
        assert_eq!(c.read(), 0);
    }

    #[test]
    fn test_counter_increment() {
        let c = Counter::new();
        c.increment(1);
        assert_eq!(c.read(), 1);
        c.increment(99);
        assert_eq!(c.read(), 100);
    }

    #[test]
    fn test_counter_increment_by_zero() {
        let c = Counter::new();
        c.increment(0);
        assert_eq!(c.read(), 0);
    }

    #[test]
    fn test_counter_reset() {
        let c = Counter::new();
        c.increment(42);
        let prev = c.reset();
        assert_eq!(prev, 42);
        assert_eq!(c.read(), 0);
    }

    #[test]
    fn test_counter_concurrent_increments() {
        use std::sync::Arc;
        use std::thread;

        let c = Arc::new(Counter::new());
        let mut handles = Vec::new();

        for _ in 0..10 {
            let c = Arc::clone(&c);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    c.increment(1);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(c.read(), 10_000);
    }

    #[test]
    fn test_counter_debug() {
        let c = Counter::new();
        c.increment(7);
        let debug = format!("{:?}", c);
        assert!(debug.contains("7"));
    }
}
