//! Internal error helpers for the torvyn-resources crate.
//!
//! The canonical [`ResourceError`] type lives in `torvyn-types`. This module
//! provides crate-internal convenience constructors and a `Result` alias.

// Some helpers are defined for Part B (ownership, manager modules).
#![allow(dead_code)]

use torvyn_types::{BufferHandle, ComponentId, ResourceError};

/// Crate-internal result type alias.
pub(crate) type Result<T> = std::result::Result<T, ResourceError>;

/// Create a [`ResourceError::StaleHandle`] from a [`BufferHandle`].
///
/// # COLD PATH — errors are not on the happy path.
#[inline]
pub(crate) fn stale_handle(handle: BufferHandle) -> ResourceError {
    ResourceError::StaleHandle {
        handle: handle.resource_id(),
    }
}

/// Create a [`ResourceError::NotOwner`] error.
///
/// # COLD PATH
#[inline]
pub(crate) fn not_owner(
    handle: BufferHandle,
    expected: &str,
    actual: &str,
) -> ResourceError {
    ResourceError::NotOwner {
        handle: handle.resource_id(),
        expected_owner: expected.to_string(),
        actual_caller: actual.to_string(),
    }
}

/// Create a [`ResourceError::NotAllocated`] error.
///
/// # COLD PATH
#[inline]
pub(crate) fn not_allocated(handle: BufferHandle) -> ResourceError {
    ResourceError::NotAllocated {
        handle: handle.resource_id(),
    }
}

/// Create a [`ResourceError::BorrowsOutstanding`] error.
///
/// # COLD PATH
#[inline]
pub(crate) fn borrows_outstanding(handle: BufferHandle, count: u32) -> ResourceError {
    ResourceError::BorrowsOutstanding {
        handle: handle.resource_id(),
        borrow_count: count,
    }
}

/// Create a [`ResourceError::BudgetExceeded`] error.
///
/// # COLD PATH
#[inline]
pub(crate) fn budget_exceeded(
    component: ComponentId,
    current: u64,
    requested: u64,
    budget: u64,
) -> ResourceError {
    ResourceError::BudgetExceeded {
        component,
        current_bytes: current,
        requested_bytes: requested,
        budget_bytes: budget,
    }
}

/// Create a [`ResourceError::AllocationFailed`] error.
///
/// # COLD PATH
#[inline]
pub(crate) fn allocation_failed(capacity: u32, reason: &str) -> ResourceError {
    ResourceError::AllocationFailed {
        requested_capacity: capacity,
        reason: reason.to_string(),
    }
}

/// Create a [`ResourceError::OutOfBounds`] error.
///
/// # COLD PATH
#[inline]
pub(crate) fn out_of_bounds(handle: BufferHandle, offset: u64, size: u64) -> ResourceError {
    ResourceError::OutOfBounds {
        handle: handle.resource_id(),
        offset,
        buffer_size: size,
    }
}

/// Create a [`ResourceError::CapacityExceeded`] error.
///
/// # COLD PATH
#[inline]
pub(crate) fn capacity_exceeded(
    handle: BufferHandle,
    capacity: u32,
    attempted: u64,
) -> ResourceError {
    ResourceError::CapacityExceeded {
        handle: handle.resource_id(),
        capacity,
        attempted_size: attempted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::ResourceId;

    #[test]
    fn test_stale_handle_error_contains_resource_id() {
        let handle = BufferHandle::new(ResourceId::new(5, 3));
        let err = stale_handle(handle);
        let msg = format!("{err}");
        assert!(msg.contains("resource-5:g3"));
    }

    #[test]
    fn test_not_owner_error_contains_details() {
        let handle = BufferHandle::new(ResourceId::new(1, 0));
        let err = not_owner(handle, "host", "component-7");
        let msg = format!("{err}");
        assert!(msg.contains("host"));
        assert!(msg.contains("component-7"));
    }

    #[test]
    fn test_budget_exceeded_error_contains_amounts() {
        let err = budget_exceeded(ComponentId::new(3), 900, 200, 1024);
        let msg = format!("{err}");
        assert!(msg.contains("900"));
        assert!(msg.contains("200"));
        assert!(msg.contains("1024"));
    }

    #[test]
    fn test_allocation_failed_error_contains_reason() {
        let err = allocation_failed(4096, "system OOM");
        let msg = format!("{err}");
        assert!(msg.contains("4096"));
        assert!(msg.contains("system OOM"));
    }

    #[test]
    fn test_out_of_bounds_error_contains_offset() {
        let handle = BufferHandle::new(ResourceId::new(2, 1));
        let err = out_of_bounds(handle, 500, 256);
        let msg = format!("{err}");
        assert!(msg.contains("500"));
        assert!(msg.contains("256"));
    }
}
