//! Metrics registry: owns and indexes all metric structures.
//!
//! Flow metrics are registered at flow creation and deregistered at flow
//! completion. The registry provides Arc references for hot-path access
//! without locking.

use super::flow_metrics::FlowMetrics;
use super::pool_metrics::{PoolId, ResourcePoolMetrics};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use torvyn_types::{ComponentId, FlowId, StreamId};

/// Central metrics registry.
///
/// # Invariants
/// - Each flow ID maps to exactly one `FlowMetrics`.
/// - Registration and deregistration are serialized via `RwLock`.
/// - Hot-path access uses `Arc<FlowMetrics>` without locking.
pub struct MetricsRegistry {
    /// Flow metrics indexed by FlowId.
    flows: RwLock<HashMap<FlowId, Arc<FlowMetrics>>>,
    /// System-level counters.
    pub system: SystemMetrics,
    /// Pool metrics indexed by PoolId.
    pools: RwLock<HashMap<PoolId, Arc<ResourcePoolMetrics>>>,
}

/// System-wide metrics.
pub struct SystemMetrics {
    /// Currently active flows.
    pub active_flows: super::Gauge,
    /// Currently instantiated components.
    pub active_components: super::Gauge,
    /// Total scheduler wakeups.
    pub scheduler_wakeups: super::Counter,
    /// Time spent idle (ns).
    pub scheduler_idle_ns: super::Counter,
    /// Trace spans dropped due to export backpressure.
    pub spans_dropped: super::Counter,
    /// Diagnostic events dropped.
    pub events_dropped: super::Counter,
}

impl SystemMetrics {
    fn new() -> Self {
        Self {
            active_flows: super::Gauge::new(),
            active_components: super::Gauge::new(),
            scheduler_wakeups: super::Counter::new(),
            scheduler_idle_ns: super::Counter::new(),
            spans_dropped: super::Counter::new(),
            events_dropped: super::Counter::new(),
        }
    }
}

impl MetricsRegistry {
    /// Create a new empty registry.
    ///
    /// # COLD PATH
    pub fn new() -> Self {
        Self {
            flows: RwLock::new(HashMap::new()),
            system: SystemMetrics::new(),
            pools: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new flow and pre-allocate its metric structures.
    ///
    /// Returns an `Arc<FlowMetrics>` for hot-path access.
    ///
    /// # COLD PATH
    ///
    /// # Preconditions
    /// - `flow_id` must not already be registered.
    ///
    /// # Errors
    /// Returns `Err` if the flow is already registered.
    pub fn register_flow(
        &self,
        flow_id: FlowId,
        component_ids: &[ComponentId],
        stream_ids: &[StreamId],
        start_time_ns: u64,
    ) -> Result<Arc<FlowMetrics>, RegistryError> {
        let metrics = Arc::new(FlowMetrics::new(
            flow_id,
            component_ids,
            stream_ids,
            start_time_ns,
        ));

        let mut flows = self
            .flows
            .write()
            .map_err(|_| RegistryError::LockPoisoned)?;
        if flows.contains_key(&flow_id) {
            return Err(RegistryError::FlowAlreadyRegistered(flow_id));
        }
        flows.insert(flow_id, Arc::clone(&metrics));
        drop(flows);

        self.system.active_flows.increment(1);
        self.system
            .active_components
            .increment(component_ids.len() as u64);

        Ok(metrics)
    }

    /// Deregister a flow and return its final metrics.
    ///
    /// # COLD PATH
    pub fn deregister_flow(&self, flow_id: FlowId) -> Result<Arc<FlowMetrics>, RegistryError> {
        let mut flows = self
            .flows
            .write()
            .map_err(|_| RegistryError::LockPoisoned)?;
        let metrics = flows
            .remove(&flow_id)
            .ok_or(RegistryError::FlowNotFound(flow_id))?;
        drop(flows);

        self.system.active_flows.decrement(1);
        self.system
            .active_components
            .decrement(metrics.components.len() as u64);

        Ok(metrics)
    }

    /// Get flow metrics by ID (hot-path friendly — acquires read lock).
    ///
    /// # WARM PATH — read lock is shared.
    pub fn get_flow(&self, flow_id: FlowId) -> Option<Arc<FlowMetrics>> {
        let flows = self.flows.read().ok()?;
        flows.get(&flow_id).cloned()
    }

    /// List all active flow IDs.
    ///
    /// # COLD PATH
    pub fn active_flow_ids(&self) -> Vec<FlowId> {
        let flows = self.flows.read().unwrap_or_else(|e| e.into_inner());
        flows.keys().copied().collect()
    }

    /// Register a pool for metrics tracking.
    ///
    /// # COLD PATH
    pub fn register_pool(&self, pool_id: PoolId) -> Arc<ResourcePoolMetrics> {
        let metrics = Arc::new(ResourcePoolMetrics::new(pool_id));
        let mut pools = self.pools.write().unwrap_or_else(|e| e.into_inner());
        pools.insert(pool_id, Arc::clone(&metrics));
        metrics
    }

    /// Get pool metrics.
    pub fn get_pool(&self, pool_id: PoolId) -> Option<Arc<ResourcePoolMetrics>> {
        let pools = self.pools.read().ok()?;
        pools.get(&pool_id).cloned()
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from the metrics registry.
#[derive(Debug, Clone, PartialEq)]
pub enum RegistryError {
    /// Flow is already registered.
    FlowAlreadyRegistered(FlowId),
    /// Flow not found in registry.
    FlowNotFound(FlowId),
    /// Internal lock was poisoned.
    LockPoisoned,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FlowAlreadyRegistered(id) => {
                write!(f, "flow {id} is already registered in the metrics registry")
            }
            Self::FlowNotFound(id) => {
                write!(f, "flow {id} not found in the metrics registry")
            }
            Self::LockPoisoned => write!(f, "metrics registry lock poisoned"),
        }
    }
}

impl std::error::Error for RegistryError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_get_flow() {
        let reg = MetricsRegistry::new();
        let flow_id = FlowId::new(1);
        let comps = vec![ComponentId::new(1), ComponentId::new(2)];
        let streams = vec![StreamId::new(1)];

        let metrics = reg.register_flow(flow_id, &comps, &streams, 0).unwrap();
        assert_eq!(metrics.flow_id, flow_id);

        let retrieved = reg.get_flow(flow_id).unwrap();
        assert_eq!(retrieved.flow_id, flow_id);
    }

    #[test]
    fn test_register_duplicate_flow_fails() {
        let reg = MetricsRegistry::new();
        let flow_id = FlowId::new(1);

        reg.register_flow(flow_id, &[ComponentId::new(1)], &[StreamId::new(1)], 0)
            .unwrap();

        let result = reg.register_flow(flow_id, &[ComponentId::new(1)], &[StreamId::new(1)], 0);
        assert!(matches!(
            result,
            Err(RegistryError::FlowAlreadyRegistered(_))
        ));
    }

    #[test]
    fn test_deregister_flow() {
        let reg = MetricsRegistry::new();
        let flow_id = FlowId::new(1);

        reg.register_flow(flow_id, &[ComponentId::new(1)], &[StreamId::new(1)], 0)
            .unwrap();

        let metrics = reg.deregister_flow(flow_id).unwrap();
        assert_eq!(metrics.flow_id, flow_id);
        assert!(reg.get_flow(flow_id).is_none());
    }

    #[test]
    fn test_deregister_missing_flow_fails() {
        let reg = MetricsRegistry::new();
        let result = reg.deregister_flow(FlowId::new(99));
        assert!(matches!(result, Err(RegistryError::FlowNotFound(_))));
    }

    #[test]
    fn test_system_metrics_track_active_flows() {
        let reg = MetricsRegistry::new();
        reg.register_flow(
            FlowId::new(1),
            &[ComponentId::new(1), ComponentId::new(2)],
            &[StreamId::new(1)],
            0,
        )
        .unwrap();

        assert_eq!(reg.system.active_flows.read(), 1);
        assert_eq!(reg.system.active_components.read(), 2);

        reg.deregister_flow(FlowId::new(1)).unwrap();
        assert_eq!(reg.system.active_flows.read(), 0);
        assert_eq!(reg.system.active_components.read(), 0);
    }

    #[test]
    fn test_active_flow_ids() {
        let reg = MetricsRegistry::new();
        reg.register_flow(
            FlowId::new(1),
            &[ComponentId::new(1)],
            &[StreamId::new(1)],
            0,
        )
        .unwrap();
        reg.register_flow(
            FlowId::new(2),
            &[ComponentId::new(2)],
            &[StreamId::new(2)],
            0,
        )
        .unwrap();

        let ids = reg.active_flow_ids();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn test_pool_metrics() {
        let reg = MetricsRegistry::new();
        let pm = reg.register_pool(PoolId::new(0));
        pm.allocations.increment(10);

        let retrieved = reg.get_pool(PoolId::new(0)).unwrap();
        assert_eq!(retrieved.allocations.read(), 10);
    }
}
