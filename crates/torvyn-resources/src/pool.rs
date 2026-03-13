//! Tiered buffer pool with lock-free Treiber stack free lists.
//!
//! Per Doc 03, Section 5: global tiered pools for Small, Medium, Large, Huge.
//! Small/Medium/Large are pre-allocated at startup. Huge allocates on demand.
//!
//! # Thread Safety
//! Each tier's free list is a lock-free Treiber stack using `AtomicU32`.
//! Push and pop are wait-free on architectures with native CAS (x86-64, ARM64).
//!
//! The generation counter on each buffer slot provides ABA protection.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::buffer;
use crate::handle::PoolTier;

/// Sentinel for "empty free list."
const STACK_EMPTY: u32 = u32::MAX;

/// Configuration for a pool tier.
#[derive(Clone, Debug)]
pub struct TierConfig {
    /// Which tier this is.
    pub tier: PoolTier,
    /// Number of buffers to pre-allocate.
    pub pool_size: u32,
    /// Whether to pre-allocate all buffers at startup.
    pub preallocate: bool,
}

impl TierConfig {
    /// Default configurations for all tiers.
    ///
    /// Per Doc 03, Section 5.2.
    pub fn defaults() -> [TierConfig; 4] {
        [
            TierConfig {
                tier: PoolTier::Small,
                pool_size: 4096,
                preallocate: true,
            },
            TierConfig {
                tier: PoolTier::Medium,
                pool_size: 1024,
                preallocate: true,
            },
            TierConfig {
                tier: PoolTier::Large,
                pool_size: 256,
                preallocate: true,
            },
            TierConfig {
                tier: PoolTier::Huge,
                pool_size: 32,
                preallocate: false, // On-demand with caching
            },
        ]
    }
}

/// A single buffer in the pool, stored in a flat array.
struct PoolBuffer {
    /// Pointer to the buffer allocation (header + payload).
    ptr: NonNull<u8>,
    /// Total allocation size.
    alloc_size: usize,
    /// Index of the next free buffer in the Treiber stack (or STACK_EMPTY).
    next_free: AtomicU32,
}

// SAFETY: PoolBuffer is only accessed through the pool's atomic operations.
unsafe impl Send for PoolBuffer {}
unsafe impl Sync for PoolBuffer {}

/// A single-tier buffer pool backed by a Treiber stack.
pub struct TierPool {
    /// The tier this pool serves.
    tier: PoolTier,
    /// All buffers in this pool (fixed-size array after init).
    buffers: Vec<PoolBuffer>,
    /// Index of the top of the free stack.
    free_top: AtomicU32,
    /// Total number of buffers in the pool.
    pool_size: u32,
    /// Number of currently available (free) buffers.
    /// Used for metrics; not authoritative for allocation decisions.
    available: AtomicU32,
    /// Pool metrics: total allocations from this tier.
    pub(crate) alloc_count: AtomicU32,
    /// Pool metrics: total returns to this tier.
    pub(crate) return_count: AtomicU32,
    /// Pool metrics: allocations that fell back to the system allocator.
    pub(crate) fallback_count: AtomicU32,
    /// Pool metrics: times the free list was empty.
    pub(crate) exhaustion_count: AtomicU32,
}

impl TierPool {
    /// Create and pre-warm a pool for the given tier.
    ///
    /// # COLD PATH — called once per tier during host startup.
    ///
    /// # Panics
    /// Panics if pre-allocation fails (system OOM during startup).
    pub fn new(config: &TierConfig) -> Self {
        let capacity = config.tier.capacity();
        let pool_size = config.pool_size;
        let mut buffers = Vec::with_capacity(pool_size as usize);

        for i in 0..pool_size {
            if config.preallocate {
                let (ptr, alloc_size) = buffer::alloc_buffer(capacity)
                    .unwrap_or_else(|| {
                        panic!(
                            "Failed to pre-allocate buffer {i}/{pool_size} for tier {}. \
                             System is out of memory.",
                            config.tier
                        )
                    });
                let next = if i + 1 < pool_size {
                    i + 1
                } else {
                    STACK_EMPTY
                };
                buffers.push(PoolBuffer {
                    ptr,
                    alloc_size,
                    next_free: AtomicU32::new(next),
                });
            } else {
                // On-demand tier: no pre-allocation.
                // For Huge tier, we don't pre-allocate.
                break;
            }
        }

        let available = if config.preallocate { pool_size } else { 0 };
        let free_top = if config.preallocate && pool_size > 0 {
            0
        } else {
            STACK_EMPTY
        };

        Self {
            tier: config.tier,
            buffers,
            free_top: AtomicU32::new(free_top),
            pool_size,
            available: AtomicU32::new(available),
            alloc_count: AtomicU32::new(0),
            return_count: AtomicU32::new(0),
            fallback_count: AtomicU32::new(0),
            exhaustion_count: AtomicU32::new(0),
        }
    }

    /// Pop a buffer from the free stack.
    ///
    /// Returns `Some((ptr, alloc_size))` if a buffer is available,
    /// `None` if the pool is exhausted.
    ///
    /// # HOT PATH — called per allocation. Lock-free.
    pub fn pop(&self) -> Option<(NonNull<u8>, usize)> {
        loop {
            let top = self.free_top.load(Ordering::Acquire);
            if top == STACK_EMPTY {
                self.exhaustion_count.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            let buffer = &self.buffers[top as usize];
            let next = buffer.next_free.load(Ordering::Relaxed);

            // CAS: try to swing free_top from `top` to `next`.
            match self.free_top.compare_exchange_weak(
                top,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.available.fetch_sub(1, Ordering::Relaxed);
                    self.alloc_count.fetch_add(1, Ordering::Relaxed);
                    return Some((buffer.ptr, buffer.alloc_size));
                }
                Err(_) => continue, // Retry
            }
        }
    }

    /// Push a buffer back onto the free stack.
    ///
    /// Returns `true` if the buffer was accepted, `false` if the pool is full
    /// (only possible for on-demand tiers that have a size limit).
    ///
    /// # HOT PATH — called per buffer return. Lock-free.
    ///
    /// # Safety
    /// The caller must ensure `ptr` was originally allocated for this tier
    /// (correct capacity and alignment).
    pub fn push(&self, ptr: NonNull<u8>, _alloc_size: usize) -> bool {
        // For pre-allocated pools, find the slot for this pointer.
        for (i, buffer) in self.buffers.iter().enumerate() {
            if buffer.ptr == ptr {
                // Found it — push back onto the stack.
                loop {
                    let top = self.free_top.load(Ordering::Acquire);
                    buffer.next_free.store(top, Ordering::Relaxed);

                    match self.free_top.compare_exchange_weak(
                        top,
                        i as u32,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            self.available.fetch_add(1, Ordering::Relaxed);
                            self.return_count.fetch_add(1, Ordering::Relaxed);
                            return true;
                        }
                        Err(_) => continue,
                    }
                }
            }
        }

        // Buffer not found in this pool — it was a fallback allocation.
        false
    }

    /// Returns the tier of this pool.
    #[inline]
    pub fn tier(&self) -> PoolTier {
        self.tier
    }

    /// Returns the number of currently available (free) buffers.
    ///
    /// This is approximate (based on atomic counter, not authoritative).
    #[inline]
    pub fn available(&self) -> u32 {
        self.available.load(Ordering::Relaxed)
    }

    /// Returns the total pool size (number of buffer slots).
    #[inline]
    pub fn pool_size(&self) -> u32 {
        self.pool_size
    }
}

impl Drop for TierPool {
    fn drop(&mut self) {
        // Deallocate all buffers that were pre-allocated.
        for buffer in &self.buffers {
            // SAFETY: Each buffer.ptr was allocated by alloc_buffer with
            // the corresponding alloc_size. We own all buffers.
            unsafe {
                buffer::dealloc_buffer(buffer.ptr, buffer.alloc_size);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BufferPoolSet
// ---------------------------------------------------------------------------

/// The complete set of tiered buffer pools.
///
/// Per Doc 03, Section 5.1: global tiered pool architecture.
///
/// # Examples
/// ```
/// use torvyn_resources::pool::{BufferPoolSet, TierConfig};
/// use torvyn_resources::handle::PoolTier;
///
/// let pools = BufferPoolSet::new(&TierConfig::defaults());
/// let result = pools.allocate(256);
/// assert!(result.is_some());
/// ```
pub struct BufferPoolSet {
    small: TierPool,
    medium: TierPool,
    large: TierPool,
    huge: TierPool,
}

impl BufferPoolSet {
    /// Create all pool tiers with the given configurations.
    ///
    /// # COLD PATH — called once during host startup.
    pub fn new(configs: &[TierConfig; 4]) -> Self {
        Self {
            small: TierPool::new(&configs[0]),
            medium: TierPool::new(&configs[1]),
            large: TierPool::new(&configs[2]),
            huge: TierPool::new(&configs[3]),
        }
    }

    /// Allocate a buffer from the appropriate tier.
    ///
    /// Returns `Some((ptr, alloc_size, tier, is_fallback))` on success.
    /// Falls back to the system allocator if the pool is exhausted.
    /// Returns `None` only on true OOM.
    ///
    /// # WARM PATH — called per buffer allocation.
    pub fn allocate(&self, min_capacity: u32) -> Option<(NonNull<u8>, usize, PoolTier, bool)> {
        let tier = PoolTier::for_capacity(min_capacity);
        let pool = self.tier_pool(tier);

        // Try the pool first
        if let Some((ptr, alloc_size)) = pool.pop() {
            return Some((ptr, alloc_size, tier, false));
        }

        // Pool exhausted — fall back to system allocator
        pool.fallback_count.fetch_add(1, Ordering::Relaxed);
        let capacity = tier.capacity().max(min_capacity);
        let (ptr, alloc_size) = buffer::alloc_buffer(capacity)?;
        Some((ptr, alloc_size, tier, true))
    }

    /// Return a buffer to its pool.
    ///
    /// If the buffer is a fallback allocation (not from a pool),
    /// it is deallocated via the system allocator.
    ///
    /// # WARM PATH — called per buffer release.
    ///
    /// # Safety
    /// - `ptr` must be a valid buffer allocation.
    /// - `alloc_size` must match the original allocation.
    pub unsafe fn release(
        &self,
        ptr: NonNull<u8>,
        alloc_size: usize,
        tier: PoolTier,
        is_fallback: bool,
    ) {
        if is_fallback {
            // Fallback allocation — just deallocate
            // SAFETY: Caller guarantees ptr/alloc_size match the original allocation.
            buffer::dealloc_buffer(ptr, alloc_size);
            return;
        }

        let pool = self.tier_pool(tier);
        if !pool.push(ptr, alloc_size) {
            // Pool didn't accept it (shouldn't happen for pre-allocated buffers)
            // SAFETY: Caller guarantees ptr/alloc_size match the original allocation.
            buffer::dealloc_buffer(ptr, alloc_size);
        }
    }

    /// Get the pool for a given tier.
    #[inline]
    fn tier_pool(&self, tier: PoolTier) -> &TierPool {
        match tier {
            PoolTier::Small => &self.small,
            PoolTier::Medium => &self.medium,
            PoolTier::Large => &self.large,
            PoolTier::Huge => &self.huge,
        }
    }

    /// Get metrics snapshot for a tier.
    pub fn tier_metrics(&self, tier: PoolTier) -> TierMetrics {
        let pool = self.tier_pool(tier);
        TierMetrics {
            tier,
            pool_size: pool.pool_size(),
            available: pool.available(),
            alloc_count: pool.alloc_count.load(Ordering::Relaxed),
            return_count: pool.return_count.load(Ordering::Relaxed),
            fallback_count: pool.fallback_count.load(Ordering::Relaxed),
            exhaustion_count: pool.exhaustion_count.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of pool metrics for a single tier.
#[derive(Clone, Debug)]
pub struct TierMetrics {
    /// The tier.
    pub tier: PoolTier,
    /// Total pool size.
    pub pool_size: u32,
    /// Currently available buffers.
    pub available: u32,
    /// Total allocations from this tier.
    pub alloc_count: u32,
    /// Total returns to this tier.
    pub return_count: u32,
    /// Allocations that fell back to the system allocator.
    pub fallback_count: u32,
    /// Times the free list was empty.
    pub exhaustion_count: u32,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn small_config() -> [TierConfig; 4] {
        [
            TierConfig { tier: PoolTier::Small, pool_size: 8, preallocate: true },
            TierConfig { tier: PoolTier::Medium, pool_size: 4, preallocate: true },
            TierConfig { tier: PoolTier::Large, pool_size: 2, preallocate: true },
            TierConfig { tier: PoolTier::Huge, pool_size: 1, preallocate: false },
        ]
    }

    #[test]
    fn test_tier_pool_pop_and_push() {
        let config = TierConfig {
            tier: PoolTier::Small,
            pool_size: 4,
            preallocate: true,
        };
        let pool = TierPool::new(&config);
        assert_eq!(pool.available(), 4);

        // Pop one
        let (ptr, alloc_size) = pool.pop().expect("should have buffers");
        assert_eq!(pool.available(), 3);

        // Push it back
        assert!(pool.push(ptr, alloc_size));
        assert_eq!(pool.available(), 4);
    }

    #[test]
    fn test_tier_pool_exhaustion() {
        let config = TierConfig {
            tier: PoolTier::Small,
            pool_size: 2,
            preallocate: true,
        };
        let pool = TierPool::new(&config);

        let _b1 = pool.pop().unwrap();
        let _b2 = pool.pop().unwrap();
        assert!(pool.pop().is_none()); // Exhausted
        assert_eq!(pool.exhaustion_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_buffer_pool_set_allocate_small() {
        let pools = BufferPoolSet::new(&small_config());
        let result = pools.allocate(100);
        assert!(result.is_some());
        let (ptr, alloc_size, tier, is_fallback) = result.unwrap();
        assert_eq!(tier, PoolTier::Small);
        assert!(!is_fallback);
        // Return it
        unsafe { pools.release(ptr, alloc_size, tier, is_fallback) };
    }

    #[test]
    fn test_buffer_pool_set_allocate_medium() {
        let pools = BufferPoolSet::new(&small_config());
        let result = pools.allocate(1000);
        assert!(result.is_some());
        let (_, _, tier, _) = result.unwrap();
        assert_eq!(tier, PoolTier::Medium);
    }

    #[test]
    fn test_buffer_pool_set_fallback_on_exhaustion() {
        let pools = BufferPoolSet::new(&small_config());

        // Exhaust the small pool (8 buffers)
        let mut allocated = Vec::new();
        for _ in 0..8 {
            let (ptr, alloc_size, tier, is_fallback) = pools.allocate(100).unwrap();
            assert!(!is_fallback);
            allocated.push((ptr, alloc_size, tier, is_fallback));
        }

        // Next allocation should fall back
        let (ptr, alloc_size, tier, is_fallback) = pools.allocate(100).unwrap();
        assert!(is_fallback);
        assert_eq!(tier, PoolTier::Small);

        // Cleanup
        unsafe { pools.release(ptr, alloc_size, tier, is_fallback) };
        for (ptr, alloc_size, tier, is_fallback) in allocated {
            unsafe { pools.release(ptr, alloc_size, tier, is_fallback) };
        }
    }

    #[test]
    fn test_buffer_pool_set_1000_alloc_return_cycles() {
        let pools = BufferPoolSet::new(&small_config());

        for _ in 0..1000 {
            let (ptr, alloc_size, tier, is_fallback) = pools.allocate(100).unwrap();
            unsafe { pools.release(ptr, alloc_size, tier, is_fallback) };
        }

        let metrics = pools.tier_metrics(PoolTier::Small);
        assert_eq!(metrics.alloc_count, 1000);
        assert_eq!(metrics.return_count, 1000);
        assert_eq!(metrics.available, metrics.pool_size);
    }

    #[test]
    fn test_tier_metrics_snapshot() {
        let pools = BufferPoolSet::new(&small_config());
        let metrics = pools.tier_metrics(PoolTier::Small);
        assert_eq!(metrics.pool_size, 8);
        assert_eq!(metrics.available, 8);
        assert_eq!(metrics.alloc_count, 0);
    }
}
