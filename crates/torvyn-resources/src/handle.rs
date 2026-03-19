//! Resource ownership identity and resource table entry types.
//!
//! [`OwnerId`] identifies who owns or is interacting with a resource.
//! [`ResourceEntry`] is the per-slot data in the resource table.
//! `Slot` is the enum that distinguishes occupied from vacant slots.

use std::fmt;
use torvyn_types::{ComponentId, FlowId, ResourceId, ResourceState};

// ---------------------------------------------------------------------------
// OwnerId
// ---------------------------------------------------------------------------

/// Identifies the entity that owns a resource.
///
/// Per Doc 03 §12.1: three variants — Host, Component, and Transit.
///
/// # Invariants
/// - Exactly one `OwnerId` is associated with each active resource at any time.
/// - `Transit` is a temporary state during cross-component transfer.
///
/// # Examples
/// ```
/// use torvyn_resources::OwnerId;
/// use torvyn_types::ComponentId;
///
/// let owner = OwnerId::Component(ComponentId::new(1));
/// assert!(!owner.is_host());
/// assert!(owner.component_id().is_some());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OwnerId {
    /// The host runtime owns this resource.
    Host,
    /// A specific component instance owns this resource.
    Component(ComponentId),
    /// The resource is temporarily held by the host during cross-component transfer.
    Transit,
}

impl OwnerId {
    /// Returns `true` if this is the host.
    ///
    /// # HOT PATH
    #[inline]
    pub const fn is_host(&self) -> bool {
        matches!(self, OwnerId::Host)
    }

    /// Returns `true` if this is a component.
    ///
    /// # HOT PATH
    #[inline]
    pub const fn is_component(&self) -> bool {
        matches!(self, OwnerId::Component(_))
    }

    /// Returns `true` if this is in transit.
    ///
    /// # HOT PATH
    #[inline]
    pub const fn is_transit(&self) -> bool {
        matches!(self, OwnerId::Transit)
    }

    /// Returns the component ID if this is a component owner.
    ///
    /// # HOT PATH
    #[inline]
    pub const fn component_id(&self) -> Option<ComponentId> {
        match self {
            OwnerId::Component(id) => Some(*id),
            _ => None,
        }
    }
}

impl fmt::Display for OwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnerId::Host => write!(f, "host"),
            OwnerId::Component(id) => write!(f, "{id}"),
            OwnerId::Transit => write!(f, "transit"),
        }
    }
}

// ---------------------------------------------------------------------------
// PoolTier
// ---------------------------------------------------------------------------

/// Pool tier classification.
///
/// Per Doc 03, Section 4.3: four tiers by payload capacity.
///
/// # Examples
/// ```
/// use torvyn_resources::PoolTier;
///
/// let tier = PoolTier::for_capacity(1024);
/// assert_eq!(tier, PoolTier::Medium);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PoolTier {
    /// 256-byte payload. Metadata, small messages, control signals.
    Small,
    /// 4 KiB payload. Typical stream elements, JSON documents.
    Medium,
    /// 64 KiB payload. Binary payloads, image chunks, audio frames.
    Large,
    /// 1 MiB payload. Large transfers, batch payloads.
    Huge,
}

impl PoolTier {
    /// The payload capacity for this tier in bytes.
    ///
    /// # COLD PATH — called during pool setup.
    #[inline]
    pub const fn capacity(&self) -> u32 {
        match self {
            PoolTier::Small => 256,
            PoolTier::Medium => 4 * 1024,
            PoolTier::Large => 64 * 1024,
            PoolTier::Huge => 1024 * 1024,
        }
    }

    /// Select the smallest tier that can hold `requested` bytes.
    ///
    /// Returns `Huge` if the requested size exceeds the Large tier.
    /// Buffers larger than 1 MiB up to the global max (16 MiB) are allocated
    /// as on-demand (non-pooled) buffers.
    ///
    /// # WARM PATH — called per allocation.
    ///
    /// # Examples
    /// ```
    /// use torvyn_resources::PoolTier;
    ///
    /// assert_eq!(PoolTier::for_capacity(100), PoolTier::Small);
    /// assert_eq!(PoolTier::for_capacity(257), PoolTier::Medium);
    /// assert_eq!(PoolTier::for_capacity(5000), PoolTier::Large);
    /// assert_eq!(PoolTier::for_capacity(100_000), PoolTier::Huge);
    /// ```
    #[inline]
    pub fn for_capacity(requested: u32) -> Self {
        if requested <= Self::Small.capacity() {
            PoolTier::Small
        } else if requested <= Self::Medium.capacity() {
            PoolTier::Medium
        } else if requested <= Self::Large.capacity() {
            PoolTier::Large
        } else {
            PoolTier::Huge
        }
    }

    /// Returns all tiers in order from smallest to largest.
    pub const ALL: [PoolTier; 4] = [
        PoolTier::Small,
        PoolTier::Medium,
        PoolTier::Large,
        PoolTier::Huge,
    ];
}

impl fmt::Display for PoolTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolTier::Small => write!(f, "Small(256B)"),
            PoolTier::Medium => write!(f, "Medium(4KiB)"),
            PoolTier::Large => write!(f, "Large(64KiB)"),
            PoolTier::Huge => write!(f, "Huge(1MiB)"),
        }
    }
}

// ---------------------------------------------------------------------------
// BufferFlags
// ---------------------------------------------------------------------------

/// Flags on a buffer.
///
/// Per Doc 03, Section 12.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BufferFlags {
    /// This buffer was allocated from the system allocator, not a pool.
    pub is_fallback: bool,
    /// This buffer's payload is read-only (reserved for future use).
    pub read_only: bool,
}

// ---------------------------------------------------------------------------
// ResourceEntry
// ---------------------------------------------------------------------------

/// A live resource in the resource table.
///
/// Per Doc 03, Section 9.1. Each occupied slot in the slab holds one of these.
///
/// # Invariants
/// - `generation` matches the generation of the `ResourceId` that refers to this slot.
/// - `state` accurately reflects the ownership state machine.
/// - `borrow_count` is zero when `state` is not `Borrowed`.
/// - `owner` is `Transit` only when `state` is `Transit`.
/// - `buffer_ptr` is non-null and points to a valid, aligned buffer allocation.
#[derive(Debug)]
pub struct ResourceEntry {
    /// Generation counter. Incremented each time this slot is reused.
    pub generation: u32,
    /// Current ownership state.
    pub state: ResourceState,
    /// Current owner of the resource.
    pub owner: OwnerId,
    /// Number of outstanding borrows. Zero unless state is Borrowed.
    pub borrow_count: u32,
    /// Pointer to the buffer allocation (header + payload).
    /// This is raw because the buffer is allocated via the system allocator
    /// and managed manually.
    pub buffer_ptr: std::ptr::NonNull<u8>,
    /// Total allocation size in bytes (header + payload capacity).
    pub alloc_size: usize,
    /// Payload capacity in bytes (the usable payload area).
    pub payload_capacity: u32,
    /// Current valid payload length (how many bytes are written).
    pub payload_len: u32,
    /// Which pool tier this buffer belongs to.
    pub pool_tier: PoolTier,
    /// Buffer flags.
    pub flags: BufferFlags,
    /// Content type tag (short string, stored inline).
    /// Empty string means unset.
    pub content_type: ContentType,
    /// The flow this resource was allocated for. Used for copy accounting.
    pub flow_id: FlowId,
    /// Timestamp when this resource was allocated (nanoseconds since epoch).
    pub created_at_ns: u64,
}

// ResourceEntry cannot be Send/Sync automatically because of NonNull.
// We assert it is safe because:
// - The resource table ensures exclusive access to entries (single-threaded within flow).
// - The raw pointer is to a host-owned allocation that outlives any borrow.
// SAFETY: ResourceEntry is accessed only under the ResourceTable's synchronization.
unsafe impl Send for ResourceEntry {}

/// Inline content type storage to avoid heap allocation on the hot path.
///
/// Stores up to 63 bytes of ASCII content type (e.g., "application/json").
/// This avoids a `String` allocation per buffer on the hot path.
///
/// # Invariants
/// - `len` <= 63.
/// - `data[0..len]` is valid UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentType {
    data: [u8; 63],
    len: u8,
}

impl ContentType {
    /// Maximum inline content type length.
    pub const MAX_LEN: usize = 63;

    /// Create an empty content type.
    ///
    /// # HOT PATH
    #[inline]
    pub const fn empty() -> Self {
        Self {
            data: [0u8; 63],
            len: 0,
        }
    }

    /// Create a content type from a string slice. Truncates if > 63 bytes.
    ///
    /// # WARM PATH — called per allocation.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(Self::MAX_LEN);
        let mut data = [0u8; 63];
        data[..copy_len].copy_from_slice(&bytes[..copy_len]);
        Self {
            data,
            len: copy_len as u8,
        }
    }

    /// Returns the content type as a string slice.
    ///
    /// # HOT PATH
    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: We only store valid UTF-8 bytes (from_str validates input).
        // The `from_str` constructor copies from a `&str`, which is guaranteed
        // to be valid UTF-8.
        unsafe { std::str::from_utf8_unchecked(&self.data[..self.len as usize]) }
    }

    /// Returns `true` if the content type is empty (unset).
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the length of the content type string.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len as usize
    }
}

impl Default for ContentType {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Slot
// ---------------------------------------------------------------------------

/// A slot in the resource table slab: either occupied or vacant.
///
/// Per Doc 03, Section 9.1. Vacant slots form a free list.
pub enum Slot {
    /// The slot contains a live resource.
    Occupied(ResourceEntry),
    /// The slot is vacant. `next_free` is the index of the next vacant slot
    /// (or `u32::MAX` if this is the tail of the free list).
    /// `generation` tracks how many times this slot has been reused.
    Vacant {
        /// Index of the next vacant slot in the free list.
        next_free: u32,
        /// Generation counter for this slot.
        generation: u32,
    },
}

impl Slot {
    /// Returns `true` if this slot is occupied.
    #[inline]
    pub fn is_occupied(&self) -> bool {
        matches!(self, Slot::Occupied(_))
    }

    /// Returns a reference to the entry if occupied.
    #[inline]
    pub fn as_entry(&self) -> Option<&ResourceEntry> {
        match self {
            Slot::Occupied(entry) => Some(entry),
            Slot::Vacant { .. } => None,
        }
    }

    /// Returns a mutable reference to the entry if occupied.
    #[inline]
    pub fn as_entry_mut(&mut self) -> Option<&mut ResourceEntry> {
        match self {
            Slot::Occupied(entry) => Some(entry),
            Slot::Vacant { .. } => None,
        }
    }

    /// Returns the generation of this slot, regardless of occupied/vacant.
    #[inline]
    pub fn generation(&self) -> u32 {
        match self {
            Slot::Occupied(entry) => entry.generation,
            Slot::Vacant { generation, .. } => *generation,
        }
    }
}

// ---------------------------------------------------------------------------
// ResourceReclaimed — returned from force_reclaim
// ---------------------------------------------------------------------------

/// Information about a resource that was forcefully reclaimed.
///
/// Returned by `force_reclaim` for diagnostic reporting.
#[derive(Clone, Debug)]
pub struct ResourceReclaimed {
    /// The resource ID that was reclaimed.
    pub resource_id: ResourceId,
    /// The state the resource was in when reclaimed.
    pub previous_state: ResourceState,
    /// The previous owner.
    pub previous_owner: OwnerId,
    /// The payload capacity of the buffer.
    pub payload_capacity: u32,
    /// Whether the buffer was returned to the pool or deallocated.
    pub returned_to_pool: bool,
}

// ---------------------------------------------------------------------------
// FlowResourceStats — returned from release_flow_resources
// ---------------------------------------------------------------------------

/// Summary of resources released when a flow completes.
///
/// Per C03-5: returned by `release_flow_resources`.
#[derive(Clone, Debug, Default)]
pub struct FlowResourceStats {
    /// Number of resources that were returned to pools.
    pub returned_to_pool: u32,
    /// Number of resources that were deallocated (fallback or oversized).
    pub deallocated: u32,
    /// Total bytes released.
    pub total_bytes_released: u64,
    /// Number of resources that had outstanding borrows at cleanup time.
    pub borrows_cleared: u32,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_owner_id_host() {
        let owner = OwnerId::Host;
        assert!(owner.is_host());
        assert!(!owner.is_component());
        assert!(!owner.is_transit());
        assert!(owner.component_id().is_none());
        assert_eq!(format!("{owner}"), "host");
    }

    #[test]
    fn test_owner_id_component() {
        let owner = OwnerId::Component(ComponentId::new(42));
        assert!(!owner.is_host());
        assert!(owner.is_component());
        assert_eq!(owner.component_id(), Some(ComponentId::new(42)));
        assert_eq!(format!("{owner}"), "component-42");
    }

    #[test]
    fn test_owner_id_transit() {
        let owner = OwnerId::Transit;
        assert!(owner.is_transit());
        assert!(owner.component_id().is_none());
        assert_eq!(format!("{owner}"), "transit");
    }

    #[test]
    fn test_pool_tier_capacities() {
        assert_eq!(PoolTier::Small.capacity(), 256);
        assert_eq!(PoolTier::Medium.capacity(), 4096);
        assert_eq!(PoolTier::Large.capacity(), 65536);
        assert_eq!(PoolTier::Huge.capacity(), 1048576);
    }

    #[test]
    fn test_pool_tier_for_capacity_small() {
        assert_eq!(PoolTier::for_capacity(0), PoolTier::Small);
        assert_eq!(PoolTier::for_capacity(1), PoolTier::Small);
        assert_eq!(PoolTier::for_capacity(256), PoolTier::Small);
    }

    #[test]
    fn test_pool_tier_for_capacity_medium() {
        assert_eq!(PoolTier::for_capacity(257), PoolTier::Medium);
        assert_eq!(PoolTier::for_capacity(4096), PoolTier::Medium);
    }

    #[test]
    fn test_pool_tier_for_capacity_large() {
        assert_eq!(PoolTier::for_capacity(4097), PoolTier::Large);
        assert_eq!(PoolTier::for_capacity(65536), PoolTier::Large);
    }

    #[test]
    fn test_pool_tier_for_capacity_huge() {
        assert_eq!(PoolTier::for_capacity(65537), PoolTier::Huge);
        assert_eq!(PoolTier::for_capacity(1048576), PoolTier::Huge);
    }

    #[test]
    fn test_pool_tier_for_capacity_beyond_huge() {
        // Anything > 1 MiB still returns Huge (on-demand allocation)
        assert_eq!(PoolTier::for_capacity(2_000_000), PoolTier::Huge);
    }

    #[test]
    fn test_content_type_empty() {
        let ct = ContentType::empty();
        assert!(ct.is_empty());
        assert_eq!(ct.len(), 0);
        assert_eq!(ct.as_str(), "");
    }

    #[test]
    fn test_content_type_from_str() {
        let ct = ContentType::from_str("application/json");
        assert!(!ct.is_empty());
        assert_eq!(ct.as_str(), "application/json");
        assert_eq!(ct.len(), 16);
    }

    #[test]
    fn test_content_type_truncation() {
        let long = "a".repeat(100);
        let ct = ContentType::from_str(&long);
        assert_eq!(ct.len(), ContentType::MAX_LEN);
        assert_eq!(ct.as_str().len(), ContentType::MAX_LEN);
    }

    #[test]
    fn test_content_type_display() {
        let ct = ContentType::from_str("text/plain");
        assert_eq!(format!("{ct}"), "text/plain");
    }

    #[test]
    fn test_slot_vacant() {
        let slot = Slot::Vacant {
            next_free: 5,
            generation: 3,
        };
        assert!(!slot.is_occupied());
        assert!(slot.as_entry().is_none());
        assert_eq!(slot.generation(), 3);
    }

    #[test]
    fn test_buffer_flags_default() {
        let flags = BufferFlags::default();
        assert!(!flags.is_fallback);
        assert!(!flags.read_only);
    }

    #[test]
    fn test_flow_resource_stats_default() {
        let stats = FlowResourceStats::default();
        assert_eq!(stats.returned_to_pool, 0);
        assert_eq!(stats.deallocated, 0);
        assert_eq!(stats.total_bytes_released, 0);
    }
}
