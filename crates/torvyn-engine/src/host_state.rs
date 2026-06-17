//! Host-side state stored in each Wasmtime `Store<HostState>`.
//!
//! `HostState` carries Torvyn-specific context that the bindgen-generated
//! host trait implementations access via `caller.data_mut()`. The three
//! resource types — [`HostBuffer`], [`HostMutableBuffer`], [`HostFlowContext`]
//! — are the Rust counterparts of the WIT resources defined in
//! `torvyn:streaming/types`, plumbed in via the `bindgen!` `with:` redirect
//! in [`crate::wit_bindings`].
//!
//! # Session 2.2: wired to `DefaultResourceManager`
//!
//! Buffer payload bytes live exclusively in the shared
//! [`torvyn_resources::DefaultResourceManager`]; the per-store
//! `wasmtime::component::ResourceTable` is a thin handle-indirection table
//! mapping `Resource<HostBuffer>` (a per-store integer) to a
//! [`torvyn_types::BufferHandle`] (manager-wide identity).
//!
//! Every guest call that crosses the boundary with bytes — `buffer.read`,
//! `mutable-buffer.write`, `mutable-buffer.append` — funnels through
//! `DefaultResourceManager::{read_payload, write_payload}`, which record
//! the copy in the `CopyLedger`.

use std::sync::Arc;

use wasmtime::component::{Resource, ResourceTable};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use torvyn_resources::{DefaultResourceManager, OwnerId};
use torvyn_security::WasiConfiguration;
use torvyn_types::{BufferHandle, ComponentId, FlowId, ResourceId};

/// Sentinel value that the `mutable-buffer.freeze` impl writes into the
/// retired [`HostMutableBuffer`] wrapper so its eventual host-side
/// `drop` callback knows the manager-wide buffer has already moved to
/// the new [`HostBuffer`] wrapper and must not be double-released.
const FROZEN_SENTINEL: BufferHandle = BufferHandle::new(ResourceId::new(u32::MAX, u32::MAX));

use crate::wit_bindings::data_sink::torvyn::streaming::types as wit_types;
use crate::wit_bindings::data_source::torvyn::streaming::buffer_allocator as wit_alloc;

/// Maximum capacity for a single mutable buffer (16 MiB), per `types.wit`
/// note C01-3 ("internal buffer capacity is capped at 16 MiB").
const MAX_BUFFER_CAPACITY: u64 = 16 * 1024 * 1024;

/// Host state stored in each Wasmtime `Store`.
///
/// This is the `T` in `Store<T>`. It holds Torvyn-specific context: the
/// component identity, store-level resource limits, the fuel budget that
/// was applied at instantiation, a [`ResourceTable`] mapping per-store
/// `Resource<T>` handles to host wrapper structs, an `Arc` to the shared
/// [`DefaultResourceManager`] that owns the actual buffer bytes, and a
/// [`FlowId`] used as the accounting key for every manager call originating
/// from this instance.
///
/// The struct is single-owner: only the owning `Store<HostState>` ever
/// holds a `&mut HostState`. The manager's interior `Mutex` is the sole
/// synchronisation point on the hot path.
pub(crate) struct HostState {
    /// The component ID for this instance.
    pub(crate) component_id: ComponentId,

    /// Resource limits for this store.
    pub(crate) limits: wasmtime::StoreLimits,

    /// The fuel budget configured for this component. Tracked for
    /// observability/diagnostics.
    #[allow(dead_code)]
    pub(crate) fuel_budget: u64,

    /// Resource table shared by Torvyn host resources
    /// (`buffer`, `mutable-buffer`, `flow-context`) and the
    /// `wasmtime-wasi` Preview-2 resource types (`input-stream`,
    /// `output-stream`, `error`, etc.). Per-store: each
    /// `Resource<T>` is meaningful only inside this store.
    pub(crate) table: ResourceTable,

    /// Shared manager that owns the buffer pool, ownership state machine,
    /// and copy ledger. Cloned by `Arc::clone` from the engine; this is
    /// the only place the manager pointer lives during the lifetime of
    /// the store.
    pub(crate) resources: Arc<DefaultResourceManager>,

    /// Flow identifier used as the accounting key for every manager call
    /// originating from this instance. For Session 2.2 this is derived
    /// from the component ID; Session 2.3 wires real reactor-assigned
    /// flow identifiers.
    pub(crate) flow_id: FlowId,

    /// WASI Preview-2 sandbox context, built from the component's resolved
    /// [`WasiConfiguration`] by [`build_wasi_ctx`].
    ///
    /// Guest components produced by `cargo-component` / TinyGo /
    /// `componentize-py` pull in WASI imports through their language runtimes
    /// even when the guest code itself never performs I/O. The host satisfies
    /// those imports through this context, which grants only what the
    /// component's capabilities allow — a deny-all configuration yields no
    /// filesystem preopens, no environment, discarded stdio, and no sockets.
    ///
    /// The engine links the full WASI Preview-2 surface into the linker
    /// ([`crate::wasmtime_engine`]), so access is gated here, by the context.
    /// Filesystem, environment, stdio, and network are gated precisely.
    /// `wasi:clocks` and `wasi:random` are always provided by `wasmtime-wasi`
    /// once linked and therefore cannot be denied at the context level; gating
    /// them would require selectively omitting their interfaces from the
    /// linker. `wasi:http` is not wired (it needs the `wasmtime-wasi-http`
    /// integration).
    pub(crate) wasi: WasiCtx,
}

/// Build a WASI Preview-2 context from a resolved [`WasiConfiguration`].
///
/// Applies the capabilities `wasmtime-wasi` can gate at the context level:
/// filesystem preopens, environment, stdout/stderr, and TCP/UDP sockets. A
/// deny-all configuration produces the most restrictive context
/// `wasmtime-wasi` ships (an empty [`WasiCtxBuilder`]).
///
/// See the [`HostState::wasi`] field docs for the capabilities that cannot be
/// gated here (clocks, random, http).
///
/// # COLD PATH — called once per `Store<HostState>` creation.
///
/// # Errors
/// Returns a human-readable reason if a granted directory cannot be preopened
/// (e.g. it does not exist or is not accessible). The caller wraps this in
/// [`EngineError::WasiConfigError`](crate::error::EngineError::WasiConfigError).
pub(crate) fn build_wasi_ctx(wasi: &WasiConfiguration) -> Result<WasiCtx, String> {
    let mut builder = WasiCtxBuilder::new();

    if wasi.allow_stdout {
        builder.inherit_stdout();
    }
    if wasi.allow_stderr {
        builder.inherit_stderr();
    }
    if wasi.allow_environment {
        builder.inherit_env();
    }

    for dir in &wasi.preopened_dirs {
        let mut dir_perms = DirPerms::empty();
        let mut file_perms = FilePerms::empty();
        if dir.read {
            dir_perms |= DirPerms::READ;
            file_perms |= FilePerms::READ;
        }
        if dir.write {
            dir_perms |= DirPerms::MUTATE;
            file_perms |= FilePerms::WRITE;
        }
        builder
            .preopened_dir(&dir.host_path, &dir.guest_path, dir_perms, file_perms)
            .map_err(|e| {
                format!(
                    "failed to preopen granted directory '{}': {e}",
                    dir.host_path
                )
            })?;
    }

    // Network: `wasmtime-wasi` denies all socket addresses unless a check is
    // installed, so granting any network capability opens the address space
    // (`inherit_network`) and enables the requested socket families. Per-host
    // address restriction from the grant is a future refinement.
    let tcp = wasi.allow_tcp_connect || wasi.allow_tcp_listen;
    if tcp || wasi.allow_udp {
        builder.inherit_network();
        builder.allow_tcp(tcp);
        builder.allow_udp(wasi.allow_udp);
    }

    Ok(builder.build())
}

/// A fully-sandboxed (deny-all) WASI context, for tests that construct a
/// [`HostState`] directly without going through the engine.
#[cfg(test)]
pub(crate) fn deny_all_wasi_ctx() -> WasiCtx {
    build_wasi_ctx(&WasiConfiguration::deny_all()).expect("deny-all WASI context is infallible")
}

impl HostState {
    /// Construct a new host state, register the flow and the component
    /// with the manager, and return the populated state.
    ///
    /// The `wasi` context is built by [`build_wasi_ctx`] from the component's
    /// resolved [`WasiConfiguration`] and passed in by the caller.
    ///
    /// # COLD PATH — called once per `Store<HostState>` creation.
    pub(crate) fn new(
        component_id: ComponentId,
        limits: wasmtime::StoreLimits,
        fuel_budget: u64,
        resources: Arc<DefaultResourceManager>,
        flow_id: FlowId,
        wasi: WasiCtx,
    ) -> Self {
        resources.register_flow(flow_id);
        resources.register_component(component_id, None);
        Self {
            component_id,
            limits,
            fuel_budget,
            table: ResourceTable::new(),
            resources,
            flow_id,
            wasi,
        }
    }

    /// The [`OwnerId`] used when this instance is the principal of a
    /// manager call (allocate / write / read / drop).
    #[inline]
    pub(crate) fn component_owner(&self) -> OwnerId {
        OwnerId::Component(self.component_id)
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl Drop for HostState {
    /// Phase-0 placeholder: no manager-side cleanup on `HostState` drop.
    ///
    /// `DefaultResourceManager::release_flow_resources` would return any
    /// outstanding buffer entries to the pool but it also *removes the
    /// flow's entry from the [`CopyLedger`]*, making post-mortem
    /// copy-accounting queries return zeros. Until the resource manager
    /// grows a "release pool but retain ledger" mode (Session 2.4+), the
    /// engine does not invoke that cleanup automatically. Buffers stay
    /// allocated for the lifetime of the `Arc<DefaultResourceManager>`,
    /// which in practice is the lifetime of the engine; everything is
    /// reclaimed when the manager itself drops.
    ///
    /// Production reactor wiring (which has access to the manager) will
    /// invoke `release_flow_resources` explicitly on terminal-flow
    /// events so the stat snapshot is sampled first.
    fn drop(&mut self) {
        // Intentionally empty — see method-level comment.
    }
}

/// Host-side handle for a WIT `buffer` resource. Contains only the
/// manager-wide [`BufferHandle`] and the last-known owner; the bytes
/// live inside [`DefaultResourceManager`].
///
/// `pub` only because the bindgen `with:` re-export chain needs to publicly
/// reference this type; the module itself is `#[doc(hidden)]` so it does not
/// form part of the crate's public API.
pub struct HostBuffer {
    pub(crate) handle: BufferHandle,
    pub(crate) owner: OwnerId,
}

/// Host-side handle for a WIT `mutable-buffer` resource. See [`HostBuffer`]
/// for the `pub` rationale.
pub struct HostMutableBuffer {
    pub(crate) handle: BufferHandle,
    pub(crate) owner: OwnerId,
}

/// Host-side data for a WIT `flow-context` resource. See [`HostBuffer`] for
/// the `pub` rationale.
///
/// Session 2.2 stub: empty. Session 2.3 carries trace/span identifiers and
/// the deadline propagated from the reactor.
pub struct HostFlowContext;

// ---------------------------------------------------------------------------
// types::Host — the imported `types` interface
// ---------------------------------------------------------------------------

impl wit_types::Host for HostState {}

impl wit_types::HostBuffer for HostState {
    async fn size(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<u64> {
        let buf = self.table.get(&h)?;
        let len = self.resources.payload_len(buf.handle).map_err(map_err)?;
        Ok(len as u64)
    }

    async fn content_type(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<String> {
        let buf = self.table.get(&h)?;
        self.resources.content_type(buf.handle).map_err(map_err)
    }

    async fn read(
        &mut self,
        h: Resource<HostBuffer>,
        offset: u64,
        len: u64,
    ) -> wasmtime::Result<Vec<u8>> {
        let buf = self.table.get(&h)?;
        let handle = buf.handle;
        let owner = buf.owner;
        let total = self.resources.payload_len(handle).map_err(map_err)?;
        let (offset, len) = clamp_read_range(offset, len, total);
        if len == 0 {
            return Ok(Vec::new());
        }
        self.resources
            .read_payload(handle, owner, offset, len, self.flow_id)
            .map_err(map_err)
    }

    async fn read_all(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<Vec<u8>> {
        let buf = self.table.get(&h)?;
        let handle = buf.handle;
        let owner = buf.owner;
        let total = self.resources.payload_len(handle).map_err(map_err)?;
        if total == 0 {
            return Ok(Vec::new());
        }
        self.resources
            .read_payload(handle, owner, 0, total, self.flow_id)
            .map_err(map_err)
    }

    async fn drop(&mut self, h: Resource<HostBuffer>) -> wasmtime::Result<()> {
        // wasmtime fires this when the guest drops an owned `buffer`
        // handle without returning it. Release the underlying manager
        // entry — unless the wrapper points at the freeze sentinel (the
        // manager side is already owned by the corresponding mutable
        // wrapper that's still in the table; nothing to do here).
        let buf: HostBuffer = self.table.delete(h)?;
        if buf.handle == FROZEN_SENTINEL {
            return Ok(());
        }
        self.resources
            .release(buf.handle, buf.owner)
            .map_err(map_err)
    }
}

impl wit_types::HostMutableBuffer for HostState {
    async fn write(
        &mut self,
        h: Resource<HostMutableBuffer>,
        offset: u64,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), wit_types::BufferError>> {
        let buf = self.table.get(&h)?;
        let handle = buf.handle;
        let owner = buf.owner;
        let off = match u32::try_from(offset) {
            Ok(v) => v,
            Err(_) => return Ok(Err(wit_types::BufferError::OutOfBounds)),
        };
        match self
            .resources
            .write_payload(handle, owner, off, &bytes, self.flow_id)
        {
            Ok(()) => Ok(Ok(())),
            Err(e) => Ok(Err(buffer_error_from(e))),
        }
    }

    async fn append(
        &mut self,
        h: Resource<HostMutableBuffer>,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), wit_types::BufferError>> {
        let buf = self.table.get(&h)?;
        let handle = buf.handle;
        let owner = buf.owner;
        let len = self.resources.payload_len(handle).map_err(map_err)?;
        match self
            .resources
            .write_payload(handle, owner, len, &bytes, self.flow_id)
        {
            Ok(()) => Ok(Ok(())),
            Err(e) => Ok(Err(buffer_error_from(e))),
        }
    }

    async fn size(&mut self, h: Resource<HostMutableBuffer>) -> wasmtime::Result<u64> {
        let buf = self.table.get(&h)?;
        let len = self.resources.payload_len(buf.handle).map_err(map_err)?;
        Ok(len as u64)
    }

    async fn capacity(&mut self, h: Resource<HostMutableBuffer>) -> wasmtime::Result<u64> {
        let buf = self.table.get(&h)?;
        let info = self.resources.inspect(buf.handle).map_err(map_err)?;
        Ok(info.payload_capacity as u64)
    }

    async fn set_content_type(
        &mut self,
        h: Resource<HostMutableBuffer>,
        content_type: String,
    ) -> wasmtime::Result<()> {
        let buf = self.table.get(&h)?;
        let handle = buf.handle;
        let owner = buf.owner;
        self.resources
            .set_content_type(handle, owner, &content_type)
            .map_err(map_err)
    }

    async fn freeze(
        &mut self,
        h: Resource<HostMutableBuffer>,
    ) -> wasmtime::Result<Resource<HostBuffer>> {
        // `freeze` is a host-side relabel: the manager owns the bytes and
        // the WIT semantics ("mutable-buffer handle is consumed; an
        // immutable buffer handle is returned, ownership transfers to the
        // caller") map to "drop the mutable wrapper from the per-store
        // table, push an immutable wrapper that points at the same
        // manager-wide `BufferHandle`". The manager's state machine sees
        // no transition — the buffer remains owned by the component.
        //
        // wit-bindgen 0.41 generates `freeze` with a *borrowed* `self_`
        // (resource methods default to borrow). We cannot `delete` a
        // borrowed handle, so instead:
        //   1. Read the manager-wide handle + owner via `get`.
        //   2. Mark the mutable entry as "frozen" by zeroing its handle —
        //      the eventual host-side `drop` callback then skips the
        //      manager release. (Manager ownership has already passed to
        //      the new `HostBuffer` wrapper.)
        //   3. Push a fresh `HostBuffer` wrapper and return that.
        let (handle, owner) = {
            let mb = self.table.get_mut(&h)?;
            let result = (mb.handle, mb.owner);
            mb.handle = FROZEN_SENTINEL;
            result
        };
        let frozen = HostBuffer { handle, owner };
        let resource = self
            .table
            .push(frozen)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        Ok(resource)
    }

    async fn drop(&mut self, h: Resource<HostMutableBuffer>) -> wasmtime::Result<()> {
        // wasmtime fires this when the guest drops an owned
        // `mutable-buffer` handle. The wrapper points at the freeze
        // sentinel when `freeze` already handed manager ownership off to
        // a new immutable wrapper; in that case there is nothing to
        // release here.
        let buf: HostMutableBuffer = self.table.delete(h)?;
        if buf.handle == FROZEN_SENTINEL {
            return Ok(());
        }
        self.resources
            .release(buf.handle, buf.owner)
            .map_err(map_err)
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
        // SAFETY of cast: `capacity_hint <= MAX_BUFFER_CAPACITY` (16 MiB),
        // which fits in u32.
        let min_capacity = capacity_hint as u32;
        let owner = self.component_owner();
        let handle = match self.resources.allocate(owner, min_capacity, self.flow_id) {
            Ok(h) => h,
            Err(e) => return Ok(Err(buffer_error_from(e))),
        };
        let wrapper = HostMutableBuffer { handle, owner };
        let resource = self
            .table
            .push(wrapper)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        Ok(Ok(resource))
    }

    async fn clone_into_mutable(
        &mut self,
        source: Resource<HostBuffer>,
    ) -> wasmtime::Result<Result<Resource<HostMutableBuffer>, wit_types::BufferError>> {
        let src = self.table.get(&source)?;
        let src_handle = src.handle;
        let src_owner = src.owner;
        let owner = self.component_owner();

        // Read the source bytes (records HostToComponent copy event).
        let total = self.resources.payload_len(src_handle).map_err(map_err)?;
        let bytes = if total == 0 {
            Vec::new()
        } else {
            self.resources
                .read_payload(src_handle, src_owner, 0, total, self.flow_id)
                .map_err(map_err)?
        };

        // Allocate the destination.
        let dst_handle = match self.resources.allocate(owner, total.max(1), self.flow_id) {
            Ok(h) => h,
            Err(e) => return Ok(Err(buffer_error_from(e))),
        };

        // Copy in (records ComponentToHost copy event).
        if !bytes.is_empty() {
            if let Err(e) = self
                .resources
                .write_payload(dst_handle, owner, 0, &bytes, self.flow_id)
            {
                // Roll back the allocation on failure.
                let _ = self.resources.release(dst_handle, owner);
                return Ok(Err(buffer_error_from(e)));
            }
        }

        let wrapper = HostMutableBuffer {
            handle: dst_handle,
            owner,
        };
        let resource = self
            .table
            .push(wrapper)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        Ok(Ok(resource))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Clamp `(offset, len)` (both expressed in WIT-level u64) into a u32 pair
/// safe for the manager. Out-of-range offsets yield zero-length reads, in
/// line with the WIT note: "Returns fewer bytes if the buffer is shorter
/// than offset + len".
#[inline]
fn clamp_read_range(offset: u64, len: u64, payload_len: u32) -> (u32, u32) {
    let total = u64::from(payload_len);
    if offset >= total {
        return (0, 0);
    }
    let remaining = total - offset;
    let clamped_len = len.min(remaining);
    // Safe: offset < total <= u32::MAX after manager caps capacity at 16 MiB.
    (offset as u32, clamped_len as u32)
}

/// Surface a manager error as a wasmtime trap. This is the bindgen
/// `trappable_imports` convention — recoverable WIT-level errors are
/// returned as `Ok(Err(BufferError))` instead.
#[inline]
fn map_err(err: torvyn_types::ResourceError) -> wasmtime::Error {
    wasmtime::Error::msg(err.to_string())
}

/// Map a [`torvyn_types::ResourceError`] to the WIT `buffer-error` variant
/// the guest expects.
#[inline]
fn buffer_error_from(err: torvyn_types::ResourceError) -> wit_types::BufferError {
    use torvyn_types::ResourceError;
    match err {
        ResourceError::CapacityExceeded { .. } | ResourceError::BudgetExceeded { .. } => {
            wit_types::BufferError::CapacityExceeded
        }
        ResourceError::OutOfBounds { .. } => wit_types::BufferError::OutOfBounds,
        other => wit_types::BufferError::AllocationFailed(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_security::PreopenedDir;
    use torvyn_types::ResourceState;
    use wit_alloc::Host as _;

    #[test]
    fn build_wasi_ctx_deny_all_is_infallible() {
        // The most restrictive configuration must always build.
        assert!(build_wasi_ctx(&WasiConfiguration::deny_all()).is_ok());
    }

    #[test]
    fn build_wasi_ctx_applies_non_filesystem_grants() {
        // Environment, stdio, and socket grants are applied with no I/O and
        // therefore never fail to build.
        let mut wasi = WasiConfiguration::deny_all();
        wasi.allow_environment = true;
        wasi.allow_stdout = true;
        wasi.allow_stderr = true;
        wasi.allow_tcp_connect = true;
        wasi.allow_tcp_listen = true;
        wasi.allow_udp = true;
        assert!(build_wasi_ctx(&wasi).is_ok());
    }

    #[test]
    fn build_wasi_ctx_preopens_existing_directory() {
        // A granted directory that exists is preopened successfully — the
        // positive half of filesystem capability enforcement.
        let tmp = std::env::temp_dir();
        let mut wasi = WasiConfiguration::deny_all();
        wasi.preopened_dirs.push(PreopenedDir {
            host_path: tmp.to_string_lossy().into_owned(),
            guest_path: "/sandbox".to_owned(),
            read: true,
            write: false,
        });
        assert!(
            build_wasi_ctx(&wasi).is_ok(),
            "preopen of an existing directory must succeed"
        );
    }

    #[test]
    fn build_wasi_ctx_missing_preopen_directory_errors() {
        // A granted directory that does not exist surfaces a descriptive
        // error (which `create_store` maps to `EngineError::WasiConfigError`),
        // rather than silently dropping the grant.
        let mut wasi = WasiConfiguration::deny_all();
        wasi.preopened_dirs.push(PreopenedDir {
            host_path: "/torvyn-nonexistent/never/exists".to_owned(),
            guest_path: "/data".to_owned(),
            read: true,
            write: false,
        });
        // `WasiCtx` is not `Debug`, so match rather than `expect_err`.
        let err = match build_wasi_ctx(&wasi) {
            Ok(_) => panic!("a missing preopen directory must error"),
            Err(e) => e,
        };
        assert!(
            err.contains("preopen"),
            "error should describe the failed preopen, got: {err}"
        );
    }

    fn test_state(component_id: u64, flow_id: u64) -> HostState {
        let resources = Arc::new(DefaultResourceManager::new_for_testing());
        HostState::new(
            ComponentId::new(component_id),
            wasmtime::StoreLimitsBuilder::new().build(),
            1_000_000,
            resources,
            FlowId::new(flow_id),
            deny_all_wasi_ctx(),
        )
    }

    #[tokio::test]
    async fn host_state_allocate_pushes_to_manager() {
        let mut state = test_state(7, 7);
        let resources = Arc::clone(&state.resources);

        let result = state.allocate(256).await.expect("trap-free");
        let resource = result.expect("allocator success");

        assert_eq!(resources.live_resource_count(), 1);
        let mb = state.table.get(&resource).expect("entry present");
        assert_eq!(mb.owner, OwnerId::Component(ComponentId::new(7)));

        let inspect = resources.inspect(mb.handle).expect("inspect");
        assert_eq!(inspect.owner, OwnerId::Component(ComponentId::new(7)));
        assert_eq!(inspect.state, ResourceState::Owned);
    }

    #[tokio::test]
    async fn host_state_write_records_copy_event() {
        let mut state = test_state(7, 7);
        let resources = Arc::clone(&state.resources);
        let flow = state.flow_id;

        let resource = state
            .allocate(256)
            .await
            .expect("trap-free")
            .expect("allocator success");
        let bytes = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let inner =
            <HostState as wit_types::HostMutableBuffer>::write(&mut state, resource, 0, bytes)
                .await
                .expect("trap-free");
        assert!(inner.is_ok());

        let stats = resources.flow_copy_stats(flow);
        assert_eq!(stats.total_copy_ops, 1);
        assert_eq!(stats.total_payload_bytes, 16);
    }

    #[tokio::test]
    async fn host_state_freeze_is_relabel() {
        let mut state = test_state(7, 7);
        let resources = Arc::clone(&state.resources);
        let flow = state.flow_id;
        let cid = state.component_id;

        let mb_resource = state
            .allocate(256)
            .await
            .expect("trap-free")
            .expect("allocator success");
        let mb_handle = state.table.get(&mb_resource).expect("entry present").handle;

        <HostState as wit_types::HostMutableBuffer>::write(
            &mut state,
            Resource::new_own(mb_resource.rep()),
            0,
            vec![0xAA; 16],
        )
        .await
        .expect("trap-free")
        .expect("write success");

        let immutable = <HostState as wit_types::HostMutableBuffer>::freeze(
            &mut state,
            Resource::new_own(mb_resource.rep()),
        )
        .await
        .expect("trap-free");

        let entry = state.table.get(&immutable).expect("immutable entry");
        assert_eq!(
            entry.handle, mb_handle,
            "freeze preserves the manager-wide BufferHandle"
        );
        assert_eq!(
            entry.owner,
            OwnerId::Component(cid),
            "freeze does not transfer ownership"
        );
        assert_eq!(resources.live_resource_count(), 1);

        let stats = resources.flow_copy_stats(flow);
        assert_eq!(
            stats.total_copy_ops, 1,
            "freeze itself records no copy event"
        );
    }

    #[tokio::test]
    async fn host_state_read_records_copy_event() {
        let mut state = test_state(7, 7);
        let resources = Arc::clone(&state.resources);
        let flow = state.flow_id;

        let mb_resource = state
            .allocate(256)
            .await
            .expect("trap-free")
            .expect("allocator success");
        <HostState as wit_types::HostMutableBuffer>::write(
            &mut state,
            Resource::new_own(mb_resource.rep()),
            0,
            vec![0xCC; 16],
        )
        .await
        .expect("trap-free")
        .expect("write success");

        let immutable = <HostState as wit_types::HostMutableBuffer>::freeze(
            &mut state,
            Resource::new_own(mb_resource.rep()),
        )
        .await
        .expect("trap-free");

        let read_back = <HostState as wit_types::HostBuffer>::read_all(
            &mut state,
            Resource::new_own(immutable.rep()),
        )
        .await
        .expect("trap-free");
        assert_eq!(read_back, vec![0xCC; 16]);

        let stats = resources.flow_copy_stats(flow);
        assert_eq!(stats.total_copy_ops, 2);
        assert_eq!(stats.total_payload_bytes, 32);
    }

    #[tokio::test]
    async fn host_state_drop_releases_to_pool() {
        let mut state = test_state(7, 7);
        let resources = Arc::clone(&state.resources);

        let mb_resource = state
            .allocate(256)
            .await
            .expect("trap-free")
            .expect("allocator success");
        assert_eq!(resources.live_resource_count(), 1);

        <HostState as wit_types::HostMutableBuffer>::drop(&mut state, mb_resource)
            .await
            .expect("trap-free");

        assert_eq!(resources.live_resource_count(), 0);
    }

    #[tokio::test]
    async fn host_state_drop_is_a_phase0_noop_preserving_ledger() {
        // Pinning the Phase-0 behaviour: dropping a `HostState` does NOT
        // automatically return its outstanding buffers to the pool or
        // wipe its `CopyLedger` entry. That intentionally lets
        // end-to-end tests sample the post-flow copy stats. Production
        // wiring (a future session) will call
        // `release_flow_resources` explicitly on terminal-flow events
        // after sampling.
        let resources = Arc::new(DefaultResourceManager::new_for_testing());
        let flow = FlowId::new(7);
        {
            let mut state = HostState::new(
                ComponentId::new(7),
                wasmtime::StoreLimitsBuilder::new().build(),
                1_000_000,
                Arc::clone(&resources),
                flow,
                deny_all_wasi_ctx(),
            );
            let _ = state.allocate(256).await.expect("trap-free").expect("ok");
            let _ = state.allocate(256).await.expect("trap-free").expect("ok");
            assert_eq!(resources.live_resource_count(), 2);
            // state goes out of scope here, invoking `Drop for HostState`,
            // which is intentionally a no-op for Phase 0.
        }
        assert_eq!(
            resources.live_resource_count(),
            2,
            "Phase-0 Drop must leave buffers allocated for post-mortem inspection"
        );
        // The same goes for the ledger entry — explicit teardown is
        // required to reclaim it.
        resources
            .release_flow_resources(flow)
            .expect("explicit release must succeed");
        assert_eq!(resources.live_resource_count(), 0);
    }

    #[tokio::test]
    async fn host_state_allocate_capacity_exceeded() {
        let mut state = test_state(7, 7);
        let resources = Arc::clone(&state.resources);

        let outcome = state
            .allocate(MAX_BUFFER_CAPACITY + 1)
            .await
            .expect("trap-free");
        assert!(matches!(
            outcome,
            Err(wit_types::BufferError::CapacityExceeded)
        ));
        assert_eq!(resources.live_resource_count(), 0);
    }
}
