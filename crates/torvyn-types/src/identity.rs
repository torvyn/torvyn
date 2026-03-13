//! Identity types for the Torvyn runtime.
//!
//! These are the foundational identifier types used by every subsystem.
//! All identity types are `Copy`, `Eq`, `Hash`, and cheaply comparable.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ComponentTypeId
// ---------------------------------------------------------------------------

/// Content-addressed identifier for a compiled component artifact.
///
/// Derived from the SHA-256 hash of the component binary. Two components
/// compiled from the same source with the same toolchain produce the same
/// `ComponentTypeId`. Used for compilation caching and artifact deduplication.
///
/// # Invariants
/// - The inner `[u8; 32]` is always a valid SHA-256 digest.
/// - Two distinct component binaries must never share a `ComponentTypeId`
///   (guaranteed by SHA-256 collision resistance).
///
/// # Examples
/// ```
/// use torvyn_types::ComponentTypeId;
///
/// let hash = [0xab; 32];
/// let id = ComponentTypeId::new(hash);
/// assert_eq!(id.as_bytes(), &[0xab; 32]);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ComponentTypeId([u8; 32]);

impl ComponentTypeId {
    /// Create a new `ComponentTypeId` from a SHA-256 digest.
    ///
    /// # COLD PATH — called during component compilation/loading.
    #[inline]
    pub const fn new(hash: [u8; 32]) -> Self {
        Self(hash)
    }

    /// Returns the raw bytes of the content hash.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns a zero-valued `ComponentTypeId`, useful as a sentinel or default.
    #[inline]
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }
}

impl fmt::Debug for ComponentTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ComponentTypeId({})", self)
    }
}

impl fmt::Display for ComponentTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display first 8 bytes as hex for readability (16 hex chars)
        for byte in &self.0[..8] {
            write!(f, "{:02x}", byte)?;
        }
        write!(f, "\u{2026}")
    }
}

// ---------------------------------------------------------------------------
// ComponentInstanceId
// ---------------------------------------------------------------------------

/// Runtime identity for a component instance within a host.
///
/// Assigned by the host at instantiation time. Monotonically increasing
/// within a single host process lifetime. Not stable across restarts.
///
/// # Invariants
/// - Unique within a single host process.
/// - Never reused during the lifetime of a host process.
///
/// # Examples
/// ```
/// use torvyn_types::ComponentInstanceId;
///
/// let id = ComponentInstanceId::new(42);
/// assert_eq!(id.as_u64(), 42);
/// assert_eq!(format!("{}", id), "component-42");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ComponentInstanceId(u64);

impl ComponentInstanceId {
    /// Create a new `ComponentInstanceId`.
    ///
    /// # COLD PATH — called during component instantiation.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw `u64` value.
    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Debug for ComponentInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ComponentInstanceId({})", self.0)
    }
}

impl fmt::Display for ComponentInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "component-{}", self.0)
    }
}

impl From<u64> for ComponentInstanceId {
    #[inline]
    fn from(id: u64) -> Self {
        Self(id)
    }
}

/// Type alias for `ComponentInstanceId`.
///
/// Per consolidated review (Doc 10, Section 7.2): `ComponentId` is the
/// canonical short name used throughout the runtime. It is an alias for
/// `ComponentInstanceId` to resolve naming conflicts between Doc 02, 03, and 04.
pub type ComponentId = ComponentInstanceId;

// ---------------------------------------------------------------------------
// FlowId
// ---------------------------------------------------------------------------

/// Unique identifier for a flow (pipeline execution instance).
///
/// Assigned by the reactor when a flow is created. Monotonically increasing.
/// Not reused within a single host process lifetime.
///
/// # Invariants
/// - Unique within a single host process.
/// - Never reused.
///
/// # Examples
/// ```
/// use torvyn_types::FlowId;
///
/// let id = FlowId::new(7);
/// assert_eq!(id.as_u64(), 7);
/// assert_eq!(format!("{}", id), "flow-7");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FlowId(u64);

impl FlowId {
    /// Create a new `FlowId`.
    ///
    /// # COLD PATH — called during flow creation.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw `u64` value.
    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Debug for FlowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FlowId({})", self.0)
    }
}

impl fmt::Display for FlowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "flow-{}", self.0)
    }
}

impl From<u64> for FlowId {
    #[inline]
    fn from(id: u64) -> Self {
        Self(id)
    }
}

// ---------------------------------------------------------------------------
// StreamId
// ---------------------------------------------------------------------------

/// Unique identifier for a stream connection within the reactor.
///
/// A stream connects two components in a pipeline. Each stream has a
/// bounded queue and backpressure policy.
///
/// # Invariants
/// - Unique within a single host process.
///
/// # Examples
/// ```
/// use torvyn_types::StreamId;
///
/// let id = StreamId::new(3);
/// assert_eq!(id.as_u64(), 3);
/// assert_eq!(format!("{}", id), "stream-3");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct StreamId(u64);

impl StreamId {
    /// Create a new `StreamId`.
    ///
    /// # COLD PATH — called during flow construction.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw `u64` value.
    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Debug for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StreamId({})", self.0)
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stream-{}", self.0)
    }
}

impl From<u64> for StreamId {
    #[inline]
    fn from(id: u64) -> Self {
        Self(id)
    }
}

// ---------------------------------------------------------------------------
// ResourceId
// ---------------------------------------------------------------------------

/// Generational index into the resource table.
///
/// Combines a slot index with a generation counter to prevent ABA problems.
/// When a resource is freed and its slot reused, the generation is incremented.
/// Any handle holding the old generation will fail validation.
///
/// # Invariants
/// - `index` identifies the slot in the resource table.
/// - `generation` is incremented each time the slot is reused.
/// - A handle is valid only if its generation matches the slot's current generation.
///
/// # Examples
/// ```
/// use torvyn_types::ResourceId;
///
/// let id = ResourceId::new(10, 1);
/// assert_eq!(id.index(), 10);
/// assert_eq!(id.generation(), 1);
/// assert_eq!(format!("{}", id), "resource-10:g1");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceId {
    index: u32,
    generation: u32,
}

impl ResourceId {
    /// Create a new `ResourceId`.
    ///
    /// # HOT PATH — called during resource allocation.
    #[inline]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Returns the slot index.
    #[inline]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns the generation counter.
    #[inline]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Returns a new `ResourceId` with the same index but incremented generation.
    ///
    /// # HOT PATH — called when a resource slot is reused.
    #[inline]
    pub const fn next_generation(&self) -> Self {
        Self {
            index: self.index,
            generation: self.generation.wrapping_add(1),
        }
    }
}

impl fmt::Debug for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResourceId({}, gen={})", self.index, self.generation)
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "resource-{}:g{}", self.index, self.generation)
    }
}

// ---------------------------------------------------------------------------
// BufferHandle
// ---------------------------------------------------------------------------

/// Typed wrapper for buffer resources, built on `ResourceId`.
///
/// Provides type safety: you cannot accidentally pass a `BufferHandle` where
/// a raw `ResourceId` for a different resource kind is expected.
///
/// # Invariants
/// - The inner `ResourceId` must refer to a buffer slot in the resource table.
///
/// # Examples
/// ```
/// use torvyn_types::{BufferHandle, ResourceId};
///
/// let rid = ResourceId::new(5, 0);
/// let handle = BufferHandle::new(rid);
/// assert_eq!(handle.resource_id().index(), 5);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BufferHandle(ResourceId);

impl BufferHandle {
    /// Create a new `BufferHandle` from a `ResourceId`.
    ///
    /// # HOT PATH — called during buffer allocation.
    #[inline]
    pub const fn new(resource_id: ResourceId) -> Self {
        Self(resource_id)
    }

    /// Returns the underlying `ResourceId`.
    #[inline]
    pub const fn resource_id(&self) -> ResourceId {
        self.0
    }
}

impl fmt::Debug for BufferHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BufferHandle({})", self.0)
    }
}

impl fmt::Display for BufferHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "buffer-{}:g{}", self.0.index(), self.0.generation())
    }
}

impl From<ResourceId> for BufferHandle {
    #[inline]
    fn from(id: ResourceId) -> Self {
        Self(id)
    }
}

// ---------------------------------------------------------------------------
// TraceId
// ---------------------------------------------------------------------------

/// W3C Trace Context trace ID (128-bit / 16 bytes).
///
/// Used for distributed trace correlation across pipeline components.
/// Formatted as 32 lowercase hex characters per the W3C specification.
///
/// # Invariants
/// - An all-zero trace ID is considered invalid (per W3C spec).
///
/// # Examples
/// ```
/// use torvyn_types::TraceId;
///
/// let id = TraceId::new([0x4b, 0xf9, 0x2f, 0x35, 0x77, 0xb3, 0x4d, 0xa6,
///                        0xa3, 0xce, 0x92, 0x9d, 0x0e, 0x0e, 0x47, 0x36]);
/// assert!(id.is_valid());
/// assert_eq!(id.to_string().len(), 32);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// Create a new `TraceId` from raw bytes.
    ///
    /// # COLD PATH — called during flow creation or trace context propagation.
    #[inline]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Returns `true` if this trace ID is valid (non-zero per W3C spec).
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.0 != [0u8; 16]
    }

    /// Returns an invalid (all-zero) trace ID, indicating tracing is disabled.
    #[inline]
    pub const fn invalid() -> Self {
        Self([0u8; 16])
    }
}

impl fmt::Debug for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TraceId({})", self)
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SpanId
// ---------------------------------------------------------------------------

/// W3C Trace Context span ID (64-bit / 8 bytes).
///
/// Identifies a specific span within a trace. Formatted as 16 lowercase
/// hex characters per the W3C specification.
///
/// # Invariants
/// - An all-zero span ID is considered invalid (per W3C spec).
///
/// # Examples
/// ```
/// use torvyn_types::SpanId;
///
/// let id = SpanId::new([0x00, 0xf0, 0x67, 0xaa, 0x0b, 0xa9, 0x02, 0xb7]);
/// assert!(id.is_valid());
/// assert_eq!(id.to_string().len(), 16);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SpanId([u8; 8]);

impl SpanId {
    /// Create a new `SpanId` from raw bytes.
    ///
    /// # WARM PATH — called per span creation.
    #[inline]
    pub const fn new(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }

    /// Returns `true` if this span ID is valid (non-zero per W3C spec).
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.0 != [0u8; 8]
    }

    /// Returns an invalid (all-zero) span ID.
    #[inline]
    pub const fn invalid() -> Self {
        Self([0u8; 8])
    }
}

impl fmt::Debug for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SpanId({})", self)
    }
}

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- ComponentTypeId ---

    #[test]
    fn test_component_type_id_new_and_bytes() {
        let hash = [0xab; 32];
        let id = ComponentTypeId::new(hash);
        assert_eq!(id.as_bytes(), &[0xab; 32]);
    }

    #[test]
    fn test_component_type_id_zero() {
        let id = ComponentTypeId::zero();
        assert_eq!(id.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn test_component_type_id_equality() {
        let a = ComponentTypeId::new([1; 32]);
        let b = ComponentTypeId::new([1; 32]);
        let c = ComponentTypeId::new([2; 32]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_component_type_id_display_truncated() {
        let id = ComponentTypeId::new([0xab; 32]);
        let display = format!("{}", id);
        assert!(display.starts_with("abababab"));
        assert!(display.ends_with('\u{2026}'));
    }

    #[test]
    fn test_component_type_id_hash_consistency() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        let id = ComponentTypeId::new([0xcd; 32]);
        set.insert(id);
        assert!(set.contains(&ComponentTypeId::new([0xcd; 32])));
    }

    // --- ComponentInstanceId ---

    #[test]
    fn test_component_instance_id_new_and_value() {
        let id = ComponentInstanceId::new(42);
        assert_eq!(id.as_u64(), 42);
    }

    #[test]
    fn test_component_instance_id_display() {
        let id = ComponentInstanceId::new(42);
        assert_eq!(format!("{}", id), "component-42");
    }

    #[test]
    fn test_component_instance_id_ordering() {
        let a = ComponentInstanceId::new(1);
        let b = ComponentInstanceId::new(2);
        assert!(a < b);
    }

    #[test]
    fn test_component_instance_id_from_u64() {
        let id: ComponentInstanceId = 99u64.into();
        assert_eq!(id.as_u64(), 99);
    }

    #[test]
    fn test_component_id_alias() {
        let id: ComponentId = ComponentInstanceId::new(10);
        assert_eq!(id.as_u64(), 10);
    }

    // --- FlowId ---

    #[test]
    fn test_flow_id_new_and_value() {
        let id = FlowId::new(7);
        assert_eq!(id.as_u64(), 7);
    }

    #[test]
    fn test_flow_id_display() {
        let id = FlowId::new(7);
        assert_eq!(format!("{}", id), "flow-7");
    }

    #[test]
    fn test_flow_id_ordering() {
        let a = FlowId::new(1);
        let b = FlowId::new(2);
        assert!(a < b);
    }

    #[test]
    fn test_flow_id_from_u64() {
        let id: FlowId = 55u64.into();
        assert_eq!(id.as_u64(), 55);
    }

    // --- StreamId ---

    #[test]
    fn test_stream_id_new_and_value() {
        let id = StreamId::new(3);
        assert_eq!(id.as_u64(), 3);
    }

    #[test]
    fn test_stream_id_display() {
        let id = StreamId::new(3);
        assert_eq!(format!("{}", id), "stream-3");
    }

    // --- ResourceId ---

    #[test]
    fn test_resource_id_new_and_fields() {
        let id = ResourceId::new(10, 1);
        assert_eq!(id.index(), 10);
        assert_eq!(id.generation(), 1);
    }

    #[test]
    fn test_resource_id_display() {
        let id = ResourceId::new(10, 1);
        assert_eq!(format!("{}", id), "resource-10:g1");
    }

    #[test]
    fn test_resource_id_next_generation() {
        let id = ResourceId::new(5, 0);
        let next = id.next_generation();
        assert_eq!(next.index(), 5);
        assert_eq!(next.generation(), 1);
    }

    #[test]
    fn test_resource_id_generation_wraps() {
        let id = ResourceId::new(0, u32::MAX);
        let next = id.next_generation();
        assert_eq!(next.generation(), 0);
    }

    #[test]
    fn test_resource_id_different_generations_not_equal() {
        let a = ResourceId::new(5, 0);
        let b = ResourceId::new(5, 1);
        assert_ne!(a, b);
    }

    // --- BufferHandle ---

    #[test]
    fn test_buffer_handle_new_and_resource_id() {
        let rid = ResourceId::new(5, 0);
        let handle = BufferHandle::new(rid);
        assert_eq!(handle.resource_id(), rid);
    }

    #[test]
    fn test_buffer_handle_display() {
        let handle = BufferHandle::new(ResourceId::new(5, 2));
        assert_eq!(format!("{}", handle), "buffer-5:g2");
    }

    #[test]
    fn test_buffer_handle_from_resource_id() {
        let rid = ResourceId::new(3, 1);
        let handle: BufferHandle = rid.into();
        assert_eq!(handle.resource_id(), rid);
    }

    // --- TraceId ---

    #[test]
    fn test_trace_id_valid() {
        let id = TraceId::new([1; 16]);
        assert!(id.is_valid());
    }

    #[test]
    fn test_trace_id_invalid_zero() {
        let id = TraceId::invalid();
        assert!(!id.is_valid());
    }

    #[test]
    fn test_trace_id_display_length() {
        let id = TraceId::new([0xab; 16]);
        assert_eq!(id.to_string().len(), 32); // 16 bytes * 2 hex chars
    }

    #[test]
    fn test_trace_id_display_value() {
        let id = TraceId::new([0x4b, 0xf9, 0x2f, 0x35, 0x77, 0xb3, 0x4d, 0xa6,
                               0xa3, 0xce, 0x92, 0x9d, 0x0e, 0x0e, 0x47, 0x36]);
        assert_eq!(id.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    // --- SpanId ---

    #[test]
    fn test_span_id_valid() {
        let id = SpanId::new([1; 8]);
        assert!(id.is_valid());
    }

    #[test]
    fn test_span_id_invalid_zero() {
        let id = SpanId::invalid();
        assert!(!id.is_valid());
    }

    #[test]
    fn test_span_id_display_length() {
        let id = SpanId::new([0xab; 8]);
        assert_eq!(id.to_string().len(), 16); // 8 bytes * 2 hex chars
    }
}
