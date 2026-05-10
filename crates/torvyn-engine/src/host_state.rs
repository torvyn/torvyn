//! Host-side state stored in each Wasmtime `Store<HostState>`.
//!
//! `HostState` carries Torvyn-specific context that the bindgen-generated
//! host trait implementations access via `caller.data_mut()`. The three
//! resource types — [`HostBuffer`], [`HostMutableBuffer`], [`HostFlowContext`]
//! — are the Rust counterparts of the WIT resources defined in
//! `torvyn:streaming/types`, plumbed in via the `bindgen!` `with:` redirect
//! in [`crate::wit_bindings`].
//!
//! # Session 2.1 stub
//!
//! The buffer-allocator implementation here is a self-contained stub: data is
//! held in `Vec<u8>` inside resource-table entries, with no interaction with
//! `torvyn-resources`. Session 2.2 replaces the trait bodies with calls into
//! `torvyn-resources::DefaultResourceManager`; the trait surface stays the
//! same, so `wasmtime_invoker` does not need to learn anything new.

use wasmtime::component::{Resource, ResourceTable};

use torvyn_types::ComponentId;

use crate::wit_bindings::data_sink::torvyn::streaming::types as wit_types;
use crate::wit_bindings::data_source::torvyn::streaming::buffer_allocator as wit_alloc;

/// Maximum capacity for a single mutable buffer (16 MiB), per `types.wit`
/// note C01-3 ("internal buffer capacity is capped at 16 MiB").
const MAX_BUFFER_CAPACITY: u64 = 16 * 1024 * 1024;

/// Host state stored in each Wasmtime `Store`.
///
/// This is the `T` in `Store<T>`. It holds Torvyn-specific context: the
/// component identity, store-level resource limits, the fuel budget that
/// was applied at instantiation, and the [`ResourceTable`] that backs all
/// host-defined resources (buffers, mutable buffers, flow contexts).
///
/// The struct is single-owner: only the owning `Store<HostState>` ever
/// holds a `&mut HostState`, so no synchronisation is required.
pub(crate) struct HostState {
    /// The component ID for this instance.
    #[allow(dead_code)]
    pub(crate) component_id: ComponentId,

    /// Resource limits for this store.
    pub(crate) limits: wasmtime::StoreLimits,

    /// The fuel budget configured for this component. Tracked for
    /// observability/diagnostics.
    #[allow(dead_code)]
    pub(crate) fuel_budget: u64,

    /// Resource table backing the host-defined resource handles
    /// (`buffer`, `mutable-buffer`, `flow-context`).
    pub(crate) table: ResourceTable,
}

impl HostState {
    /// Construct a new host state with an empty resource table.
    pub(crate) fn new(
        component_id: ComponentId,
        limits: wasmtime::StoreLimits,
        fuel_budget: u64,
    ) -> Self {
        Self {
            component_id,
            limits,
            fuel_budget,
            table: ResourceTable::new(),
        }
    }
}

/// Host-side data for an immutable WIT `buffer` resource.
///
/// `pub` only because the bindgen `with:` re-export chain needs to publicly
/// reference this type; the module itself is `#[doc(hidden)]` so it does not
/// form part of the crate's public API.
pub struct HostBuffer {
    pub(crate) data: Vec<u8>,
    pub(crate) content_type: String,
}

/// Host-side data for a mutable WIT `mutable-buffer` resource. See
/// [`HostBuffer`] for why this is `pub`.
pub struct HostMutableBuffer {
    pub(crate) data: Vec<u8>,
    pub(crate) capacity: u64,
    pub(crate) content_type: String,
}

/// Host-side data for a WIT `flow-context` resource. See [`HostBuffer`] for
/// why this is `pub`.
///
/// Session 2.1 stub: empty. Session 2.2 carries trace/span identifiers and
/// the deadline propagated from the reactor.
pub struct HostFlowContext;

// ---------------------------------------------------------------------------
// types::Host — the imported `types` interface
// ---------------------------------------------------------------------------

impl wit_types::Host for HostState {}

impl wit_types::HostBuffer for HostState {
    async fn size(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<u64> {
        let buf = self.table.get(&h)?;
        Ok(buf.data.len() as u64)
    }

    async fn content_type(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<String> {
        let buf = self.table.get(&h)?;
        Ok(buf.content_type.clone())
    }

    async fn read(
        &mut self,
        h: Resource<HostBuffer>,
        offset: u64,
        len: u64,
    ) -> wasmtime::Result<Vec<u8>> {
        let buf = self.table.get(&h)?;
        let total = buf.data.len();
        let start = (offset as usize).min(total);
        let end = offset.saturating_add(len).min(total as u64) as usize;
        Ok(buf.data[start..end].to_vec())
    }

    async fn read_all(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<Vec<u8>> {
        let buf = self.table.get(&h)?;
        Ok(buf.data.clone())
    }

    async fn drop(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<()> {
        self.table.delete(h)?;
        Ok(())
    }
}

impl wit_types::HostMutableBuffer for HostState {
    async fn write(
        &mut self,
        h: Resource<HostMutableBuffer>,
        offset: u64,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), wit_types::BufferError>> {
        let buf = self.table.get_mut(&h)?;
        let needed = offset.saturating_add(bytes.len() as u64);
        if needed > buf.capacity {
            return Ok(Err(wit_types::BufferError::CapacityExceeded));
        }
        if offset > buf.data.len() as u64 {
            return Ok(Err(wit_types::BufferError::OutOfBounds));
        }
        let off = offset as usize;
        let end = off + bytes.len();
        if end > buf.data.len() {
            buf.data.resize(end, 0);
        }
        buf.data[off..end].copy_from_slice(&bytes);
        Ok(Ok(()))
    }

    async fn append(
        &mut self,
        h: Resource<HostMutableBuffer>,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), wit_types::BufferError>> {
        let buf = self.table.get_mut(&h)?;
        if (buf.data.len() as u64).saturating_add(bytes.len() as u64) > buf.capacity {
            return Ok(Err(wit_types::BufferError::CapacityExceeded));
        }
        buf.data.extend_from_slice(&bytes);
        Ok(Ok(()))
    }

    async fn size(&mut self, h: Resource<HostMutableBuffer>) -> wasmtime::Result<u64> {
        let buf = self.table.get(&h)?;
        Ok(buf.data.len() as u64)
    }

    async fn capacity(&mut self, h: Resource<HostMutableBuffer>) -> wasmtime::Result<u64> {
        let buf = self.table.get(&h)?;
        Ok(buf.capacity)
    }

    async fn set_content_type(
        &mut self,
        h: Resource<HostMutableBuffer>,
        content_type: String,
    ) -> wasmtime::Result<()> {
        let buf = self.table.get_mut(&h)?;
        buf.content_type = content_type;
        Ok(())
    }

    async fn freeze(
        &mut self,
        h: Resource<HostMutableBuffer>,
    ) -> wasmtime::Result<Resource<HostBuffer>> {
        let mb = self.table.delete(h)?;
        let frozen = HostBuffer {
            data: mb.data,
            content_type: mb.content_type,
        };
        let handle = self.table.push(frozen)?;
        Ok(handle)
    }

    async fn drop(&mut self, h: Resource<HostMutableBuffer>) -> wasmtime::Result<()> {
        self.table.delete(h)?;
        Ok(())
    }
}

impl wit_types::HostFlowContext for HostState {
    async fn trace_id(&mut self, _h: Resource<HostFlowContext>) -> wasmtime::Result<String> {
        Ok(String::new())
    }

    async fn span_id(&mut self, _h: Resource<HostFlowContext>) -> wasmtime::Result<String> {
        Ok(String::new())
    }

    async fn deadline_ns(&mut self, _h: Resource<HostFlowContext>) -> wasmtime::Result<u64> {
        Ok(0)
    }

    async fn flow_id(&mut self, _h: Resource<HostFlowContext>) -> wasmtime::Result<String> {
        Ok(String::new())
    }

    async fn drop(&mut self, h: Resource<HostFlowContext>) -> wasmtime::Result<()> {
        self.table.delete(h)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// buffer-allocator::Host — the imported allocator interface
// ---------------------------------------------------------------------------

impl wit_alloc::Host for HostState {
    async fn allocate(
        &mut self,
        capacity_hint: u64,
    ) -> wasmtime::Result<Result<Resource<HostMutableBuffer>, wit_types::BufferError>> {
        if capacity_hint > MAX_BUFFER_CAPACITY {
            return Ok(Err(wit_types::BufferError::CapacityExceeded));
        }
        let mb = HostMutableBuffer {
            data: Vec::new(),
            capacity: capacity_hint,
            content_type: String::new(),
        };
        let handle = self
            .table
            .push(mb)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        Ok(Ok(handle))
    }

    async fn clone_into_mutable(
        &mut self,
        source: Resource<HostBuffer>,
    ) -> wasmtime::Result<Result<Resource<HostMutableBuffer>, wit_types::BufferError>> {
        let src = self.table.get(&source)?;
        let data = src.data.clone();
        let content_type = src.content_type.clone();
        let mb = HostMutableBuffer {
            capacity: data.len() as u64,
            data,
            content_type,
        };
        let handle = self
            .table
            .push(mb)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        Ok(Ok(handle))
    }
}
