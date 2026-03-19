//! Host-side byte buffer: header + aligned payload.
//!
//! Buffers are allocated as a single contiguous memory region:
//! `[header padding][payload of capacity bytes]`.
//! The payload is aligned to `PAYLOAD_ALIGNMENT` bytes.
//!
//! # Safety
//! This module contains `unsafe` code for allocation, deallocation, and
//! raw pointer access. Every `unsafe` block has a `// SAFETY:` comment.

use std::alloc::{self, Layout};
use std::ptr::NonNull;

/// Payload alignment in bytes.
///
/// Per Doc 03, Section 4.2: 64 bytes (cache-line aligned, AVX-512 friendly).
pub const PAYLOAD_ALIGNMENT: usize = 64;

/// Maximum global buffer size in bytes.
///
/// Per Doc 03, Section 4.3 and `torvyn-types` constants: 16 MiB.
pub const MAX_BUFFER_SIZE: u32 = 16 * 1024 * 1024;

/// Header size before payload, rounded up to alignment.
/// We store a small header: just the capacity (u32) + len (u32) = 8 bytes.
/// But we pad to PAYLOAD_ALIGNMENT so the payload starts aligned.
const HEADER_SIZE: usize = PAYLOAD_ALIGNMENT;

/// Compute the allocation Layout for a buffer with the given payload capacity.
///
/// # WARM PATH — called per allocation.
///
/// # Errors
/// Returns `None` if the layout cannot be constructed (e.g., overflow).
#[inline]
fn buffer_layout(payload_capacity: u32) -> Option<Layout> {
    let total_size = HEADER_SIZE.checked_add(payload_capacity as usize)?;
    Layout::from_size_align(total_size, PAYLOAD_ALIGNMENT).ok()
}

/// Allocate a new buffer with the given payload capacity.
///
/// Returns `(ptr, alloc_size)` where `ptr` points to the start of the
/// allocation and `alloc_size` is the total allocated bytes.
///
/// # WARM PATH — called per buffer allocation (pool miss or initial fill).
///
/// # Safety
/// The caller must eventually call [`dealloc_buffer`] with the same pointer
/// and alloc_size to free the memory. Failure to do so leaks memory.
///
/// # Errors
/// Returns `None` if allocation fails (OOM) or the capacity exceeds the global max.
pub fn alloc_buffer(payload_capacity: u32) -> Option<(NonNull<u8>, usize)> {
    if payload_capacity > MAX_BUFFER_SIZE {
        return None;
    }

    let layout = buffer_layout(payload_capacity)?;

    // SAFETY: layout is non-zero-sized (HEADER_SIZE >= 64, so total >= 64).
    // The returned pointer is non-null or the allocation failed (returns null).
    let ptr = unsafe { alloc::alloc_zeroed(layout) };

    NonNull::new(ptr).map(|nn| (nn, layout.size()))
}

/// Deallocate a buffer previously allocated by [`alloc_buffer`].
///
/// # WARM PATH — called per buffer deallocation.
///
/// # Safety
/// - `ptr` must have been returned by [`alloc_buffer`] with the same `alloc_size`.
/// - `ptr` must not have been previously deallocated.
/// - No references to the buffer memory may exist after this call.
pub unsafe fn dealloc_buffer(ptr: NonNull<u8>, alloc_size: usize) {
    // SAFETY: The caller guarantees `ptr` was allocated with this layout.
    let layout = Layout::from_size_align_unchecked(alloc_size, PAYLOAD_ALIGNMENT);
    alloc::dealloc(ptr.as_ptr(), layout);
}

/// Returns a slice of the payload region of a buffer.
///
/// # HOT PATH — called per buffer read.
///
/// # Safety
/// - `buffer_ptr` must point to a valid buffer allocation of at least
///   `HEADER_SIZE + offset + len` bytes.
/// - The payload region must contain valid data in `[offset..offset+len]`.
/// - No mutable references to the same payload region may exist.
#[inline]
pub unsafe fn read_payload(buffer_ptr: NonNull<u8>, offset: u32, len: u32) -> &'static [u8] {
    // SAFETY: The caller guarantees bounds and validity.
    let payload_start = buffer_ptr.as_ptr().add(HEADER_SIZE);
    let read_start = payload_start.add(offset as usize);
    std::slice::from_raw_parts(read_start, len as usize)
}

/// Write bytes into the payload region of a buffer.
///
/// # HOT PATH — called per buffer write.
///
/// # Safety
/// - `buffer_ptr` must point to a valid buffer allocation of at least
///   `HEADER_SIZE + offset + data.len()` bytes.
/// - The caller must have exclusive access to the buffer (owner, not borrowed).
/// - `offset + data.len()` must not exceed the buffer's payload capacity.
#[inline]
pub unsafe fn write_payload(buffer_ptr: NonNull<u8>, offset: u32, data: &[u8]) {
    // SAFETY: The caller guarantees bounds, ownership, and exclusive access.
    let payload_start = buffer_ptr.as_ptr().add(HEADER_SIZE);
    let write_start = payload_start.add(offset as usize);
    std::ptr::copy_nonoverlapping(data.as_ptr(), write_start, data.len());
}

/// Zero out the payload region of a buffer (for pool return).
///
/// # WARM PATH — called when returning a buffer to the pool.
///
/// # Safety
/// - `buffer_ptr` must point to a valid buffer allocation.
/// - `capacity` must not exceed the actual allocation size minus header.
#[inline]
pub unsafe fn zero_payload(buffer_ptr: NonNull<u8>, capacity: u32) {
    let payload_start = buffer_ptr.as_ptr().add(HEADER_SIZE);
    // SAFETY: The caller guarantees the bounds.
    std::ptr::write_bytes(payload_start, 0, capacity as usize);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_and_dealloc_small() {
        let (ptr, alloc_size) = alloc_buffer(256).expect("allocation failed");
        assert!(alloc_size >= HEADER_SIZE + 256);
        // SAFETY: ptr was just allocated and we own it.
        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_alloc_and_dealloc_medium() {
        let (ptr, alloc_size) = alloc_buffer(4096).expect("allocation failed");
        assert!(alloc_size >= HEADER_SIZE + 4096);
        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_alloc_and_dealloc_large() {
        let (ptr, alloc_size) = alloc_buffer(65536).expect("allocation failed");
        assert!(alloc_size >= HEADER_SIZE + 65536);
        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_alloc_exceeds_max_returns_none() {
        let result = alloc_buffer(MAX_BUFFER_SIZE + 1);
        assert!(result.is_none());
    }

    #[test]
    fn test_alloc_at_max_succeeds() {
        let result = alloc_buffer(MAX_BUFFER_SIZE);
        assert!(result.is_some());
        let (ptr, alloc_size) = result.unwrap();
        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_write_and_read_payload() {
        let (ptr, alloc_size) = alloc_buffer(256).expect("allocation failed");
        let data = b"hello, torvyn!";

        unsafe {
            write_payload(ptr, 0, data);
            let read_back = read_payload(ptr, 0, data.len() as u32);
            assert_eq!(read_back, data);
        }

        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_write_at_offset_and_read() {
        let (ptr, alloc_size) = alloc_buffer(256).expect("allocation failed");
        let data = b"world";

        unsafe {
            write_payload(ptr, 10, data);
            let read_back = read_payload(ptr, 10, data.len() as u32);
            assert_eq!(read_back, data);

            // First 10 bytes should still be zero (from alloc_zeroed)
            let zeroes = read_payload(ptr, 0, 10);
            assert_eq!(zeroes, &[0u8; 10]);
        }

        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_zero_payload() {
        let (ptr, alloc_size) = alloc_buffer(256).expect("allocation failed");

        unsafe {
            write_payload(ptr, 0, b"non-zero data here");
            zero_payload(ptr, 256);
            let read_back = read_payload(ptr, 0, 256);
            assert!(read_back.iter().all(|&b| b == 0));
        }

        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_payload_alignment() {
        let (ptr, alloc_size) = alloc_buffer(256).expect("allocation failed");
        // SAFETY: ptr is valid.
        let payload_addr = unsafe { ptr.as_ptr().add(HEADER_SIZE) } as usize;
        assert_eq!(
            payload_addr % PAYLOAD_ALIGNMENT,
            0,
            "payload is not aligned to {PAYLOAD_ALIGNMENT} bytes"
        );
        unsafe { dealloc_buffer(ptr, alloc_size) };
    }

    #[test]
    fn test_alloc_buffer_zero_capacity() {
        // Zero-capacity buffer is valid (header-only)
        let result = alloc_buffer(0);
        assert!(result.is_some());
        let (ptr, alloc_size) = result.unwrap();
        assert!(alloc_size >= HEADER_SIZE);
        unsafe { dealloc_buffer(ptr, alloc_size) };
    }
}
