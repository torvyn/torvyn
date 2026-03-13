//! Pre-allocated fixed-capacity ring buffer for stream elements.
//!
//! [`BoundedQueue<T>`] is the core buffering primitive between pipeline stages.
//! It is designed for the HOT PATH:
//! - Zero heap allocations after construction.
//! - O(1) push and pop.
//! - Single-threaded access (owned by the flow driver task).
//!
//! # Design Decision (DI-11)
//! Pre-allocated ring buffer over `VecDeque` because `VecDeque` may reallocate
//! on internal growth even when logical length is bounded.

use std::fmt;
use torvyn_types::BackpressurePolicy;

// ---------------------------------------------------------------------------
// BoundedQueue
// ---------------------------------------------------------------------------

/// A fixed-capacity FIFO ring buffer.
///
/// Elements are stored in a pre-allocated `Vec<Option<T>>` with `head` and
/// `tail` indices. The queue never grows or shrinks after construction.
///
/// # Invariants
/// - `capacity > 0`.
/// - `len <= capacity` at all times.
/// - `head` and `tail` are always in `[0, capacity)`.
/// - Slots in `[head, head+len) mod capacity` are `Some`.
/// - All other slots are `None`.
///
/// # Performance
/// - **HOT PATH** — `push`, `pop`, `len`, `is_full`, `is_empty` are O(1)
///   with zero allocations.
/// - `peek_front` is O(1).
pub struct BoundedQueue<T> {
    /// Pre-allocated storage.
    buf: Vec<Option<T>>,
    /// Index of the front element (next to pop).
    head: usize,
    /// Number of elements currently in the queue.
    len: usize,
    /// Fixed capacity (equals `buf.len()`).
    capacity: usize,
}

/// Result of a push operation.
#[derive(Debug, PartialEq, Eq)]
pub enum PushResult<T> {
    /// Element was successfully enqueued.
    Ok,
    /// Queue is full under `BlockProducer` policy; element NOT enqueued.
    Full(T),
    /// Oldest element was dropped to make room (returned here).
    DroppedOldest(T),
    /// The new element was dropped (not enqueued); returned here.
    DroppedNewest(T),
}

impl<T> BoundedQueue<T> {
    /// Create a new `BoundedQueue` with the given capacity.
    ///
    /// # Panics
    /// Panics if `capacity == 0`.
    ///
    /// # COLD PATH — called once per stream during flow setup.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "BoundedQueue capacity must be > 0");
        let mut buf = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buf.push(None);
        }
        Self {
            buf,
            head: 0,
            len: 0,
            capacity,
        }
    }

    /// Returns the fixed capacity of this queue.
    ///
    /// # HOT PATH — zero-cost.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of elements currently in the queue.
    ///
    /// # HOT PATH — zero-cost.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the queue contains no elements.
    ///
    /// # HOT PATH — zero-cost.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns `true` if the queue is at capacity.
    ///
    /// # HOT PATH — zero-cost.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.len == self.capacity
    }

    /// Compute the tail index (where the next element would be inserted).
    ///
    /// # HOT PATH
    #[inline(always)]
    fn tail(&self) -> usize {
        (self.head + self.len) % self.capacity
    }

    /// Push an element into the queue.
    ///
    /// If the queue is full, returns `PushResult::Full(element)`.
    /// The caller is responsible for applying backpressure policy.
    ///
    /// # HOT PATH — zero allocations.
    #[inline]
    pub fn push(&mut self, element: T) -> PushResult<T> {
        if self.len == self.capacity {
            PushResult::Full(element)
        } else {
            let tail = self.tail();
            self.buf[tail] = Some(element);
            self.len += 1;
            PushResult::Ok
        }
    }

    /// Push with a specific backpressure policy applied.
    ///
    /// - `BlockProducer` → returns `Full` if queue is full.
    /// - `DropOldest` → drops the oldest element, enqueues the new one.
    /// - `DropNewest` → drops the new element, does not enqueue.
    /// - `Error` → returns `Full` (caller converts to error).
    ///
    /// # HOT PATH — zero allocations.
    #[inline]
    pub fn push_with_policy(&mut self, element: T, policy: BackpressurePolicy) -> PushResult<T> {
        if self.len < self.capacity {
            let tail = self.tail();
            self.buf[tail] = Some(element);
            self.len += 1;
            return PushResult::Ok;
        }

        match policy {
            BackpressurePolicy::BlockProducer | BackpressurePolicy::Error => {
                PushResult::Full(element)
            }
            BackpressurePolicy::DropOldest => {
                // Remove oldest, insert new at head position, then advance head.
                let dropped = self.buf[self.head].take().expect(
                    "BoundedQueue invariant violation: head slot is None when len == capacity",
                );
                self.buf[self.head] = Some(element);
                self.head = (self.head + 1) % self.capacity;
                // len stays the same
                PushResult::DroppedOldest(dropped)
            }
            BackpressurePolicy::DropNewest => PushResult::DroppedNewest(element),
        }
    }

    /// Pop the front (oldest) element.
    ///
    /// Returns `None` if the queue is empty.
    ///
    /// # HOT PATH — zero allocations.
    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let element = self.buf[self.head].take().expect(
            "BoundedQueue invariant violation: head slot is None when len > 0",
        );
        self.head = (self.head + 1) % self.capacity;
        self.len -= 1;
        Some(element)
    }

    /// Peek at the front element without removing it.
    ///
    /// # HOT PATH — zero-cost reference.
    #[inline]
    pub fn peek_front(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            self.buf[self.head].as_ref()
        }
    }

    /// Drain all elements, returning them as a `Vec`.
    ///
    /// # WARM PATH — called during flow cleanup.
    pub fn drain_all(&mut self) -> Vec<T> {
        let mut result = Vec::with_capacity(self.len);
        while let Some(element) = self.pop() {
            result.push(element);
        }
        result
    }

    /// Returns the fill ratio (0.0 to 1.0).
    ///
    /// # HOT PATH — used for backpressure watermark checks.
    #[inline(always)]
    pub fn fill_ratio(&self) -> f64 {
        self.len as f64 / self.capacity as f64
    }
}

impl<T: fmt::Debug> fmt::Debug for BoundedQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundedQueue")
            .field("capacity", &self.capacity)
            .field("len", &self.len)
            .field("head", &self.head)
            .finish()
    }
}

impl<T> fmt::Display for BoundedQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BoundedQueue({}/{})", self.len, self.capacity)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::BackpressurePolicy;

    #[test]
    fn test_new_queue() {
        let q: BoundedQueue<u32> = BoundedQueue::new(8);
        assert_eq!(q.capacity(), 8);
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        assert!(!q.is_full());
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn test_zero_capacity_panics() {
        let _q: BoundedQueue<u32> = BoundedQueue::new(0);
    }

    #[test]
    fn test_push_pop_basic() {
        let mut q = BoundedQueue::new(4);
        assert!(matches!(q.push(10), PushResult::Ok));
        assert!(matches!(q.push(20), PushResult::Ok));
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(10));
        assert_eq!(q.pop(), Some(20));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn test_push_full_returns_full() {
        let mut q = BoundedQueue::new(2);
        assert!(matches!(q.push(1), PushResult::Ok));
        assert!(matches!(q.push(2), PushResult::Ok));
        assert!(q.is_full());
        match q.push(3) {
            PushResult::Full(v) => assert_eq!(v, 3),
            _ => panic!("expected Full"),
        }
    }

    #[test]
    fn test_push_with_policy_block() {
        let mut q = BoundedQueue::new(1);
        q.push(1);
        match q.push_with_policy(2, BackpressurePolicy::BlockProducer) {
            PushResult::Full(v) => assert_eq!(v, 2),
            _ => panic!("expected Full"),
        }
    }

    #[test]
    fn test_push_with_policy_drop_oldest() {
        let mut q = BoundedQueue::new(2);
        q.push(1);
        q.push(2);
        match q.push_with_policy(3, BackpressurePolicy::DropOldest) {
            PushResult::DroppedOldest(v) => assert_eq!(v, 1),
            _ => panic!("expected DroppedOldest"),
        }
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), Some(3));
    }

    #[test]
    fn test_push_with_policy_drop_newest() {
        let mut q = BoundedQueue::new(2);
        q.push(1);
        q.push(2);
        match q.push_with_policy(3, BackpressurePolicy::DropNewest) {
            PushResult::DroppedNewest(v) => assert_eq!(v, 3),
            _ => panic!("expected DroppedNewest"),
        }
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
    }

    #[test]
    fn test_push_with_policy_error() {
        let mut q = BoundedQueue::new(1);
        q.push(1);
        match q.push_with_policy(2, BackpressurePolicy::Error) {
            PushResult::Full(v) => assert_eq!(v, 2),
            _ => panic!("expected Full"),
        }
    }

    #[test]
    fn test_wrap_around() {
        let mut q = BoundedQueue::new(3);
        q.push(1);
        q.push(2);
        q.push(3);
        q.pop(); // remove 1, head advances
        q.pop(); // remove 2, head advances
        q.push(4); // wraps around
        q.push(5);
        assert_eq!(q.len(), 3);
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), Some(4));
        assert_eq!(q.pop(), Some(5));
    }

    #[test]
    fn test_peek_front() {
        let mut q = BoundedQueue::new(4);
        assert_eq!(q.peek_front(), None);
        q.push(42);
        assert_eq!(q.peek_front(), Some(&42));
        q.push(99);
        assert_eq!(q.peek_front(), Some(&42)); // still 42
    }

    #[test]
    fn test_drain_all() {
        let mut q = BoundedQueue::new(4);
        q.push(1);
        q.push(2);
        q.push(3);
        let drained = q.drain_all();
        assert_eq!(drained, vec![1, 2, 3]);
        assert!(q.is_empty());
    }

    #[test]
    fn test_fill_ratio() {
        let mut q = BoundedQueue::new(4);
        assert_eq!(q.fill_ratio(), 0.0);
        q.push(1);
        assert_eq!(q.fill_ratio(), 0.25);
        q.push(2);
        assert_eq!(q.fill_ratio(), 0.5);
        q.push(3);
        q.push(4);
        assert_eq!(q.fill_ratio(), 1.0);
    }

    #[test]
    fn test_heavy_wrap_around_stress() {
        let mut q = BoundedQueue::new(8);
        for i in 0..10_000u64 {
            if q.is_full() {
                let v = q.pop().unwrap();
                assert_eq!(v, i - 8);
            }
            assert!(matches!(q.push(i), PushResult::Ok));
        }
        // Queue should contain last 8 elements
        assert_eq!(q.len(), 8);
        for i in 0..8 {
            assert_eq!(q.pop(), Some(10_000 - 8 + i));
        }
    }

    #[test]
    fn test_stress_1m_operations() {
        let mut q = BoundedQueue::new(64);
        for i in 0..1_000_000u64 {
            if q.is_full() {
                q.pop();
            }
            assert!(matches!(q.push(i), PushResult::Ok));
            assert!(q.len() <= q.capacity());
        }
        assert!(q.len() <= 64);
    }

    #[test]
    fn test_display() {
        let mut q = BoundedQueue::new(16);
        q.push(1);
        q.push(2);
        assert_eq!(format!("{q}"), "BoundedQueue(2/16)");
    }
}
