//! Ownership state machine enforcement.
//!
//! Validates every resource state transition against the legal transition table.
//! Per Doc 03, Section 3.2-3.3, with leases deferred per Doc 10 MR-09.
//!
//! Phase 0 states: Pooled, Owned, Borrowed, Transit, Freed.
//!
//! # Legal Transitions (Phase 0)
//! - Pooled → Owned (allocate)
//! - Owned → Borrowed (borrow_start)
//! - Owned → Transit (begin_transfer)
//! - Owned → Pooled (release)
//! - Owned → Freed (force reclaim / shutdown)
//! - Borrowed → Owned (borrow_end, when count reaches 0)
//! - Borrowed → Borrowed (additional borrow)
//! - Transit → Owned (complete_transfer)
//! - Any → Freed (force reclaim / shutdown)

use torvyn_types::{BufferHandle, ResourceError, ResourceState};

use crate::error;
use crate::handle::{OwnerId, ResourceEntry};

/// Validate and perform a borrow_start transition.
///
/// Preconditions:
/// - Entry state must be Owned or Borrowed.
/// - Caller must not be the owner (borrowing from yourself is a no-op).
///
/// Postconditions:
/// - State is Borrowed.
/// - borrow_count is incremented.
///
/// # HOT PATH — called per borrow<buffer> invocation.
pub fn borrow_start(
    handle: BufferHandle,
    entry: &mut ResourceEntry,
    _borrower: torvyn_types::ComponentId,
) -> error::Result<()> {
    match entry.state {
        ResourceState::Owned | ResourceState::Borrowed => {
            entry.borrow_count += 1;
            entry.state = ResourceState::Borrowed;
            Ok(())
        }
        ResourceState::Pooled => Err(error::not_allocated(handle)),
        _ => Err(ResourceError::StaleHandle {
            handle: handle.resource_id(),
        }),
    }
}

/// Validate and perform a borrow_end transition.
///
/// Preconditions:
/// - Entry state must be Borrowed.
/// - borrow_count must be > 0.
///
/// Postconditions:
/// - borrow_count is decremented.
/// - If borrow_count reaches 0, state transitions to Owned.
///
/// # HOT PATH — called when component function returns.
pub fn borrow_end(
    handle: BufferHandle,
    entry: &mut ResourceEntry,
    _borrower: torvyn_types::ComponentId,
) -> error::Result<()> {
    match entry.state {
        ResourceState::Borrowed => {
            if entry.borrow_count == 0 {
                // Shouldn't happen — Borrowed state with zero borrows is invalid.
                return Err(ResourceError::StaleHandle {
                    handle: handle.resource_id(),
                });
            }
            entry.borrow_count -= 1;
            if entry.borrow_count == 0 {
                entry.state = ResourceState::Owned;
            }
            Ok(())
        }
        _ => Err(ResourceError::StaleHandle {
            handle: handle.resource_id(),
        }),
    }
}

/// Validate and perform an ownership transfer start (Owned → Transit).
///
/// Preconditions:
/// - Entry state must be Owned.
/// - Caller must be the current owner.
/// - No outstanding borrows.
///
/// Postconditions:
/// - State is Transit.
/// - Owner is OwnerId::Transit.
///
/// # HOT PATH — called per cross-component transfer.
pub fn begin_transfer(
    handle: BufferHandle,
    entry: &mut ResourceEntry,
    caller: OwnerId,
) -> error::Result<()> {
    if entry.state != ResourceState::Owned {
        return match entry.state {
            ResourceState::Borrowed => Err(error::borrows_outstanding(handle, entry.borrow_count)),
            ResourceState::Pooled => Err(error::not_allocated(handle)),
            _ => Err(error::stale_handle(handle)),
        };
    }

    if entry.owner != caller {
        return Err(error::not_owner(
            handle,
            &format!("{}", entry.owner),
            &format!("{}", caller),
        ));
    }

    if entry.borrow_count > 0 {
        return Err(error::borrows_outstanding(handle, entry.borrow_count));
    }

    entry.state = ResourceState::Transit;
    entry.owner = OwnerId::Transit;
    Ok(())
}

/// Validate and perform an ownership transfer completion (Transit → Owned).
///
/// Preconditions:
/// - Entry state must be Transit.
///
/// Postconditions:
/// - State is Owned.
/// - Owner is set to the new owner.
///
/// # HOT PATH
pub fn complete_transfer(
    handle: BufferHandle,
    entry: &mut ResourceEntry,
    new_owner: OwnerId,
) -> error::Result<()> {
    if entry.state != ResourceState::Transit {
        return Err(error::stale_handle(handle));
    }

    entry.state = ResourceState::Owned;
    entry.owner = new_owner;
    Ok(())
}

/// Validate and perform a release (Owned → Pooled).
///
/// Preconditions:
/// - Entry state must be Owned.
/// - Caller must be the owner.
/// - No outstanding borrows.
///
/// Postconditions:
/// - State is Pooled.
///
/// # WARM PATH
pub fn release_to_pool(
    handle: BufferHandle,
    entry: &mut ResourceEntry,
    caller: OwnerId,
) -> error::Result<()> {
    if entry.state != ResourceState::Owned {
        return match entry.state {
            ResourceState::Borrowed => Err(error::borrows_outstanding(handle, entry.borrow_count)),
            ResourceState::Pooled => Err(error::not_allocated(handle)),
            _ => Err(error::stale_handle(handle)),
        };
    }

    if entry.owner != caller {
        return Err(error::not_owner(
            handle,
            &format!("{}", entry.owner),
            &format!("{}", caller),
        ));
    }

    if entry.borrow_count > 0 {
        return Err(error::borrows_outstanding(handle, entry.borrow_count));
    }

    entry.state = ResourceState::Pooled;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::{BufferFlags, ContentType, PoolTier};
    use torvyn_types::{ComponentId, FlowId, ResourceId, ResourceState};

    fn test_entry(state: ResourceState, owner: OwnerId) -> ResourceEntry {
        ResourceEntry {
            generation: 1,
            state,
            owner,
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

    fn test_handle() -> BufferHandle {
        BufferHandle::new(ResourceId::new(0, 1))
    }

    // --- borrow_start ---

    #[test]
    fn test_borrow_start_from_owned() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        borrow_start(handle, &mut entry, ComponentId::new(1)).unwrap();
        assert_eq!(entry.state, ResourceState::Borrowed);
        assert_eq!(entry.borrow_count, 1);
    }

    #[test]
    fn test_borrow_start_additional_borrow() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        borrow_start(handle, &mut entry, ComponentId::new(1)).unwrap();
        borrow_start(handle, &mut entry, ComponentId::new(2)).unwrap();
        assert_eq!(entry.state, ResourceState::Borrowed);
        assert_eq!(entry.borrow_count, 2);
    }

    #[test]
    fn test_borrow_start_from_pooled_fails() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Pooled, OwnerId::Host);
        let result = borrow_start(handle, &mut entry, ComponentId::new(1));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResourceError::NotAllocated { .. }
        ));
    }

    // --- borrow_end ---

    #[test]
    fn test_borrow_end_single() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        borrow_start(handle, &mut entry, ComponentId::new(1)).unwrap();
        borrow_end(handle, &mut entry, ComponentId::new(1)).unwrap();
        assert_eq!(entry.state, ResourceState::Owned);
        assert_eq!(entry.borrow_count, 0);
    }

    #[test]
    fn test_borrow_end_multiple() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        borrow_start(handle, &mut entry, ComponentId::new(1)).unwrap();
        borrow_start(handle, &mut entry, ComponentId::new(2)).unwrap();
        borrow_end(handle, &mut entry, ComponentId::new(1)).unwrap();
        assert_eq!(entry.state, ResourceState::Borrowed);
        assert_eq!(entry.borrow_count, 1);
        borrow_end(handle, &mut entry, ComponentId::new(2)).unwrap();
        assert_eq!(entry.state, ResourceState::Owned);
    }

    // --- begin_transfer ---

    #[test]
    fn test_begin_transfer_success() {
        let handle = test_handle();
        let owner = OwnerId::Component(ComponentId::new(1));
        let mut entry = test_entry(ResourceState::Owned, owner);
        begin_transfer(handle, &mut entry, owner).unwrap();
        assert_eq!(entry.state, ResourceState::Transit);
        assert_eq!(entry.owner, OwnerId::Transit);
    }

    #[test]
    fn test_begin_transfer_wrong_owner_fails() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        let result = begin_transfer(
            handle,
            &mut entry,
            OwnerId::Component(ComponentId::new(99)),
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResourceError::NotOwner { .. }
        ));
    }

    #[test]
    fn test_begin_transfer_with_borrows_fails() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        borrow_start(handle, &mut entry, ComponentId::new(1)).unwrap();
        // State is Borrowed now, which should also fail:
        let result = begin_transfer(handle, &mut entry, OwnerId::Host);
        assert!(result.is_err());
    }

    // --- complete_transfer ---

    #[test]
    fn test_complete_transfer_success() {
        let handle = test_handle();
        let owner = OwnerId::Component(ComponentId::new(1));
        let new_owner = OwnerId::Component(ComponentId::new(2));
        let mut entry = test_entry(ResourceState::Owned, owner);
        begin_transfer(handle, &mut entry, owner).unwrap();
        complete_transfer(handle, &mut entry, new_owner).unwrap();
        assert_eq!(entry.state, ResourceState::Owned);
        assert_eq!(entry.owner, new_owner);
    }

    #[test]
    fn test_complete_transfer_not_in_transit_fails() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        let result = complete_transfer(
            handle,
            &mut entry,
            OwnerId::Component(ComponentId::new(2)),
        );
        assert!(result.is_err());
    }

    // --- release_to_pool ---

    #[test]
    fn test_release_to_pool_success() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        release_to_pool(handle, &mut entry, OwnerId::Host).unwrap();
        assert_eq!(entry.state, ResourceState::Pooled);
    }

    #[test]
    fn test_release_to_pool_wrong_owner_fails() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        let result = release_to_pool(
            handle,
            &mut entry,
            OwnerId::Component(ComponentId::new(5)),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_release_to_pool_with_borrows_fails() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Owned, OwnerId::Host);
        borrow_start(handle, &mut entry, ComponentId::new(1)).unwrap();
        let result = release_to_pool(handle, &mut entry, OwnerId::Host);
        assert!(result.is_err());
    }

    #[test]
    fn test_release_to_pool_already_pooled_fails() {
        let handle = test_handle();
        let mut entry = test_entry(ResourceState::Pooled, OwnerId::Host);
        let result = release_to_pool(handle, &mut entry, OwnerId::Host);
        assert!(result.is_err());
    }

    // --- full lifecycle ---

    #[test]
    fn test_full_lifecycle_allocate_borrow_release() {
        let handle = test_handle();
        let owner = OwnerId::Component(ComponentId::new(1));
        let mut entry = test_entry(ResourceState::Owned, owner);

        // Borrow
        borrow_start(handle, &mut entry, ComponentId::new(2)).unwrap();
        assert_eq!(entry.state, ResourceState::Borrowed);

        // Return borrow
        borrow_end(handle, &mut entry, ComponentId::new(2)).unwrap();
        assert_eq!(entry.state, ResourceState::Owned);

        // Release to pool
        release_to_pool(handle, &mut entry, owner).unwrap();
        assert_eq!(entry.state, ResourceState::Pooled);
    }

    #[test]
    fn test_full_lifecycle_transfer() {
        let handle = test_handle();
        let owner_a = OwnerId::Component(ComponentId::new(1));
        let owner_b = OwnerId::Component(ComponentId::new(2));
        let mut entry = test_entry(ResourceState::Owned, owner_a);

        // Transfer A → B
        begin_transfer(handle, &mut entry, owner_a).unwrap();
        complete_transfer(handle, &mut entry, owner_b).unwrap();
        assert_eq!(entry.owner, owner_b);
        assert_eq!(entry.state, ResourceState::Owned);

        // B releases
        release_to_pool(handle, &mut entry, owner_b).unwrap();
        assert_eq!(entry.state, ResourceState::Pooled);
    }
}
