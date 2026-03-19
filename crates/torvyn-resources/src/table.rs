//! Generational slab resource table.
//!
//! The [`ResourceTable`] is the central registry of all live resources.
//! It stores [`ResourceEntry`] values in a dense array indexed by the
//! `index` field of `ResourceId`. Generation counters on each slot
//! detect use-after-free (stale handles).
//!
//! # Performance
//! - Lookup: O(1) — array index + generation comparison.
//! - Insert: O(1) amortized — pop from free list, or grow.
//! - Remove: O(1) — push to free list.
//!
//! # Thread Safety
//! `ResourceTable` is NOT internally synchronized. All access must be
//! externally serialized (the `DefaultResourceManager` uses a `Mutex`).

use crate::error;
use crate::handle::{ResourceEntry, Slot};
use torvyn_types::{BufferHandle, ResourceId};

/// Sentinel value for "no next free slot."
const FREE_LIST_END: u32 = u32::MAX;

/// Default initial capacity for the resource table.
///
/// Per Doc 03 Q4: default 65,536 initial capacity, growable to u32::MAX.
pub const DEFAULT_INITIAL_CAPACITY: u32 = 65_536;

/// The resource table: a generational slab.
///
/// # Invariants
/// - `entries.len() == capacity as usize`.
/// - `len` <= `capacity`.
/// - `free_head` is either `FREE_LIST_END` or a valid index into `entries`
///   where `entries[free_head]` is `Slot::Vacant`.
/// - Every occupied slot has `generation` >= 1 (generation 0 is the initial
///   vacant generation).
/// - The free list is acyclic.
///
/// # Examples
/// ```
/// use torvyn_resources::table::ResourceTable;
///
/// let mut table = ResourceTable::new(128);
/// assert_eq!(table.len(), 0);
/// assert_eq!(table.capacity(), 128);
/// ```
pub struct ResourceTable {
    entries: Vec<Slot>,
    free_head: u32,
    len: u32,
    capacity: u32,
}

impl ResourceTable {
    /// Create a new `ResourceTable` with the given initial capacity.
    ///
    /// All slots are initially vacant, forming a linked free list.
    ///
    /// # COLD PATH — called once during host startup.
    ///
    /// # Panics
    /// Panics if `initial_capacity` is 0.
    pub fn new(initial_capacity: u32) -> Self {
        assert!(initial_capacity > 0, "ResourceTable capacity must be > 0");
        let cap = initial_capacity as usize;
        let mut entries = Vec::with_capacity(cap);
        for i in 0..cap {
            let next = if i + 1 < cap {
                (i + 1) as u32
            } else {
                FREE_LIST_END
            };
            entries.push(Slot::Vacant {
                next_free: next,
                generation: 0,
            });
        }
        Self {
            entries,
            free_head: 0,
            len: 0,
            capacity: initial_capacity,
        }
    }

    /// Returns the number of occupied slots.
    ///
    /// # HOT PATH
    #[inline]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Returns `true` if the table has no occupied slots.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the total capacity (number of slots).
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Insert a resource entry, returning the assigned `ResourceId`.
    ///
    /// If the free list is empty, the table grows by doubling capacity.
    ///
    /// # WARM PATH — called per buffer allocation.
    ///
    /// # Errors
    /// Returns `ResourceError::AllocationFailed` if the table cannot grow
    /// (capacity would exceed `u32::MAX`).
    pub fn insert(&mut self, mut entry: ResourceEntry) -> error::Result<ResourceId> {
        if self.free_head == FREE_LIST_END {
            self.grow()?;
        }

        let index = self.free_head;
        let slot = &mut self.entries[index as usize];

        // Extract the generation from the vacant slot and compute the new generation.
        let new_generation = match slot {
            Slot::Vacant {
                next_free,
                generation,
            } => {
                self.free_head = *next_free;
                generation.wrapping_add(1)
            }
            Slot::Occupied(_) => {
                // This should never happen — free_head should always point to a vacant slot.
                return Err(error::allocation_failed(
                    0,
                    "internal error: free_head pointed to an occupied slot",
                ));
            }
        };

        entry.generation = new_generation;
        self.entries[index as usize] = Slot::Occupied(entry);
        self.len += 1;

        Ok(ResourceId::new(index, new_generation))
    }

    /// Look up a resource by handle. Validates the generation.
    ///
    /// # HOT PATH — called per resource operation.
    ///
    /// # Errors
    /// Returns `ResourceError::StaleHandle` if the generation does not match.
    /// Returns `ResourceError::NotAllocated` if the slot is vacant.
    #[inline]
    pub fn get(&self, handle: BufferHandle) -> error::Result<&ResourceEntry> {
        let id = handle.resource_id();
        let index = id.index() as usize;

        if index >= self.entries.len() {
            return Err(error::stale_handle(handle));
        }

        match &self.entries[index] {
            Slot::Occupied(entry) => {
                if entry.generation == id.generation() {
                    Ok(entry)
                } else {
                    Err(error::stale_handle(handle))
                }
            }
            Slot::Vacant { .. } => Err(error::not_allocated(handle)),
        }
    }

    /// Look up a resource mutably by handle. Validates the generation.
    ///
    /// # HOT PATH — called per resource mutation.
    ///
    /// # Errors
    /// Same as `get`.
    #[inline]
    pub fn get_mut(&mut self, handle: BufferHandle) -> error::Result<&mut ResourceEntry> {
        let id = handle.resource_id();
        let index = id.index() as usize;

        if index >= self.entries.len() {
            return Err(error::stale_handle(handle));
        }

        match &mut self.entries[index] {
            Slot::Occupied(entry) => {
                if entry.generation == id.generation() {
                    Ok(entry)
                } else {
                    Err(error::stale_handle(handle))
                }
            }
            Slot::Vacant { .. } => Err(error::not_allocated(handle)),
        }
    }

    /// Remove a resource from the table, returning the entry.
    ///
    /// The slot becomes vacant and is added to the free list.
    /// The generation is preserved in the vacant slot so the next `insert`
    /// will increment it, invalidating any old handles.
    ///
    /// # WARM PATH — called per resource release.
    ///
    /// # Errors
    /// Same as `get`.
    pub fn remove(&mut self, handle: BufferHandle) -> error::Result<ResourceEntry> {
        let id = handle.resource_id();
        let index = id.index() as usize;

        if index >= self.entries.len() {
            return Err(error::stale_handle(handle));
        }

        // Verify the slot is occupied with the correct generation.
        let current_gen = match &self.entries[index] {
            Slot::Occupied(entry) => {
                if entry.generation != id.generation() {
                    return Err(error::stale_handle(handle));
                }
                entry.generation
            }
            Slot::Vacant { .. } => {
                return Err(error::not_allocated(handle));
            }
        };

        // Replace with a vacant slot. Keep the same generation —
        // the next insert will increment it.
        let old_slot = std::mem::replace(
            &mut self.entries[index],
            Slot::Vacant {
                next_free: self.free_head,
                generation: current_gen,
            },
        );
        self.free_head = index as u32;
        self.len -= 1;

        match old_slot {
            Slot::Occupied(entry) => Ok(entry),
            Slot::Vacant { .. } => unreachable!("we already verified the slot was occupied"),
        }
    }

    /// Iterate over all occupied entries with their ResourceIds.
    ///
    /// # COLD PATH — used during cleanup and diagnostics.
    pub fn iter(&self) -> impl Iterator<Item = (ResourceId, &ResourceEntry)> + '_ {
        self.entries.iter().enumerate().filter_map(|(i, slot)| {
            if let Slot::Occupied(entry) = slot {
                Some((ResourceId::new(i as u32, entry.generation), entry))
            } else {
                None
            }
        })
    }

    /// Iterate mutably over all occupied entries with their ResourceIds.
    ///
    /// # COLD PATH — used during cleanup.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (ResourceId, &mut ResourceEntry)> + '_ {
        self.entries.iter_mut().enumerate().filter_map(|(i, slot)| {
            if let Slot::Occupied(entry) = slot {
                let id = ResourceId::new(i as u32, entry.generation);
                Some((id, entry))
            } else {
                None
            }
        })
    }

    /// Grow the table by doubling capacity.
    ///
    /// # COLD PATH — called when the free list is empty.
    fn grow(&mut self) -> error::Result<()> {
        let old_cap = self.capacity;
        let new_cap = old_cap.checked_mul(2).ok_or_else(|| {
            error::allocation_failed(0, "resource table capacity overflow (> u32::MAX)")
        })?;

        let new_cap_usize = new_cap as usize;
        self.entries.reserve(new_cap_usize - self.entries.len());

        // Add new vacant slots, linking them into a free list.
        for i in (old_cap as usize)..new_cap_usize {
            let next = if i + 1 < new_cap_usize {
                (i + 1) as u32
            } else {
                FREE_LIST_END
            };
            self.entries.push(Slot::Vacant {
                next_free: next,
                generation: 0,
            });
        }

        self.free_head = old_cap;
        self.capacity = new_cap;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::{BufferFlags, ContentType, OwnerId, PoolTier};
    use torvyn_types::{FlowId, ResourceError, ResourceState};

    fn make_entry() -> ResourceEntry {
        ResourceEntry {
            generation: 0, // Will be overwritten by insert
            state: ResourceState::Owned,
            owner: OwnerId::Host,
            borrow_count: 0,
            buffer_ptr: std::ptr::NonNull::dangling(),
            alloc_size: 320,
            payload_capacity: 256,
            payload_len: 0,
            pool_tier: PoolTier::Small,
            flags: BufferFlags::default(),
            content_type: ContentType::empty(),
            flow_id: FlowId::new(1),
            created_at_ns: 0,
        }
    }

    #[test]
    fn test_resource_table_new() {
        let table = ResourceTable::new(128);
        assert_eq!(table.len(), 0);
        assert!(table.is_empty());
        assert_eq!(table.capacity(), 128);
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn test_resource_table_new_zero_capacity_panics() {
        let _ = ResourceTable::new(0);
    }

    #[test]
    fn test_resource_table_insert_and_get() {
        let mut table = ResourceTable::new(16);
        let id = table.insert(make_entry()).unwrap();
        assert_eq!(table.len(), 1);

        let handle = BufferHandle::new(id);
        let entry = table.get(handle).unwrap();
        assert_eq!(entry.state, ResourceState::Owned);
        assert_eq!(entry.generation, id.generation());
    }

    #[test]
    fn test_resource_table_insert_sequential_indices() {
        let mut table = ResourceTable::new(16);
        let id0 = table.insert(make_entry()).unwrap();
        let id1 = table.insert(make_entry()).unwrap();
        let id2 = table.insert(make_entry()).unwrap();
        assert_eq!(id0.index(), 0);
        assert_eq!(id1.index(), 1);
        assert_eq!(id2.index(), 2);
        assert_eq!(table.len(), 3);
    }

    #[test]
    fn test_resource_table_remove_and_reinsert() {
        let mut table = ResourceTable::new(16);
        let id0 = table.insert(make_entry()).unwrap();
        let handle0 = BufferHandle::new(id0);

        // Remove slot 0
        let _ = table.remove(handle0).unwrap();
        assert_eq!(table.len(), 0);

        // Reinsert — should reuse slot 0 with a new generation
        let id0_new = table.insert(make_entry()).unwrap();
        assert_eq!(id0_new.index(), 0);
        assert!(id0_new.generation() > id0.generation());
    }

    #[test]
    fn test_resource_table_stale_handle_detected() {
        let mut table = ResourceTable::new(16);
        let id0 = table.insert(make_entry()).unwrap();
        let stale_handle = BufferHandle::new(id0);

        // Remove and reinsert
        let _ = table.remove(stale_handle).unwrap();
        let _id0_new = table.insert(make_entry()).unwrap();

        // Stale handle should fail
        let result = table.get(stale_handle);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ResourceError::StaleHandle { .. }));
    }

    #[test]
    fn test_resource_table_get_vacant_returns_not_allocated() {
        let table = ResourceTable::new(16);
        let handle = BufferHandle::new(ResourceId::new(0, 1));
        let result = table.get(handle);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResourceError::NotAllocated { .. }
        ));
    }

    #[test]
    fn test_resource_table_get_out_of_bounds_returns_stale() {
        let table = ResourceTable::new(4);
        let handle = BufferHandle::new(ResourceId::new(100, 0));
        let result = table.get(handle);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResourceError::StaleHandle { .. }
        ));
    }

    #[test]
    fn test_resource_table_get_mut() {
        let mut table = ResourceTable::new(16);
        let id = table.insert(make_entry()).unwrap();
        let handle = BufferHandle::new(id);

        let entry = table.get_mut(handle).unwrap();
        entry.state = ResourceState::Borrowed;
        entry.borrow_count = 1;

        let entry = table.get(handle).unwrap();
        assert_eq!(entry.state, ResourceState::Borrowed);
        assert_eq!(entry.borrow_count, 1);
    }

    #[test]
    fn test_resource_table_grow() {
        let mut table = ResourceTable::new(2);
        assert_eq!(table.capacity(), 2);

        let _id0 = table.insert(make_entry()).unwrap();
        let _id1 = table.insert(make_entry()).unwrap();
        // Table is now full — next insert triggers growth
        let id2 = table.insert(make_entry()).unwrap();
        assert_eq!(table.capacity(), 4);
        assert_eq!(table.len(), 3);
        assert_eq!(id2.index(), 2);
    }

    #[test]
    fn test_resource_table_remove_stale_handle_fails() {
        let mut table = ResourceTable::new(16);
        let id = table.insert(make_entry()).unwrap();
        let handle = BufferHandle::new(id);
        let _ = table.remove(handle).unwrap();

        // Double-remove should fail
        let result = table.remove(handle);
        assert!(result.is_err());
    }

    #[test]
    fn test_resource_table_iter() {
        let mut table = ResourceTable::new(16);
        let _id0 = table.insert(make_entry()).unwrap();
        let id1 = table.insert(make_entry()).unwrap();
        let _id2 = table.insert(make_entry()).unwrap();

        // Remove id1
        let _ = table.remove(BufferHandle::new(id1)).unwrap();

        let entries: Vec<_> = table.iter().collect();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_resource_table_generation_wrapping() {
        let mut table = ResourceTable::new(4);

        // Manually test generation wrapping by inserting/removing many times
        let mut last_id = table.insert(make_entry()).unwrap();
        for _ in 0..10 {
            let handle = BufferHandle::new(last_id);
            let _ = table.remove(handle).unwrap();
            last_id = table.insert(make_entry()).unwrap();
        }
        // Generation should have incremented
        assert!(last_id.generation() > 1);
    }

    #[test]
    fn test_resource_table_fill_and_empty() {
        let mut table = ResourceTable::new(8);
        let mut handles = Vec::new();

        for _ in 0..8 {
            let id = table.insert(make_entry()).unwrap();
            handles.push(BufferHandle::new(id));
        }
        assert_eq!(table.len(), 8);

        for h in handles {
            table.remove(h).unwrap();
        }
        assert_eq!(table.len(), 0);
        assert!(table.is_empty());
    }
}
