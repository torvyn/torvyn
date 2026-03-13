//! Resource pool metric structures.
//!
//! Per HLI Doc 05 §3.3.4. PoolId is defined locally because it is not
//! in `torvyn-types` and the resource crate does not export it as a
//! standalone type.
//!
//! HLI DEVIATION: Doc 05 §3.2 references `PoolId` as if it were in
//! `torvyn-types`. Since no LLI session defined `PoolId`, we define it
//! here as a simple u32 newtype. If `torvyn-resources` later exports
//! `PoolId`, this should be replaced.

use super::{Counter, Gauge, Histogram, SIZE_BUCKETS_BYTES};
use serde::{Deserialize, Serialize};

/// Identifier for a buffer pool tier.
///
/// Maps to the pool tiers in `torvyn-resources` (Small=0, Medium=1,
/// Large=2, Huge=3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PoolId(pub u32);

impl PoolId {
    /// Create a new pool identifier.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw pool identifier value.
    #[inline]
    pub const fn raw(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for PoolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pool-{}", self.0)
    }
}

/// Pre-allocated metric storage for a resource buffer pool.
///
/// Per HLI Doc 05 §3.3.4.
pub struct ResourcePoolMetrics {
    /// Pool tier identifier.
    pub pool_id: PoolId,
    /// Total buffer allocations.
    pub allocations: Counter,
    /// Total buffers returned to pool.
    pub deallocations: Counter,
    /// Allocations satisfied from recycled buffers.
    pub reuses: Counter,
    /// Allocation attempts when pool was empty.
    pub exhaustion_events: Counter,
    /// Current pool utilization (fixed-point: value x 1000).
    pub utilization_permille: Gauge,
    /// Buffers currently checked out.
    pub active_buffers: Gauge,
    /// Size distribution of allocated buffers.
    pub allocation_size: Histogram,
}

impl ResourcePoolMetrics {
    /// # COLD PATH
    pub fn new(pool_id: PoolId) -> Self {
        Self {
            pool_id,
            allocations: Counter::new(),
            deallocations: Counter::new(),
            reuses: Counter::new(),
            exhaustion_events: Counter::new(),
            utilization_permille: Gauge::new(),
            active_buffers: Gauge::new(),
            allocation_size: Histogram::new(SIZE_BUCKETS_BYTES),
        }
    }

    /// Compute utilization as f64 ratio [0.0, 1.0].
    pub fn utilization(&self) -> f64 {
        self.utilization_permille.read() as f64 / 1000.0
    }
}

impl std::fmt::Debug for ResourcePoolMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourcePoolMetrics")
            .field("pool_id", &self.pool_id)
            .field("allocations", &self.allocations.read())
            .field("active_buffers", &self.active_buffers.read())
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
    fn test_pool_metrics_new() {
        let pm = ResourcePoolMetrics::new(PoolId::new(0));
        assert_eq!(pm.allocations.read(), 0);
        assert_eq!(pm.active_buffers.read(), 0);
    }

    #[test]
    fn test_pool_metrics_utilization() {
        let pm = ResourcePoolMetrics::new(PoolId::new(0));
        pm.utilization_permille.set(500);
        assert!((pm.utilization() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_pool_id_display() {
        assert_eq!(format!("{}", PoolId::new(3)), "pool-3");
    }
}
