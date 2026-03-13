//! DefaultResourceManager: the unified resource lifecycle API.
//!
//! Per Doc 03 §12.2 with changes from Doc 10 (C03-4: `&self`, C03-5: flow cleanup,
//! C03-6: EventSink instead of MetricsSink).
//!
//! All public methods take `&self` and use interior mutability via a Mutex
//! on the resource table. The buffer pool and copy ledger are also internally
//! synchronized.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;

use torvyn_types::{
    BufferHandle, ComponentId, CopyReason, EventSink, FlowId, NoopEventSink, ResourceId,
    ResourceState,
};

use crate::accounting::{CopyLedger, FlowCopyStatsSnapshot};
use crate::budget::BudgetRegistry;
use crate::handle::{
    BufferFlags, ContentType, FlowResourceStats, OwnerId, PoolTier, ResourceEntry,
    ResourceReclaimed,
};
use crate::pool::{BufferPoolSet, TierConfig};
use crate::table::ResourceTable;
use crate::{buffer, error, ownership};

/// Configuration for the resource manager.
#[derive(Clone, Debug)]
pub struct ResourceManagerConfig {
    /// Initial capacity of the resource table.
    pub table_capacity: u32,
    /// Pool tier configurations.
    pub pool_configs: [TierConfig; 4],
    /// Default per-component memory budget in bytes.
    pub default_budget_bytes: u64,
}

impl Default for ResourceManagerConfig {
    fn default() -> Self {
        Self {
            table_capacity: crate::table::DEFAULT_INITIAL_CAPACITY,
            pool_configs: TierConfig::defaults(),
            default_budget_bytes: crate::budget::DEFAULT_MAX_OWNED_BYTES,
        }
    }
}

/// Internal mutable state protected by a Mutex.
struct Inner {
    table: ResourceTable,
    /// Reverse index: component → set of owned resource IDs.
    component_resources: HashMap<ComponentId, HashSet<ResourceId>>,
    /// Reverse index: flow → set of resource IDs allocated for that flow.
    flow_resources: HashMap<FlowId, HashSet<ResourceId>>,
}

/// The default resource manager implementation.
///
/// All methods take `&self` (per C03-4). Interior mutability is via `Mutex<Inner>`
/// for the resource table and reverse indices. The pool and budget registry
/// have their own internal synchronization.
///
/// # Thread Safety
/// Safe to share via `Arc<DefaultResourceManager>` across the reactor and
/// host runtime.
pub struct DefaultResourceManager {
    inner: Mutex<Inner>,
    pools: BufferPoolSet,
    budgets: BudgetRegistry,
    ledger: CopyLedger,
    event_sink: Arc<dyn EventSink>,
}

impl DefaultResourceManager {
    /// Create a new resource manager with the given configuration and event sink.
    ///
    /// # COLD PATH — called once during host startup.
    pub fn new(config: ResourceManagerConfig, event_sink: Arc<dyn EventSink>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                table: ResourceTable::new(config.table_capacity),
                component_resources: HashMap::new(),
                flow_resources: HashMap::new(),
            }),
            pools: BufferPoolSet::new(&config.pool_configs),
            budgets: BudgetRegistry::new(config.default_budget_bytes),
            ledger: CopyLedger::new(),
            event_sink,
        }
    }

    /// Create a resource manager with default configuration and no observability.
    ///
    /// Useful for testing.
    pub fn new_for_testing() -> Self {
        Self::new(
            ResourceManagerConfig::default(),
            Arc::new(NoopEventSink),
        )
    }

    // -----------------------------------------------------------------------
    // Allocation
    // -----------------------------------------------------------------------

    /// Allocate a new buffer of at least `min_capacity` bytes.
    ///
    /// The buffer is owned by `owner`. Budget checks are applied if
    /// the owner is a component.
    ///
    /// # WARM PATH — called per buffer allocation.
    pub fn allocate(
        &self,
        owner: OwnerId,
        min_capacity: u32,
        flow_id: FlowId,
    ) -> error::Result<BufferHandle> {
        // Validate capacity
        if min_capacity > buffer::MAX_BUFFER_SIZE {
            return Err(error::capacity_exceeded(
                BufferHandle::new(ResourceId::new(0, 0)),
                buffer::MAX_BUFFER_SIZE,
                min_capacity as u64,
            ));
        }

        // Budget check
        if let OwnerId::Component(cid) = owner {
            let tier = PoolTier::for_capacity(min_capacity);
            let actual_capacity = tier.capacity().max(min_capacity);
            self.budgets.try_reserve(cid, actual_capacity as u64)?;
        }

        // Allocate from pool (or fallback)
        let (ptr, alloc_size, tier, is_fallback) = self
            .pools
            .allocate(min_capacity)
            .ok_or_else(|| error::allocation_failed(min_capacity, "system allocator OOM"))?;

        let actual_capacity = tier.capacity().max(min_capacity);

        // Create the resource entry
        let entry = ResourceEntry {
            generation: 0, // overwritten by table.insert
            state: ResourceState::Owned,
            owner,
            borrow_count: 0,
            buffer_ptr: ptr,
            alloc_size,
            payload_capacity: actual_capacity,
            payload_len: 0,
            pool_tier: tier,
            flags: BufferFlags {
                is_fallback,
                read_only: false,
            },
            content_type: ContentType::empty(),
            flow_id,
            created_at_ns: torvyn_types::current_timestamp_ns(),
        };

        // Insert into the resource table
        let mut inner = self.inner.lock();
        let resource_id = match inner.table.insert(entry) {
            Ok(id) => id,
            Err(e) => {
                // Rollback: return buffer to pool and budget
                unsafe { self.pools.release(ptr, alloc_size, tier, is_fallback) };
                if let OwnerId::Component(cid) = owner {
                    self.budgets.release_bytes(cid, actual_capacity as u64);
                }
                return Err(e);
            }
        };

        // Update reverse indices
        if let OwnerId::Component(cid) = owner {
            inner
                .component_resources
                .entry(cid)
                .or_default()
                .insert(resource_id);
        }
        inner
            .flow_resources
            .entry(flow_id)
            .or_default()
            .insert(resource_id);

        Ok(BufferHandle::new(resource_id))
    }

    // -----------------------------------------------------------------------
    // Release
    // -----------------------------------------------------------------------

    /// Release a buffer back to the pool or deallocate it.
    ///
    /// # WARM PATH
    pub fn release(
        &self,
        handle: BufferHandle,
        caller: OwnerId,
    ) -> error::Result<()> {
        let mut inner = self.inner.lock();
        let entry = inner.table.get_mut(handle)?;

        // Validate ownership and state
        ownership::release_to_pool(handle, entry, caller)?;

        // Extract info for cleanup (before removing from table)
        let ptr = entry.buffer_ptr;
        let alloc_size = entry.alloc_size;
        let tier = entry.pool_tier;
        let is_fallback = entry.flags.is_fallback;
        let capacity = entry.payload_capacity;
        let flow_id = entry.flow_id;
        let resource_id = handle.resource_id();

        // Remove from table
        let _removed = inner.table.remove(handle)?;

        // Update reverse indices
        if let OwnerId::Component(cid) = caller {
            if let Some(set) = inner.component_resources.get_mut(&cid) {
                set.remove(&resource_id);
            }
            // Release budget
            self.budgets.release_bytes(cid, capacity as u64);
        }
        if let Some(set) = inner.flow_resources.get_mut(&flow_id) {
            set.remove(&resource_id);
        }

        // Drop the lock before pool operations
        drop(inner);

        // Zero the payload and return to pool
        unsafe {
            buffer::zero_payload(ptr, capacity);
            self.pools.release(ptr, alloc_size, tier, is_fallback);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Ownership Transfer
    // -----------------------------------------------------------------------

    /// Transfer ownership of a buffer from one entity to another.
    ///
    /// # HOT PATH
    pub fn transfer_ownership(
        &self,
        handle: BufferHandle,
        from: OwnerId,
        to: OwnerId,
    ) -> error::Result<()> {
        // Budget check for recipient
        let mut inner = self.inner.lock();
        let entry = inner.table.get(handle)?;
        let capacity = entry.payload_capacity;

        if let OwnerId::Component(cid) = to {
            self.budgets.try_reserve(cid, capacity as u64)?;
        }

        let entry = inner.table.get_mut(handle)?;
        let resource_id = handle.resource_id();

        // Begin transfer (Owned → Transit)
        ownership::begin_transfer(handle, entry, from)?;
        // Complete transfer (Transit → Owned by `to`)
        ownership::complete_transfer(handle, entry, to)?;

        // Update reverse indices
        if let OwnerId::Component(old_cid) = from {
            if let Some(set) = inner.component_resources.get_mut(&old_cid) {
                set.remove(&resource_id);
            }
            self.budgets.release_bytes(old_cid, capacity as u64);
        }
        if let OwnerId::Component(new_cid) = to {
            inner
                .component_resources
                .entry(new_cid)
                .or_default()
                .insert(resource_id);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Borrow Lifecycle
    // -----------------------------------------------------------------------

    /// Record the start of a borrow.
    ///
    /// # HOT PATH
    pub fn borrow_start(
        &self,
        handle: BufferHandle,
        borrower: ComponentId,
    ) -> error::Result<()> {
        let mut inner = self.inner.lock();
        let entry = inner.table.get_mut(handle)?;
        ownership::borrow_start(handle, entry, borrower)
    }

    /// Record the end of a borrow.
    ///
    /// # HOT PATH
    pub fn borrow_end(
        &self,
        handle: BufferHandle,
        borrower: ComponentId,
    ) -> error::Result<()> {
        let mut inner = self.inner.lock();
        let entry = inner.table.get_mut(handle)?;
        ownership::borrow_end(handle, entry, borrower)
    }

    // -----------------------------------------------------------------------
    // Payload Read / Write
    // -----------------------------------------------------------------------

    /// Read payload bytes from a buffer. Records a copy event.
    ///
    /// Returns a Vec<u8> containing the requested bytes. The copy from host
    /// memory into the Vec is recorded in the copy ledger.
    ///
    /// # HOT PATH — but involves a copy (unavoidable per Wasm memory model).
    pub fn read_payload(
        &self,
        handle: BufferHandle,
        _caller: OwnerId,
        offset: u32,
        length: u32,
        flow_id: FlowId,
    ) -> error::Result<Vec<u8>> {
        let inner = self.inner.lock();
        let entry = inner.table.get(handle)?;

        // Validate bounds
        let end = offset as u64 + length as u64;
        if end > entry.payload_len as u64 {
            return Err(error::out_of_bounds(
                handle,
                offset as u64,
                entry.payload_len as u64,
            ));
        }

        // SAFETY: entry.buffer_ptr is valid, and we've validated the bounds
        // against payload_len. The buffer is either Owned or Borrowed (read access).
        let data = unsafe { buffer::read_payload(entry.buffer_ptr, offset, length) };
        let result = data.to_vec();

        let resource_id = handle.resource_id();
        let from = entry.owner.component_id().unwrap_or(ComponentId::new(0));
        let to = ComponentId::new(0); // host is the intermediary

        drop(inner);

        // Record the copy
        self.ledger.record_copy(
            flow_id,
            resource_id,
            from,
            to,
            length as u64,
            CopyReason::HostToComponent,
            self.event_sink.as_ref(),
        );

        Ok(result)
    }

    /// Write payload bytes to a buffer. Caller must be the owner.
    ///
    /// # HOT PATH — but involves a copy.
    pub fn write_payload(
        &self,
        handle: BufferHandle,
        caller: OwnerId,
        offset: u32,
        data: &[u8],
        flow_id: FlowId,
    ) -> error::Result<()> {
        let mut inner = self.inner.lock();
        let entry = inner.table.get_mut(handle)?;

        // Verify ownership
        if entry.owner != caller {
            return Err(error::not_owner(
                handle,
                &format!("{}", entry.owner),
                &format!("{}", caller),
            ));
        }

        // Verify state (must be Owned, not Borrowed)
        if entry.state != ResourceState::Owned {
            return Err(error::borrows_outstanding(handle, entry.borrow_count));
        }

        // Validate bounds
        let end = offset as u64 + data.len() as u64;
        if end > entry.payload_capacity as u64 {
            return Err(error::capacity_exceeded(handle, entry.payload_capacity, end));
        }

        // SAFETY: We've verified ownership, state, and bounds.
        unsafe {
            buffer::write_payload(entry.buffer_ptr, offset, data);
        }

        // Update payload_len
        let new_end = offset + data.len() as u32;
        if new_end > entry.payload_len {
            entry.payload_len = new_end;
        }

        let resource_id = handle.resource_id();
        let from = ComponentId::new(0); // component writes via host
        let to = caller.component_id().unwrap_or(ComponentId::new(0));

        drop(inner);

        // Record the copy
        self.ledger.record_copy(
            flow_id,
            resource_id,
            from,
            to,
            data.len() as u64,
            CopyReason::ComponentToHost,
            self.event_sink.as_ref(),
        );

        Ok(())
    }

    /// Set the content type on a buffer.
    ///
    /// # WARM PATH
    pub fn set_content_type(
        &self,
        handle: BufferHandle,
        caller: OwnerId,
        content_type: &str,
    ) -> error::Result<()> {
        let mut inner = self.inner.lock();
        let entry = inner.table.get_mut(handle)?;

        if entry.owner != caller {
            return Err(error::not_owner(
                handle,
                &format!("{}", entry.owner),
                &format!("{}", caller),
            ));
        }

        entry.content_type = ContentType::from_str(content_type);
        Ok(())
    }

    /// Get the payload length of a buffer.
    ///
    /// # HOT PATH
    pub fn payload_len(&self, handle: BufferHandle) -> error::Result<u32> {
        let inner = self.inner.lock();
        let entry = inner.table.get(handle)?;
        Ok(entry.payload_len)
    }

    /// Get the content type of a buffer.
    ///
    /// # WARM PATH
    pub fn content_type(&self, handle: BufferHandle) -> error::Result<String> {
        let inner = self.inner.lock();
        let entry = inner.table.get(handle)?;
        Ok(entry.content_type.as_str().to_string())
    }

    // -----------------------------------------------------------------------
    // Diagnostics
    // -----------------------------------------------------------------------

    /// Query the current state of a resource.
    ///
    /// # COLD PATH — diagnostics.
    pub fn inspect(&self, handle: BufferHandle) -> error::Result<ResourceInspection> {
        let inner = self.inner.lock();
        let entry = inner.table.get(handle)?;
        Ok(ResourceInspection {
            resource_id: handle.resource_id(),
            state: entry.state,
            owner: entry.owner,
            borrow_count: entry.borrow_count,
            payload_capacity: entry.payload_capacity,
            payload_len: entry.payload_len,
            pool_tier: entry.pool_tier,
            is_fallback: entry.flags.is_fallback,
            content_type: entry.content_type.as_str().to_string(),
            flow_id: entry.flow_id,
        })
    }

    /// Enumerate all resources held by a component.
    ///
    /// # COLD PATH — diagnostics and cleanup.
    pub fn resources_held_by(&self, component: ComponentId) -> Vec<ResourceId> {
        let inner = self.inner.lock();
        inner
            .component_resources
            .get(&component)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Force-reclaim all resources held by a component (crash cleanup).
    ///
    /// Per Doc 03, Section 3.4: enumerate, release borrows, reclaim owned.
    ///
    /// # COLD PATH
    pub fn force_reclaim(&self, component: ComponentId) -> Vec<ResourceReclaimed> {
        let mut inner = self.inner.lock();
        let resource_ids: Vec<ResourceId> = inner
            .component_resources
            .remove(&component)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();

        let mut reclaimed = Vec::new();

        for rid in resource_ids {
            let handle = BufferHandle::new(rid);
            if let Ok(entry) = inner.table.get_mut(handle) {
                let prev_state = entry.state;
                let prev_owner = entry.owner;
                let capacity = entry.payload_capacity;
                let ptr = entry.buffer_ptr;
                let alloc_size = entry.alloc_size;
                let tier = entry.pool_tier;
                let is_fallback = entry.flags.is_fallback;
                let flow_id = entry.flow_id;

                // Clear borrows
                entry.borrow_count = 0;
                entry.state = ResourceState::Owned;
                entry.owner = OwnerId::Host;

                // Remove from table
                if inner.table.remove(handle).is_ok() {
                    // Remove from flow index
                    if let Some(set) = inner.flow_resources.get_mut(&flow_id) {
                        set.remove(&rid);
                    }

                    // Return to pool
                    unsafe {
                        buffer::zero_payload(ptr, capacity);
                        self.pools.release(ptr, alloc_size, tier, is_fallback);
                    }

                    reclaimed.push(ResourceReclaimed {
                        resource_id: rid,
                        previous_state: prev_state,
                        previous_owner: prev_owner,
                        payload_capacity: capacity,
                        returned_to_pool: !is_fallback,
                    });
                }
            }
        }

        // Release the component's budget
        self.budgets.unregister(component);

        reclaimed
    }

    /// Get copy stats for a flow.
    ///
    /// # COLD PATH — reporting.
    pub fn flow_copy_stats(&self, flow_id: FlowId) -> FlowCopyStatsSnapshot {
        self.ledger.flow_stats(flow_id)
    }

    /// Release all resources associated with a flow.
    ///
    /// Per C03-5: called by the reactor when a flow reaches terminal state.
    ///
    /// # COLD PATH
    pub fn release_flow_resources(&self, flow_id: FlowId) -> error::Result<FlowResourceStats> {
        let mut inner = self.inner.lock();
        let resource_ids: Vec<ResourceId> = inner
            .flow_resources
            .remove(&flow_id)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();

        let mut stats = FlowResourceStats::default();

        for rid in resource_ids {
            let handle = BufferHandle::new(rid);
            if let Ok(entry) = inner.table.get_mut(handle) {
                let capacity = entry.payload_capacity;
                let ptr = entry.buffer_ptr;
                let alloc_size = entry.alloc_size;
                let tier = entry.pool_tier;
                let is_fallback = entry.flags.is_fallback;

                // Clear any outstanding borrows
                if entry.borrow_count > 0 {
                    stats.borrows_cleared += entry.borrow_count;
                    entry.borrow_count = 0;
                }

                // Remove from component reverse index
                if let OwnerId::Component(cid) = entry.owner {
                    if let Some(set) = inner.component_resources.get_mut(&cid) {
                        set.remove(&rid);
                    }
                    self.budgets.release_bytes(cid, capacity as u64);
                }

                // Remove from table
                if inner.table.remove(handle).is_ok() {
                    unsafe {
                        buffer::zero_payload(ptr, capacity);
                        self.pools.release(ptr, alloc_size, tier, is_fallback);
                    }

                    if is_fallback {
                        stats.deallocated += 1;
                    } else {
                        stats.returned_to_pool += 1;
                    }
                    stats.total_bytes_released += capacity as u64;
                }
            }
        }

        // Clean up copy accounting for this flow
        let _ = self.ledger.remove_flow(flow_id);

        Ok(stats)
    }

    /// Register a flow for copy accounting.
    ///
    /// # COLD PATH
    pub fn register_flow(&self, flow_id: FlowId) {
        self.ledger.register_flow(flow_id);
    }

    /// Register a component with a memory budget.
    ///
    /// # COLD PATH
    pub fn register_component(&self, component: ComponentId, max_owned_bytes: Option<u64>) {
        self.budgets.register(component, max_owned_bytes);
    }

    /// Get pool metrics for a tier.
    pub fn pool_metrics(&self, tier: PoolTier) -> crate::pool::TierMetrics {
        self.pools.tier_metrics(tier)
    }

    /// Get the number of live resources.
    pub fn live_resource_count(&self) -> u32 {
        self.inner.lock().table.len()
    }
}

/// Diagnostic view of a resource.
#[derive(Clone, Debug)]
pub struct ResourceInspection {
    /// The resource ID.
    pub resource_id: ResourceId,
    /// Current ownership state.
    pub state: ResourceState,
    /// Current owner.
    pub owner: OwnerId,
    /// Number of outstanding borrows.
    pub borrow_count: u32,
    /// Payload capacity in bytes.
    pub payload_capacity: u32,
    /// Current payload length.
    pub payload_len: u32,
    /// Pool tier this buffer belongs to.
    pub pool_tier: PoolTier,
    /// Whether this is a fallback allocation.
    pub is_fallback: bool,
    /// Content type tag.
    pub content_type: String,
    /// The flow this resource was allocated for.
    pub flow_id: FlowId,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::ResourceError;

    fn test_config() -> ResourceManagerConfig {
        ResourceManagerConfig {
            table_capacity: 64,
            pool_configs: [
                TierConfig {
                    tier: PoolTier::Small,
                    pool_size: 16,
                    preallocate: true,
                },
                TierConfig {
                    tier: PoolTier::Medium,
                    pool_size: 8,
                    preallocate: true,
                },
                TierConfig {
                    tier: PoolTier::Large,
                    pool_size: 4,
                    preallocate: true,
                },
                TierConfig {
                    tier: PoolTier::Huge,
                    pool_size: 2,
                    preallocate: false,
                },
            ],
            default_budget_bytes: 1024 * 1024, // 1 MiB
        }
    }

    fn test_manager() -> DefaultResourceManager {
        DefaultResourceManager::new(test_config(), Arc::new(NoopEventSink))
    }

    // --- Allocation ---

    #[test]
    fn test_allocate_and_inspect() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let handle = mgr.allocate(OwnerId::Host, 100, flow).unwrap();
        let info = mgr.inspect(handle).unwrap();
        assert_eq!(info.state, ResourceState::Owned);
        assert_eq!(info.owner, OwnerId::Host);
        assert_eq!(info.payload_len, 0);
        assert!(info.payload_capacity >= 100);
    }

    #[test]
    fn test_allocate_and_release() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let handle = mgr.allocate(OwnerId::Host, 100, flow).unwrap();
        mgr.release(handle, OwnerId::Host).unwrap();
        assert_eq!(mgr.live_resource_count(), 0);
    }

    // --- Read / Write ---

    #[test]
    fn test_write_and_read_payload() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let owner = OwnerId::Host;
        let handle = mgr.allocate(owner, 256, flow).unwrap();
        mgr.write_payload(handle, owner, 0, b"hello world", flow)
            .unwrap();

        let data = mgr.read_payload(handle, owner, 0, 11, flow).unwrap();
        assert_eq!(&data, b"hello world");

        mgr.release(handle, owner).unwrap();
    }

    #[test]
    fn test_read_out_of_bounds() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let owner = OwnerId::Host;
        let handle = mgr.allocate(owner, 256, flow).unwrap();
        mgr.write_payload(handle, owner, 0, b"short", flow)
            .unwrap();

        let result = mgr.read_payload(handle, owner, 0, 100, flow);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResourceError::OutOfBounds { .. }
        ));

        mgr.release(handle, owner).unwrap();
    }

    // --- Borrow Lifecycle ---

    #[test]
    fn test_borrow_lifecycle() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let owner = OwnerId::Host;
        let handle = mgr.allocate(owner, 256, flow).unwrap();

        mgr.borrow_start(handle, ComponentId::new(1)).unwrap();
        let info = mgr.inspect(handle).unwrap();
        assert_eq!(info.state, ResourceState::Borrowed);
        assert_eq!(info.borrow_count, 1);

        mgr.borrow_end(handle, ComponentId::new(1)).unwrap();
        let info = mgr.inspect(handle).unwrap();
        assert_eq!(info.state, ResourceState::Owned);
        assert_eq!(info.borrow_count, 0);

        mgr.release(handle, owner).unwrap();
    }

    // --- Ownership Transfer ---

    #[test]
    fn test_transfer_ownership() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);
        mgr.register_component(ComponentId::new(1), None);
        mgr.register_component(ComponentId::new(2), None);

        let owner_a = OwnerId::Component(ComponentId::new(1));
        let owner_b = OwnerId::Component(ComponentId::new(2));

        let handle = mgr.allocate(owner_a, 256, flow).unwrap();
        mgr.transfer_ownership(handle, owner_a, owner_b).unwrap();

        let info = mgr.inspect(handle).unwrap();
        assert_eq!(info.owner, owner_b);

        mgr.release(handle, owner_b).unwrap();
    }

    // --- Force Reclaim ---

    #[test]
    fn test_force_reclaim() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);
        mgr.register_component(ComponentId::new(1), None);

        let owner = OwnerId::Component(ComponentId::new(1));
        let _h1 = mgr.allocate(owner, 100, flow).unwrap();
        let _h2 = mgr.allocate(owner, 200, flow).unwrap();
        assert_eq!(mgr.live_resource_count(), 2);

        let reclaimed = mgr.force_reclaim(ComponentId::new(1));
        assert_eq!(reclaimed.len(), 2);
        assert_eq!(mgr.live_resource_count(), 0);
    }

    // --- Budget Enforcement ---

    #[test]
    fn test_budget_exceeded() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);
        mgr.register_component(ComponentId::new(1), Some(512));

        let owner = OwnerId::Component(ComponentId::new(1));

        // Small tier = 256 bytes — first allocation succeeds
        let h1 = mgr.allocate(owner, 100, flow).unwrap();

        // Second allocation would exceed 512 budget (256 + 256 = 512, OK)
        let h2 = mgr.allocate(owner, 100, flow).unwrap();

        // Third would exceed (256 + 256 + 256 = 768 > 512)
        let result = mgr.allocate(owner, 100, flow);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResourceError::BudgetExceeded { .. }
        ));

        mgr.release(h1, owner).unwrap();
        mgr.release(h2, owner).unwrap();
    }

    // --- Copy Accounting ---

    #[test]
    fn test_copy_accounting() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let owner = OwnerId::Host;
        let handle = mgr.allocate(owner, 256, flow).unwrap();
        mgr.write_payload(handle, owner, 0, &[0u8; 100], flow)
            .unwrap();
        let _ = mgr.read_payload(handle, owner, 0, 100, flow).unwrap();

        let stats = mgr.flow_copy_stats(flow);
        assert_eq!(stats.total_copy_ops, 2); // one write, one read
        assert_eq!(stats.total_payload_bytes, 200); // 100 + 100

        mgr.release(handle, owner).unwrap();
    }

    // --- Flow Resource Cleanup ---

    #[test]
    fn test_release_flow_resources() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let owner = OwnerId::Host;
        let _h1 = mgr.allocate(owner, 100, flow).unwrap();
        let _h2 = mgr.allocate(owner, 200, flow).unwrap();
        assert_eq!(mgr.live_resource_count(), 2);

        let stats = mgr.release_flow_resources(flow).unwrap();
        assert_eq!(stats.returned_to_pool + stats.deallocated, 2);
        assert_eq!(mgr.live_resource_count(), 0);
    }

    // --- Content Type ---

    #[test]
    fn test_set_and_get_content_type() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);

        let owner = OwnerId::Host;
        let handle = mgr.allocate(owner, 100, flow).unwrap();
        mgr.set_content_type(handle, owner, "application/json")
            .unwrap();

        let ct = mgr.content_type(handle).unwrap();
        assert_eq!(ct, "application/json");

        mgr.release(handle, owner).unwrap();
    }

    // --- Full Pipeline Lifecycle ---

    #[test]
    fn test_full_pipeline_lifecycle_100_elements() {
        let mgr = test_manager();
        let flow = FlowId::new(1);
        mgr.register_flow(flow);
        mgr.register_component(ComponentId::new(1), None); // source
        mgr.register_component(ComponentId::new(2), None); // processor
        mgr.register_component(ComponentId::new(3), None); // sink

        let source = OwnerId::Component(ComponentId::new(1));
        let processor = OwnerId::Component(ComponentId::new(2));

        for i in 0u64..100 {
            // Source produces: allocate + write
            let src_buf = mgr.allocate(source, 256, flow).unwrap();
            let payload = format!("element-{i}");
            mgr.write_payload(src_buf, source, 0, payload.as_bytes(), flow)
                .unwrap();

            // Transfer to host (transit)
            mgr.transfer_ownership(src_buf, source, OwnerId::Host)
                .unwrap();

            // Host borrows to processor
            mgr.borrow_start(src_buf, ComponentId::new(2)).unwrap();
            let _data = mgr
                .read_payload(src_buf, OwnerId::Host, 0, payload.len() as u32, flow)
                .unwrap();
            mgr.borrow_end(src_buf, ComponentId::new(2)).unwrap();

            // Processor produces new buffer
            let proc_buf = mgr.allocate(processor, 256, flow).unwrap();
            let output = format!("processed-{i}");
            mgr.write_payload(proc_buf, processor, 0, output.as_bytes(), flow)
                .unwrap();

            // Release source buffer
            mgr.transfer_ownership(src_buf, OwnerId::Host, OwnerId::Host)
                .unwrap();
            mgr.release(src_buf, OwnerId::Host).unwrap();

            // Transfer proc buffer to host, then borrow to sink
            mgr.transfer_ownership(proc_buf, processor, OwnerId::Host)
                .unwrap();
            mgr.borrow_start(proc_buf, ComponentId::new(3)).unwrap();
            let _sink_data = mgr
                .read_payload(proc_buf, OwnerId::Host, 0, output.len() as u32, flow)
                .unwrap();
            mgr.borrow_end(proc_buf, ComponentId::new(3)).unwrap();

            // Release proc buffer
            mgr.release(proc_buf, OwnerId::Host).unwrap();
        }

        // Verify no leaks
        assert_eq!(mgr.live_resource_count(), 0);

        // Verify copy accounting
        let stats = mgr.flow_copy_stats(flow);
        // Per element: write(source) + read(processor) + write(processor) + read(sink) = 4 copies
        assert_eq!(stats.total_copy_ops, 400); // 100 elements * 4 copies
    }
}
