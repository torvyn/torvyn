//! Torvyn benchmark suite.
//!
//! This crate provides workspace-level performance benchmarks using criterion.
//! See `benches/README.md` for usage and methodology.
//!
//! Benchmark binaries are defined as `[[bench]]` targets in `Cargo.toml`.
//! The benchmark code lives in `benches/` and `comparison/` alongside this
//! crate's `Cargo.toml`; the harness code they share lives here.
//!
//! Two execution paths are benchmarked, and keeping them straight is the
//! whole point of this crate's layout:
//!
//! - **Mock-invoker path** (`torvyn_integration_tests::TestInvoker`) —
//!   measures the runtime's own overhead: queue transfer, demand-driven
//!   scheduling, copy-ledger accounting. No WebAssembly is executed and no
//!   payload bytes move. This is a *floor*, not the shipping cost.
//! - **Real-Wasm path** (the `real_wasm` module, feature `real-wasm`) —
//!   measures what production actually does: `cargo component`-built guests
//!   behind `WasmtimeEngine` and `WasmtimeInvoker`, with real buffer
//!   allocation, real Canonical ABI marshalling, and the real per-element
//!   copies.
//!
//! Numbers from the two paths are not interchangeable, and no benchmark or
//! report should present one as if it were the other.

pub mod grpc;

#[cfg(feature = "real-wasm")]
pub mod real_wasm;
