//! # torvyn-engine
//!
//! Wasm engine abstraction and component invocation layer for the
//! Torvyn reactive streaming runtime.
//!
//! This crate provides:
//! - [`WasmEngine`] — abstract interface for Wasm runtime operations
//!   (compile, instantiate, fuel/memory management).
//! - [`ComponentInvoker`] — the hot-path typed invocation interface
//!   between the reactor and Wasm execution (**Gap G-01 fix**).
//! - [`WasmtimeEngine`] — Wasmtime-based `WasmEngine` implementation
//!   (feature: `wasmtime-backend`).
//! - [`WasmtimeInvoker`] — Wasmtime-based `ComponentInvoker` implementation
//!   (feature: `wasmtime-backend`).
//! - [`CompiledComponentCache`] — compiled component cache with disk support.
//! - Mock implementations for testing downstream crates (feature: `mock`).
//!
//! # Feature Flags
//! - `wasmtime-backend` (default) — enables the Wasmtime backend.
//! - `mock` — enables mock implementations for testing.
//! - `tracing-support` — enables structured logging via the `tracing` crate.
//!
//! # Architecture
//! Per Doc 02, Section 3: all Wasm engine interactions go through the
//! `WasmEngine` trait to insulate Torvyn from Wasmtime-specific APIs.
//! The `ComponentInvoker` trait (Doc 10, Gap G-01) provides typed
//! hot-path invocation between the reactor and Wasm execution.

#![deny(missing_docs)]

pub mod config;
pub mod error;
pub mod traits;
pub mod types;

#[cfg(feature = "wasmtime-backend")]
pub mod wasmtime_engine;

#[cfg(feature = "wasmtime-backend")]
pub mod wasmtime_invoker;

pub mod cache;

#[cfg(feature = "mock")]
pub mod mock;

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub use cache::CompiledComponentCache;
pub use config::{CompilationStrategy, WasmtimeEngineConfig};
pub use error::EngineError;
pub use traits::{ComponentInvoker, WasmEngine};
pub use types::{
    CompiledComponent, ComponentInstance, ImportBindings, InvocationResult, OutputElement,
    ProcessResult, StreamElement,
};

#[cfg(feature = "wasmtime-backend")]
pub use wasmtime_engine::WasmtimeEngine;

#[cfg(feature = "wasmtime-backend")]
pub use wasmtime_invoker::WasmtimeInvoker;
