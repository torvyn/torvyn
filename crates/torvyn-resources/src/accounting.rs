//! Copy accounting for observability and benchmarking.
//!
//! Per Doc 03, Section 8: every copy is instrumented. Per-flow summaries
//! are maintained on the hot path. Individual TransferRecords are recorded
//! at Diagnostic level via the EventSink.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use torvyn_types::{ComponentId, CopyReason, EventSink, FlowId, ResourceId};

/// Per-flow copy statistics, maintained on the hot path.
///
/// Per Doc 03, Section 8.2.
///
/// # Thread Safety
/// Each field is an `AtomicU64` so stats can be updated from the resource
/// manager without requiring a mutable reference to the aggregator.
pub struct FlowCopyStats {
    /// Total payload bytes copied in this flow.
    pub total_payload_bytes: AtomicU64,
    /// Total metadata bytes copied (canonical ABI marshaling).
    pub total_metadata_bytes: AtomicU64,
    /// Total number of copy operations.
    pub total_copy_ops: AtomicU64,
    /// Copies by reason: [HostToComponent, ComponentToHost, CrossComponent, PoolReturn].
    pub copies_by_reason: [AtomicU64; 4],
}

impl FlowCopyStats {
    /// Create a new zeroed stats instance.
    pub fn new() -> Self {
        Self {
            total_payload_bytes: AtomicU64::new(0),
            total_metadata_bytes: AtomicU64::new(0),
            total_copy_ops: AtomicU64::new(0),
            copies_by_reason: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
        }
    }

    /// Record a copy event.
    ///
    /// # HOT PATH — must be allocation-free.
    #[inline]
    pub fn record(&self, byte_count: u64, reason: CopyReason) {
        self.total_payload_bytes
            .fetch_add(byte_count, Ordering::Relaxed);
        self.total_copy_ops.fetch_add(1, Ordering::Relaxed);
        let idx = reason_index(reason);
        self.copies_by_reason[idx].fetch_add(1, Ordering::Relaxed);
    }

    /// Record metadata copy bytes separately.
    ///
    /// # HOT PATH
    #[inline]
    pub fn record_metadata(&self, byte_count: u64) {
        self.total_metadata_bytes
            .fetch_add(byte_count, Ordering::Relaxed);
    }

    /// Take a snapshot of current stats (for reporting).
    pub fn snapshot(&self) -> FlowCopyStatsSnapshot {
        FlowCopyStatsSnapshot {
            total_payload_bytes: self.total_payload_bytes.load(Ordering::Relaxed),
            total_metadata_bytes: self.total_metadata_bytes.load(Ordering::Relaxed),
            total_copy_ops: self.total_copy_ops.load(Ordering::Relaxed),
            copies_by_reason: [
                self.copies_by_reason[0].load(Ordering::Relaxed),
                self.copies_by_reason[1].load(Ordering::Relaxed),
                self.copies_by_reason[2].load(Ordering::Relaxed),
                self.copies_by_reason[3].load(Ordering::Relaxed),
            ],
        }
    }
}

impl Default for FlowCopyStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable snapshot of copy stats for reporting.
#[derive(Clone, Debug, Default)]
pub struct FlowCopyStatsSnapshot {
    /// Total payload bytes copied in this flow.
    pub total_payload_bytes: u64,
    /// Total metadata bytes copied.
    pub total_metadata_bytes: u64,
    /// Total number of copy operations.
    pub total_copy_ops: u64,
    /// Copies by reason: [HostToComponent, ComponentToHost, CrossComponent, PoolReturn].
    pub copies_by_reason: [u64; 4],
}

/// Map CopyReason to array index.
#[inline]
fn reason_index(reason: CopyReason) -> usize {
    match reason {
        CopyReason::HostToComponent => 0,
        CopyReason::ComponentToHost => 1,
        CopyReason::CrossComponent => 2,
        CopyReason::PoolReturn => 3,
    }
}

/// The copy ledger: manages per-flow copy stats.
///
/// Thread-safe via internal HashMap behind a parking_lot::Mutex.
pub struct CopyLedger {
    flows: parking_lot::Mutex<HashMap<FlowId, FlowCopyStats>>,
}

impl CopyLedger {
    /// Create a new empty ledger.
    pub fn new() -> Self {
        Self {
            flows: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// Ensure a flow has an entry in the ledger.
    ///
    /// # COLD PATH — called once per flow creation.
    pub fn register_flow(&self, flow_id: FlowId) {
        let mut flows = self.flows.lock();
        flows.entry(flow_id).or_default();
    }

    /// Record a copy event for a flow.
    ///
    /// # HOT PATH — but the lock is held only briefly for HashMap lookup.
    /// In a future optimization, we could use a lock-free map or pre-register
    /// flow-specific stats objects to avoid the lock entirely.
    #[allow(clippy::too_many_arguments)]
    pub fn record_copy(
        &self,
        flow_id: FlowId,
        resource_id: ResourceId,
        from: ComponentId,
        to: ComponentId,
        byte_count: u64,
        reason: CopyReason,
        event_sink: &dyn EventSink,
    ) {
        {
            let flows = self.flows.lock();
            if let Some(stats) = flows.get(&flow_id) {
                stats.record(byte_count, reason);
            }
        }

        // Emit to observability
        event_sink.record_copy(flow_id, resource_id, from, to, byte_count, reason);
    }

    /// Get a snapshot of copy stats for a flow.
    ///
    /// # COLD PATH — called for reporting.
    pub fn flow_stats(&self, flow_id: FlowId) -> FlowCopyStatsSnapshot {
        let flows = self.flows.lock();
        flows
            .get(&flow_id)
            .map(|s| s.snapshot())
            .unwrap_or_default()
    }

    /// Remove a flow from the ledger (flow completed).
    ///
    /// # COLD PATH
    pub fn remove_flow(&self, flow_id: FlowId) -> FlowCopyStatsSnapshot {
        let mut flows = self.flows.lock();
        flows
            .remove(&flow_id)
            .map(|s| s.snapshot())
            .unwrap_or_default()
    }
}

impl Default for CopyLedger {
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
    fn test_flow_copy_stats_record() {
        let stats = FlowCopyStats::new();
        stats.record(1024, CopyReason::HostToComponent);
        stats.record(512, CopyReason::ComponentToHost);

        let snap = stats.snapshot();
        assert_eq!(snap.total_payload_bytes, 1536);
        assert_eq!(snap.total_copy_ops, 2);
        assert_eq!(snap.copies_by_reason[0], 1); // HostToComponent
        assert_eq!(snap.copies_by_reason[1], 1); // ComponentToHost
    }

    #[test]
    fn test_flow_copy_stats_metadata() {
        let stats = FlowCopyStats::new();
        stats.record_metadata(48);
        let snap = stats.snapshot();
        assert_eq!(snap.total_metadata_bytes, 48);
    }

    #[test]
    fn test_copy_ledger_register_and_record() {
        let ledger = CopyLedger::new();
        let flow = FlowId::new(1);
        let sink = torvyn_types::NoopEventSink;

        ledger.register_flow(flow);
        ledger.record_copy(
            flow,
            ResourceId::new(0, 1),
            ComponentId::new(1),
            ComponentId::new(2),
            256,
            CopyReason::HostToComponent,
            &sink,
        );

        let snap = ledger.flow_stats(flow);
        assert_eq!(snap.total_payload_bytes, 256);
        assert_eq!(snap.total_copy_ops, 1);
    }

    #[test]
    fn test_copy_ledger_remove_flow() {
        let ledger = CopyLedger::new();
        let flow = FlowId::new(1);
        let sink = torvyn_types::NoopEventSink;

        ledger.register_flow(flow);
        ledger.record_copy(
            flow,
            ResourceId::new(0, 1),
            ComponentId::new(1),
            ComponentId::new(2),
            100,
            CopyReason::HostToComponent,
            &sink,
        );

        let snap = ledger.remove_flow(flow);
        assert_eq!(snap.total_payload_bytes, 100);

        // After removal, stats should be empty
        let snap2 = ledger.flow_stats(flow);
        assert_eq!(snap2.total_payload_bytes, 0);
    }

    #[test]
    fn test_copy_ledger_unregistered_flow_is_noop() {
        let ledger = CopyLedger::new();
        let sink = torvyn_types::NoopEventSink;

        // Recording to an unregistered flow should not panic
        ledger.record_copy(
            FlowId::new(999),
            ResourceId::new(0, 0),
            ComponentId::new(1),
            ComponentId::new(2),
            100,
            CopyReason::HostToComponent,
            &sink,
        );

        let snap = ledger.flow_stats(FlowId::new(999));
        assert_eq!(snap.total_copy_ops, 0);
    }
}
