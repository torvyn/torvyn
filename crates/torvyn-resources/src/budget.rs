//! Per-component memory budget enforcement.
//!
//! Per Doc 03, Section 11. Each component is assigned a max_owned_bytes
//! budget. Budget checks occur on allocation and ownership transfer.
//! Borrows do not count against budgets.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use torvyn_types::ComponentId;

use crate::error;

/// Default per-component memory budget: 64 MiB.
///
/// Per Doc 03, Section 11.3: deliberately generous.
pub const DEFAULT_MAX_OWNED_BYTES: u64 = 64 * 1024 * 1024;

/// Budget tracker for a single component.
pub struct ComponentBudget {
    /// Maximum bytes this component can own.
    pub max_owned_bytes: u64,
    /// Current bytes owned.
    pub current_owned_bytes: AtomicU64,
}

impl ComponentBudget {
    /// Create a new budget with the given limit.
    pub fn new(max_owned_bytes: u64) -> Self {
        Self {
            max_owned_bytes,
            current_owned_bytes: AtomicU64::new(0),
        }
    }

    /// Try to reserve `bytes` from the budget.
    ///
    /// Returns `Ok(())` if the reservation succeeds.
    /// Returns `Err(ResourceError::BudgetExceeded)` if it would exceed the budget.
    ///
    /// # WARM PATH — called per allocation and transfer.
    ///
    /// Note: This uses a simple load-then-CAS approach. In practice, the
    /// resource manager serializes access, so contention is minimal.
    pub fn try_reserve(&self, bytes: u64, component: ComponentId) -> crate::error::Result<()> {
        loop {
            let current = self.current_owned_bytes.load(Ordering::Relaxed);
            let new_total = current + bytes;
            if new_total > self.max_owned_bytes {
                return Err(error::budget_exceeded(
                    component,
                    current,
                    bytes,
                    self.max_owned_bytes,
                ));
            }
            match self.current_owned_bytes.compare_exchange_weak(
                current,
                new_total,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(()),
                Err(_) => continue,
            }
        }
    }

    /// Release `bytes` back to the budget.
    ///
    /// # WARM PATH — called per release and transfer-out.
    #[inline]
    pub fn release(&self, bytes: u64) {
        self.current_owned_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }

    /// Returns the current owned bytes.
    #[inline]
    pub fn current_owned(&self) -> u64 {
        self.current_owned_bytes.load(Ordering::Relaxed)
    }

    /// Returns the remaining budget.
    #[inline]
    pub fn remaining(&self) -> u64 {
        let current = self.current_owned_bytes.load(Ordering::Relaxed);
        self.max_owned_bytes.saturating_sub(current)
    }
}

/// Registry of per-component budgets.
pub struct BudgetRegistry {
    budgets: parking_lot::Mutex<HashMap<ComponentId, ComponentBudget>>,
    default_max: u64,
}

impl BudgetRegistry {
    /// Create a new registry with the given default budget per component.
    pub fn new(default_max: u64) -> Self {
        Self {
            budgets: parking_lot::Mutex::new(HashMap::new()),
            default_max,
        }
    }

    /// Register a component with a specific budget.
    ///
    /// # COLD PATH — called during component instantiation.
    pub fn register(&self, component: ComponentId, max_owned_bytes: Option<u64>) {
        let max = max_owned_bytes.unwrap_or(self.default_max);
        let mut budgets = self.budgets.lock();
        budgets.insert(component, ComponentBudget::new(max));
    }

    /// Unregister a component (component terminated).
    ///
    /// # COLD PATH
    pub fn unregister(&self, component: ComponentId) {
        let mut budgets = self.budgets.lock();
        budgets.remove(&component);
    }

    /// Try to reserve bytes for a component.
    ///
    /// If the component is not registered, allows the operation (default: no limit).
    ///
    /// # WARM PATH
    pub fn try_reserve(&self, component: ComponentId, bytes: u64) -> crate::error::Result<()> {
        let budgets = self.budgets.lock();
        if let Some(budget) = budgets.get(&component) {
            budget.try_reserve(bytes, component)
        } else {
            // Unregistered component — allow (graceful degradation)
            Ok(())
        }
    }

    /// Release bytes for a component.
    ///
    /// # WARM PATH
    pub fn release_bytes(&self, component: ComponentId, bytes: u64) {
        let budgets = self.budgets.lock();
        if let Some(budget) = budgets.get(&component) {
            budget.release(bytes);
        }
    }

    /// Get current usage for a component.
    ///
    /// # COLD PATH — diagnostics.
    pub fn current_usage(&self, component: ComponentId) -> u64 {
        let budgets = self.budgets.lock();
        budgets
            .get(&component)
            .map(|b| b.current_owned())
            .unwrap_or(0)
    }
}

impl Default for BudgetRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_OWNED_BYTES)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use torvyn_types::ResourceError;

    #[test]
    fn test_component_budget_reserve_and_release() {
        let budget = ComponentBudget::new(1024);
        let cid = ComponentId::new(1);
        budget.try_reserve(500, cid).unwrap();
        assert_eq!(budget.current_owned(), 500);
        assert_eq!(budget.remaining(), 524);

        budget.release(200);
        assert_eq!(budget.current_owned(), 300);
    }

    #[test]
    fn test_component_budget_exceeds() {
        let budget = ComponentBudget::new(1024);
        let cid = ComponentId::new(1);
        budget.try_reserve(1000, cid).unwrap();
        let result = budget.try_reserve(100, cid);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ResourceError::BudgetExceeded { .. }
        ));
    }

    #[test]
    fn test_component_budget_exact_limit() {
        let budget = ComponentBudget::new(1024);
        let cid = ComponentId::new(1);
        budget.try_reserve(1024, cid).unwrap();
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_budget_registry_register_and_check() {
        let registry = BudgetRegistry::new(1024);
        let cid = ComponentId::new(1);
        registry.register(cid, Some(2048));
        registry.try_reserve(cid, 1500).unwrap();
        assert_eq!(registry.current_usage(cid), 1500);
    }

    #[test]
    fn test_budget_registry_unregistered_allows() {
        let registry = BudgetRegistry::new(1024);
        let cid = ComponentId::new(99);
        registry.try_reserve(cid, 999_999_999).unwrap(); // Should succeed
    }

    #[test]
    fn test_budget_registry_unregister_removes() {
        let registry = BudgetRegistry::new(1024);
        let cid = ComponentId::new(1);
        registry.register(cid, None);
        registry.unregister(cid);
        assert_eq!(registry.current_usage(cid), 0);
    }
}
