//! Comparison benchmark: Torvyn vs gRPC localhost.
//!
//! IMPLEMENTATION SPIKE REQUIRED:
//! This benchmark requires a running gRPC echo server.
//! Deferred to post-Phase-0.
//!
//! Expected setup:
//! 1. Start gRPC echo server on localhost:50051
//! 2. Run this benchmark
//! 3. Compare with Torvyn source→sink latency
//!
//! The gRPC server accepts a unary RPC:
//!   service Echo { rpc Process(Payload) returns (Payload); }
//! where Payload = { bytes data = 1; uint64 sequence = 2; }
//!
//! ## Comparison Methodology
//!
//! The goal is to measure Torvyn's overhead relative to a minimal gRPC
//! localhost transport. Both benchmarks should use:
//! - Identical payload sizes (256 bytes)
//! - Same element counts (1K, 10K, 100K)
//! - Same machine, same Tokio runtime
//! - Criterion for statistical rigor (same sample size, warmup)
//!
//! Expected results (Phase 0 targets):
//! - Torvyn Source→Sink should be 2-5x faster than gRPC unary for
//!   single-element latency (no serialization overhead)
//! - Torvyn throughput should exceed gRPC streaming by 3-10x for
//!   in-process workloads (zero network copy)
//!
//! ## Dependencies
//! - `tonic` (gRPC framework for Rust)
//! - `prost` (protobuf code generation)
//! - A `.proto` file defining the Echo service
//!
//! ## Implementation Steps (Post Phase 0)
//! 1. Add `tonic` and `prost` as dev-dependencies
//! 2. Create `proto/echo.proto` with the Echo service definition
//! 3. Implement the echo server in a test helper
//! 4. Implement the client benchmark using `tonic::transport::Channel`
//! 5. Run both Torvyn and gRPC benchmarks in the same criterion group

// TODO: Implement after Phase 0 core is complete.
// Dependencies: tonic, prost, tonic-build
// See comparison methodology above for setup instructions.

fn main() {
    eprintln!(
        "gRPC comparison benchmark is not yet implemented.\n\
         This benchmark requires tonic and prost dependencies.\n\
         Deferred to post-Phase-0. See benches/comparison/grpc_baseline.rs for methodology."
    );
}
