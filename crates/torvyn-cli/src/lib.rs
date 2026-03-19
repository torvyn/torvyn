//! Torvyn CLI — the developer-facing command-line interface for the
//! Torvyn ownership-aware reactive streaming runtime.
//!
//! This binary provides commands for project scaffolding, contract validation,
//! pipeline execution, benchmarking, tracing, packaging, and environment diagnostics.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod cli;
pub mod commands;
pub mod errors;
pub mod output;
pub mod templates;
