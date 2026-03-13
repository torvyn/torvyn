//! `torvyn-reactor` — Scheduling, backpressure, and flow lifecycle.
//!
//! This crate is the execution engine of the Torvyn runtime. It provides:
//!
//! - **Flow drivers**: One Tokio task per flow, running the scheduling loop.
//! - **Bounded queues**: Pre-allocated ring buffers between pipeline stages.
//! - **Backpressure**: High/low watermark hysteresis to prevent queue overflow.
//! - **Demand propagation**: Credit-based pull flow from consumers to producers.
//! - **Cancellation**: Cooperative cancellation with reason tracking.
//! - **Scheduling policies**: Consumer-first, demand-driven execution ordering.
//! - **Flow lifecycle**: Created → Validated → Instantiated → Running → Draining → terminal.
//!
//! # Architecture
//!
//! The reactor is a domain-specific scheduling layer on top of Tokio. It does
//! NOT replace Tokio — it handles intra-flow scheduling while Tokio handles
//! inter-flow scheduling via its work-stealing thread pool.
//!
//! The reactor has **no direct dependency on Wasmtime**. All component
//! invocations go through the [`ComponentInvoker`](torvyn_engine::ComponentInvoker)
//! trait from `torvyn-engine`.

#![deny(missing_docs)]

pub mod backpressure;
pub mod cancellation;
pub mod config;
pub mod coordinator;
pub mod demand;
pub mod error;
pub mod events;
pub mod fairness;
pub mod flow_driver;
pub mod handle;
pub mod metrics;
pub mod queue;
pub mod scheduling;
pub mod stream;
pub mod topology;

// Re-exports for convenient access.
pub use backpressure::BackpressureState;
pub use cancellation::{CancellationReason, FlowCancellation};
pub use config::{FlowConfig, StreamConfig, TimeoutConfig, YieldConfig};
pub use error::{ErrorPolicy, FlowCreationError, FlowError};
pub use events::{ReactorEvent, ShutdownResult};
pub use fairness::FlowPriority;
pub use flow_driver::{FlowDriver, FlowDriverHandle};
pub use handle::ReactorHandle;
pub use metrics::{ComponentMetrics, FlowCompletionStats, ReactorMetrics, StreamMetrics};
pub use queue::BoundedQueue;
pub use scheduling::{DemandDrivenPolicy, SchedulingPolicy, StageAction, StageExecution};
pub use stream::{StreamElementRef, StreamState};
pub use topology::{FlowTopology, StageDefinition, StreamConnection};
