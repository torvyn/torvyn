//! Pre-allocated ring buffer for compact span records.
//!
//! Per HLI Doc 05 §2.3: each flow maintains a ring buffer of
//! `CompactSpanRecord` structs. If a promotion trigger fires, the buffer
//! is flushed to the export pipeline. Otherwise, records are overwritten.
//!
//! The ring buffer is single-producer (the flow's task) so no locking is needed.

use torvyn_types::{ComponentId, SpanId};

/// Compact span record for ring-buffer storage.
///
/// Target size: 64 bytes for cache efficiency.
///
/// # HOT PATH — written per invocation when sampled.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CompactSpanRecord {
    /// Span ID for this record.
    pub span_id: SpanId,
    /// Parent span ID.
    pub parent_span_id: SpanId,
    /// Component that was invoked.
    pub component_id: ComponentId,
    /// Invocation start time (ns since epoch).
    pub start_ns: u64,
    /// Invocation end time (ns since epoch).
    pub end_ns: u64,
    /// Invocation status (0 = ok, 1 = error, 2 = timeout, 3 = cancelled).
    pub status_code: u8,
    /// Element sequence number.
    pub element_sequence: u64,
}

impl CompactSpanRecord {
    /// Create a zeroed record.
    pub const fn zeroed() -> Self {
        Self {
            span_id: SpanId::invalid(),
            parent_span_id: SpanId::invalid(),
            component_id: ComponentId::new(0),
            start_ns: 0,
            end_ns: 0,
            status_code: 0,
            element_sequence: 0,
        }
    }

    /// Duration in nanoseconds.
    #[inline]
    pub fn duration_ns(&self) -> u64 {
        self.end_ns.saturating_sub(self.start_ns)
    }

    /// Whether this record represents an error.
    #[inline]
    pub fn is_error(&self) -> bool {
        self.status_code == 1
    }
}

/// Fixed-capacity ring buffer of compact span records.
///
/// # Invariants
/// - `capacity` is a power of two.
/// - `write_pos & mask` gives the current write index.
/// - Single-producer only (not thread-safe for writes).
///
/// # Memory
/// - 64 bytes per slot x capacity.
/// - Default 64 slots = 4 KiB per flow.
pub struct SpanRingBuffer {
    slots: Box<[CompactSpanRecord]>,
    /// Monotonically increasing write position.
    write_pos: u64,
    /// Bitmask for fast modulo: `capacity - 1`.
    mask: u64,
    /// Number of slots.
    capacity: usize,
}

impl SpanRingBuffer {
    /// Create a new ring buffer with the given capacity.
    ///
    /// # Preconditions
    /// - `capacity` must be a power of two and >= 8.
    ///
    /// # COLD PATH
    pub fn new(capacity: usize) -> Self {
        debug_assert!(capacity.is_power_of_two() && capacity >= 8);
        let slots = vec![CompactSpanRecord::zeroed(); capacity].into_boxed_slice();
        Self {
            slots,
            write_pos: 0,
            mask: (capacity as u64) - 1,
            capacity,
        }
    }

    /// Push a span record into the ring buffer.
    ///
    /// Overwrites the oldest record if full.
    ///
    /// # HOT PATH — zero-alloc.
    ///
    /// Must be called from a single producer (the flow's task).
    #[inline]
    pub fn push(&mut self, record: CompactSpanRecord) {
        let idx = (self.write_pos & self.mask) as usize;
        self.slots[idx] = record;
        self.write_pos = self.write_pos.wrapping_add(1);
    }

    /// Drain all valid records in order (oldest first).
    ///
    /// Returns records and resets the buffer.
    ///
    /// # COLD PATH — called on promotion trigger.
    pub fn drain(&mut self) -> Vec<CompactSpanRecord> {
        let total = self.write_pos.min(self.capacity as u64) as usize;
        let mut result = Vec::with_capacity(total);

        if self.write_pos <= self.capacity as u64 {
            // Buffer hasn't wrapped.
            for i in 0..total {
                result.push(self.slots[i]);
            }
        } else {
            // Buffer has wrapped. Start from oldest.
            let start = (self.write_pos & self.mask) as usize;
            for i in 0..self.capacity {
                let idx = (start + i) % self.capacity;
                result.push(self.slots[idx]);
            }
        }

        self.write_pos = 0;
        result
    }

    /// Number of records currently in the buffer.
    #[inline]
    pub fn len(&self) -> usize {
        self.write_pos.min(self.capacity as u64) as usize
    }

    /// Whether the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.write_pos == 0
    }

    /// Whether the buffer has been fully written at least once.
    #[inline]
    pub fn has_wrapped(&self) -> bool {
        self.write_pos >= self.capacity as u64
    }

    /// Buffer capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Check if any record in the buffer is an error.
    ///
    /// # COLD PATH — linear scan.
    pub fn contains_error(&self) -> bool {
        let total = self.len();
        for i in 0..total {
            let idx = if self.has_wrapped() {
                ((self.write_pos & self.mask) as usize + i) % self.capacity
            } else {
                i
            };
            if self.slots[idx].is_error() {
                return true;
            }
        }
        false
    }
}

impl std::fmt::Debug for SpanRingBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanRingBuffer")
            .field("capacity", &self.capacity)
            .field("len", &self.len())
            .field("write_pos", &self.write_pos)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(seq: u64, start: u64, end: u64) -> CompactSpanRecord {
        CompactSpanRecord {
            span_id: SpanId::new([seq as u8; 8]),
            parent_span_id: SpanId::invalid(),
            component_id: ComponentId::new(1),
            start_ns: start,
            end_ns: end,
            status_code: 0,
            element_sequence: seq,
        }
    }

    fn make_error_record(seq: u64) -> CompactSpanRecord {
        let mut r = make_record(seq, 0, 100);
        r.status_code = 1;
        r
    }

    #[test]
    fn test_ring_buffer_new() {
        let rb = SpanRingBuffer::new(16);
        assert_eq!(rb.capacity(), 16);
        assert_eq!(rb.len(), 0);
        assert!(rb.is_empty());
    }

    #[test]
    fn test_ring_buffer_push_and_len() {
        let mut rb = SpanRingBuffer::new(8);
        rb.push(make_record(1, 0, 100));
        assert_eq!(rb.len(), 1);
        rb.push(make_record(2, 100, 200));
        assert_eq!(rb.len(), 2);
    }

    #[test]
    fn test_ring_buffer_drain_no_wrap() {
        let mut rb = SpanRingBuffer::new(8);
        for i in 0..5 {
            rb.push(make_record(i, i * 100, (i + 1) * 100));
        }

        let records = rb.drain();
        assert_eq!(records.len(), 5);
        assert_eq!(records[0].element_sequence, 0);
        assert_eq!(records[4].element_sequence, 4);
        assert!(rb.is_empty());
    }

    #[test]
    fn test_ring_buffer_wrap_and_drain() {
        let mut rb = SpanRingBuffer::new(8);
        // Write 12 records into an 8-slot buffer.
        for i in 0..12 {
            rb.push(make_record(i, i * 100, (i + 1) * 100));
        }

        assert!(rb.has_wrapped());
        assert_eq!(rb.len(), 8);

        let records = rb.drain();
        assert_eq!(records.len(), 8);
        // Oldest should be record 4 (12 - 8).
        assert_eq!(records[0].element_sequence, 4);
        assert_eq!(records[7].element_sequence, 11);
    }

    #[test]
    fn test_ring_buffer_contains_error() {
        let mut rb = SpanRingBuffer::new(8);
        rb.push(make_record(0, 0, 100));
        assert!(!rb.contains_error());

        rb.push(make_error_record(1));
        assert!(rb.contains_error());
    }

    #[test]
    fn test_compact_span_record_duration() {
        let r = make_record(0, 100, 350);
        assert_eq!(r.duration_ns(), 250);
    }

    #[test]
    fn test_ring_buffer_drain_resets() {
        let mut rb = SpanRingBuffer::new(8);
        rb.push(make_record(0, 0, 100));
        let _ = rb.drain();
        assert!(rb.is_empty());
        assert_eq!(rb.len(), 0);
    }
}
