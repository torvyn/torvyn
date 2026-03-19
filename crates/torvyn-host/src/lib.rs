//! `torvyn-host` — Runtime orchestration shell for the Torvyn runtime.
//!
//! This crate is a **thin orchestration shell**. It owns no complex logic
//! itself — it delegates to the subsystem crates (`torvyn-engine`,
//! `torvyn-reactor`, `torvyn-pipeline`, etc.).
//!
//! # Responsibilities
//! - **Host builder**: staged construction of a configured runtime
//! - **Startup sequence**: parse config -> validate -> link -> compile -> run
//! - **Runtime loop**: runs until all flows complete, cancellation, or signal
//! - **Shutdown sequence**: graceful drain -> force-terminate -> flush
//! - **Inspection API**: runtime queries for CLI and diagnostics
//!
//! # Examples
//! ```no_run
//! use torvyn_host::HostBuilder;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), torvyn_host::HostError> {
//!     let mut host = HostBuilder::new()
//!         .with_config_file("Torvyn.toml")
//!         .build()
//!         .await?;
//!
//!     host.run().await
//! }
//! ```
//!
//! # Architecture
//! All code in this crate is **COLD PATH**. Hot-path element processing
//! lives in `torvyn-reactor`. This crate orchestrates startup, flow
//! registration, signal handling, and shutdown.

#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(clippy::pedantic)]
// Pedantic allows for justified deviations
#![allow(clippy::module_name_repetitions)]

pub mod builder;
pub mod error;
pub mod host;
pub mod inspection;
pub mod shutdown;
pub mod startup;

#[cfg(feature = "signal")]
pub mod signal;

// Re-exports
pub use builder::{HostBuilder, HostConfig};
pub use error::{FlowError, HostError, StartupError, StartupStage};
pub use host::{FlowRecord, HostStatus, TorvynHost};
pub use inspection::{FlowSummary, InspectionHandle};
pub use shutdown::ShutdownOutcome;
