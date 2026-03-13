//! # torvyn-resources
//!
//! Buffer pools, ownership tracking, and copy accounting for the
//! Torvyn reactive streaming runtime.
//!
//! This crate implements the resource manager subsystem described in
//! `03_resource_manager_and_ownership.md`. It is one of the two most
//! critical crates in Torvyn — the ownership-aware promise lives or dies here.
//!
//! ## Key Types
//! - [`DefaultResourceManager`] — the main entry point for all resource operations.
//! - [`OwnerId`] — identifies who owns a resource (Host, Component, or Transit).
//! - [`PoolTier`] — buffer pool tier classification (Small, Medium, Large, Huge).
//! - [`FlowCopyStatsSnapshot`] — per-flow copy accounting summary.
//!
//! ## Architecture
//! ```text
//! DefaultResourceManager (&self, interior mutability)
//!   ├── ResourceTable (generational slab)
//!   ├── BufferPoolSet (4 tiered Treiber stacks)
//!   ├── BudgetRegistry (per-component memory limits)
//!   └── CopyLedger (per-flow copy accounting)
//! ```

#![deny(missing_docs)]
// Unsafe code is isolated to the buffer module.
#![allow(unsafe_code)]

mod error;

/// Resource ownership identity and table entry types.
pub mod handle;

/// Generational slab resource table.
pub mod table;

/// Host-side byte buffer allocation and access.
pub mod buffer;

/// Tiered buffer pool with Treiber stacks.
pub mod pool;

/// Ownership state machine enforcement.
pub mod ownership;

/// Copy accounting and flow-level statistics.
pub mod accounting;

/// Per-component memory budget enforcement.
pub mod budget;

/// The DefaultResourceManager — unified resource lifecycle API.
pub mod manager;

// --- Re-exports ---

pub use accounting::FlowCopyStatsSnapshot;
pub use handle::{
    BufferFlags, ContentType, FlowResourceStats, OwnerId, PoolTier, ResourceEntry,
    ResourceReclaimed,
};
pub use manager::{DefaultResourceManager, ResourceInspection, ResourceManagerConfig};
pub use pool::{BufferPoolSet, TierConfig, TierMetrics};
pub use table::ResourceTable;
