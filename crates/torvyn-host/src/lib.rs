#![deny(missing_docs)]

//! Runtime orchestration shell for Torvyn.
//!
//! This crate provides the `HostBuilder` and `Host` that tie all subsystems
//! together: config parsing, contract validation, linking, compilation,
//! instantiation, and flow lifecycle management.
