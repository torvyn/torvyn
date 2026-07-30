//! Side-by-side comparison: Torvyn vs. gRPC unary on localhost.
//!
//! This benchmark answers the headline question for prospective users:
//! *"How does Torvyn compare to a minimal in-process gRPC transport on the
//! same machine?"*
//!
//! # Three arms, two of which are not interchangeable
//!
//! | Arm | What it executes | What it is good for |
//! |---|---|---|
//! | `torvyn_real_wasm` | Three `cargo component`-built guests through `WasmtimeEngine`, real buffer pool, four measured copies of a 256-byte payload per element | **The like-for-like number.** This is what a Torvyn pipeline costs. |
//! | `torvyn_mock_invoker` | Queue transfer, scheduling, and copy accounting only — no WebAssembly, no payload bytes moved | The runtime's overhead floor: how much of the real cost is Torvyn's own machinery rather than guest execution. |
//! | `grpc_unary_localhost` | Tonic unary echo over loopback TCP: protobuf encode/decode, HTTP/2 framing, kernel round trip, 256 bytes each way | The baseline. |
//!
//! The `torvyn_real_wasm` arm is the only one that may be quoted as
//! "Torvyn vs. gRPC". `torvyn_mock_invoker` crosses no component boundary
//! and moves no payload bytes, so a ratio taken against it compares
//! Torvyn's scheduling overhead to gRPC's *entire* transport cost — a
//! number that flatters Torvyn by roughly an order of magnitude and means
//! something quite different from what a reader would assume. It is kept
//! because the floor is genuinely informative, and labelled so it cannot be
//! mistaken for the shipping cost.
//!
//! The real-Wasm arm requires the `real-wasm` feature and the component
//! toolchain; without it this file still builds and runs, reporting the
//! mock arm and gRPC only.
//!
//! # Why the real-Wasm arm is the three-stage topology
//!
//! A gRPC unary echo is client-send → server-handler → client-receive. The
//! closest Torvyn analogue is Source → Processor → Sink: the source stands in
//! for the client's send, the processor for the server's handler, the sink
//! for the client's receive. It is also the more conservative choice — the
//! three-stage flow costs roughly 1.6× the two-stage one, so the comparison
//! cannot be accused of picking Torvyn's cheapest shape. `real_wasm.rs`
//! publishes both shapes if you want the two-stage number.
//!
//! # Methodology choices
//!
//! - The gRPC server is spawned **once** per benchmark group on a random
//!   `127.0.0.1` port, and the client `Channel` is constructed once and
//!   cloned per iteration (`Channel` is internally `Arc`-shared). This
//!   isolates the comparison to per-call transport cost rather than server
//!   startup or TCP/HTTP-2 handshaking, which would dominate small-N counts.
//! - Correspondingly, the Torvyn real-Wasm arm excludes component
//!   instantiation from the measured region (`iter_custom`, clock started at
//!   `RealFlow::run`). Both sides therefore measure steady-state per-element
//!   cost against warm infrastructure. Torvyn's instantiation cost is
//!   published separately by the `real_wasm_instantiation` group in
//!   `benches/real_wasm.rs`.
//! - The Torvyn mock arm follows the existing `latency.rs` pattern, with
//!   `build_driver` inside `iter` — for the mock engine that setup is
//!   negligible next to the element loop.
//! - Both real arms carry the same 256-byte payload
//!   (`torvyn_benchmarks::grpc::PAYLOAD_BYTES`). The mock arm carries none,
//!   because the mock invoker has no payload to carry.
//! - Every arm asserts every element succeeded. A benchmark that silently
//!   drops work is worse than no benchmark.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use torvyn_benchmarks::grpc::{
    drive_unary_calls, payload_template, GrpcBaseline, LATENCY_ELEMENT_COUNTS,
    THROUGHPUT_ELEMENT_COUNT,
};
use torvyn_integration_tests::{
    build_driver, conn, sink, source, FlowConfig, FlowId, FlowState, FlowTopology, TestInvoker,
};

#[cfg(feature = "real-wasm")]
use torvyn_benchmarks::{
    grpc::PAYLOAD_BYTES,
    real_wasm::{timed_runs, RunSpec, Shape, WasmFixtures},
};

/// Criterion configuration for both groups. The measurement windows match
/// the mock `latency.rs` / `throughput.rs` benchmarks so reports stay
/// visually aligned; the sample counts are lower than `latency.rs`'s 50
/// because this file now runs three arms per element count instead of two,
/// and the gRPC arm at 10 000 elements is the slowest single benchmark in
/// the suite.
const LATENCY_SAMPLE_SIZE: usize = 20;
const LATENCY_MEASUREMENT_SECS: u64 = 10;
const THROUGHPUT_SAMPLE_SIZE: usize = 20;
const THROUGHPUT_MEASUREMENT_SECS: u64 = 15;

fn torvyn_source_sink_topology() -> FlowTopology {
    FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    }
}

/// Drive the mock-invoker Source → Sink flow for `count` elements.
async fn drive_mock_flow(count: u64) {
    let invoker = TestInvoker::new(count);
    let topology = torvyn_source_sink_topology();
    topology
        .validate()
        .expect("topology must be valid for the bench");
    let config = FlowConfig::default_with_topology(topology.clone());
    let flow_id = FlowId::new(1);

    let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
    let (_id, state, stats) = driver.run().await;

    assert_eq!(state, FlowState::Completed);
    assert_eq!(stats.total_elements, count);
}

// ---------------------------------------------------------------------------
// Latency group
// ---------------------------------------------------------------------------

fn bench_latency_torvyn_vs_grpc(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let grpc = GrpcBaseline::spawn(&rt);
    let payload = payload_template();

    #[cfg(feature = "real-wasm")]
    let fixtures = WasmFixtures::new();

    let mut group = c.benchmark_group("torvyn_vs_grpc_latency");
    group.sample_size(LATENCY_SAMPLE_SIZE);
    group.measurement_time(std::time::Duration::from_secs(LATENCY_MEASUREMENT_SECS));

    for &count in LATENCY_ELEMENT_COUNTS {
        group.throughput(Throughput::Elements(count));

        // --- Torvyn, real Wasm (the like-for-like arm) ---
        #[cfg(feature = "real-wasm")]
        {
            let spec = RunSpec::new(Shape::SourceProcessorSink, count, PAYLOAD_BYTES as u64);
            rt.block_on(fixtures.validate(&spec));
            group.bench_with_input(
                BenchmarkId::new("torvyn_real_wasm", count),
                &spec,
                |b, &spec| {
                    b.to_async(&rt)
                        .iter_custom(|iters| timed_runs(&fixtures, spec, iters));
                },
            );
        }

        // --- Torvyn, mock invoker (overhead floor) ---
        group.bench_with_input(
            BenchmarkId::new("torvyn_mock_invoker", count),
            &count,
            |b, &count| {
                b.to_async(&rt).iter(|| drive_mock_flow(count));
            },
        );

        // --- gRPC baseline ---
        group.bench_with_input(
            BenchmarkId::new("grpc_unary_localhost", count),
            &count,
            |b, &count| {
                b.to_async(&rt)
                    .iter(|| drive_unary_calls(grpc.client(), &payload, count));
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Throughput group
// ---------------------------------------------------------------------------

fn bench_throughput_torvyn_vs_grpc(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let grpc = GrpcBaseline::spawn(&rt);
    let payload = payload_template();

    #[cfg(feature = "real-wasm")]
    let fixtures = WasmFixtures::new();

    let mut group = c.benchmark_group("torvyn_vs_grpc_throughput");
    group.sample_size(THROUGHPUT_SAMPLE_SIZE);
    group.measurement_time(std::time::Duration::from_secs(THROUGHPUT_MEASUREMENT_SECS));
    group.throughput(Throughput::Elements(THROUGHPUT_ELEMENT_COUNT));

    // --- Torvyn, real Wasm (the like-for-like arm) ---
    #[cfg(feature = "real-wasm")]
    {
        let spec = RunSpec::new(
            Shape::SourceProcessorSink,
            THROUGHPUT_ELEMENT_COUNT,
            PAYLOAD_BYTES as u64,
        );
        rt.block_on(fixtures.validate(&spec));
        group.bench_function("torvyn_real_wasm", |b| {
            b.to_async(&rt)
                .iter_custom(|iters| timed_runs(&fixtures, spec, iters));
        });
    }

    // --- Torvyn, mock invoker (overhead floor) ---
    group.bench_function("torvyn_mock_invoker", |b| {
        b.to_async(&rt)
            .iter(|| drive_mock_flow(THROUGHPUT_ELEMENT_COUNT));
    });

    // --- gRPC baseline ---
    group.bench_function("grpc_unary_localhost", |b| {
        b.to_async(&rt)
            .iter(|| drive_unary_calls(grpc.client(), &payload, THROUGHPUT_ELEMENT_COUNT));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_latency_torvyn_vs_grpc,
    bench_throughput_torvyn_vs_grpc
);
criterion_main!(benches);
