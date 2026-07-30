//! Real-Wasm benchmarks — the cost of the path Torvyn actually ships.
//!
//! Every arm here drives `cargo component`-built WebAssembly Components
//! through `WasmtimeEngine`, `WasmtimeInvoker`, the real buffer pool, and the
//! real flow driver. Contrast with `latency.rs`, `throughput.rs`,
//! `copy_accounting.rs`, and `memory.rs`, which drive a mock invoker that
//! executes no WebAssembly and moves no payload bytes; those measure the
//! runtime's overhead floor, this measures what a pipeline costs.
//!
//! Requires the `real-wasm` feature and the component toolchain:
//!
//! ```text
//! rustup target add wasm32-wasip2
//! cargo install cargo-component --locked
//! cargo bench -p torvyn-benchmarks --features real-wasm --bench real_wasm
//! ```
//!
//! # Groups
//!
//! - `real_wasm_latency` — per-element cost for both pipeline shapes across
//!   element counts, at the 256-byte comparison payload.
//! - `real_wasm_payload_scaling` — the same shape at four payload sizes
//!   spanning three buffer-pool tiers. This is what an ownership-aware
//!   runtime lives or dies on: how the four copies per element scale.
//! - `real_wasm_throughput` — sustained elements/second for both shapes.
//! - `real_wasm_instantiation` — pipeline startup on a warm engine, the cost
//!   the hot-path groups deliberately exclude.
//!
//! # Measurement discipline
//!
//! Instantiation is excluded from the three hot-path groups: each iteration
//! builds a fresh flow outside the clock via `iter_custom`, then times only
//! `RealFlow::run`. Instantiation is not free, so it is published in its own
//! group rather than amortised silently into a per-element number.
//!
//! Every configuration is validated before it is measured —
//! `WasmFixtures::validate` asserts clean completion, the exact element
//! count, the exact copy count and byte total for the shape, and a buffer
//! pool that returns to zero. A configuration that cannot pass those
//! assertions fails the benchmark rather than reporting a fast wrong number.

use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use torvyn_benchmarks::grpc::PAYLOAD_BYTES;
use torvyn_benchmarks::real_wasm::{timed_runs, RunSpec, Shape, WasmFixtures, PAYLOAD_SWEEP_BYTES};
use torvyn_integration_tests::FlowState;

/// Element counts for the latency group, matching the mock-invoker
/// benchmarks so the two sets of numbers line up row for row.
const LATENCY_ELEMENT_COUNTS: &[u64] = &[100, 1_000, 10_000];

/// Element count for the payload-scaling sweep. Large enough for the
/// per-element cost to dominate, small enough that four payload sizes and
/// two orders of magnitude of payload stay inside the CI budget.
const PAYLOAD_SCALING_ELEMENTS: u64 = 1_000;

/// Element count for the throughput group.
const THROUGHPUT_ELEMENTS: u64 = 10_000;

/// Real Wasm is roughly an order of magnitude slower per element than the
/// mock path, so the sample counts are lower and the measurement windows
/// shorter than the mock benchmarks'. Criterion still bootstraps confidence
/// intervals from these; the wall-clock budget is what keeps the CI
/// benchmark job inside its timeout.
const SAMPLE_SIZE: usize = 20;
const MEASUREMENT_TIME: Duration = Duration::from_secs(8);

/// Validate a configuration end to end before measuring it.
fn validate(rt: &Runtime, fixtures: &WasmFixtures, spec: RunSpec) {
    rt.block_on(fixtures.validate(&spec));
}

// ---------------------------------------------------------------------------
// Latency: per-element cost by shape and element count
// ---------------------------------------------------------------------------

fn bench_latency(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let fixtures = WasmFixtures::new();

    let mut group = c.benchmark_group("real_wasm_latency");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);

    for shape in [Shape::SourceSink, Shape::SourceProcessorSink] {
        for &elements in LATENCY_ELEMENT_COUNTS {
            let spec = RunSpec::new(shape, elements, PAYLOAD_BYTES as u64);
            validate(&rt, &fixtures, spec);

            group.throughput(Throughput::Elements(elements));
            group.bench_with_input(
                BenchmarkId::new(shape.label(), elements),
                &spec,
                |b, &spec| {
                    b.to_async(&rt)
                        .iter_custom(|iters| timed_runs(&fixtures, spec, iters));
                },
            );
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Payload scaling: what the four copies per element actually cost
// ---------------------------------------------------------------------------

fn bench_payload_scaling(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let fixtures = WasmFixtures::new();

    let mut group = c.benchmark_group("real_wasm_payload_scaling");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);

    for &payload_bytes in PAYLOAD_SWEEP_BYTES {
        let spec = RunSpec::new(
            Shape::SourceProcessorSink,
            PAYLOAD_SCALING_ELEMENTS,
            payload_bytes,
        );
        validate(&rt, &fixtures, spec);

        // Report bytes/second as well as time: the interesting question is
        // whether the four copies stay bandwidth-bound as the payload grows.
        group.throughput(Throughput::Bytes(payload_bytes * PAYLOAD_SCALING_ELEMENTS));
        group.bench_with_input(
            BenchmarkId::new("payload_bytes", payload_bytes),
            &spec,
            |b, &spec| {
                b.to_async(&rt)
                    .iter_custom(|iters| timed_runs(&fixtures, spec, iters));
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Throughput: sustained elements/second
// ---------------------------------------------------------------------------

fn bench_throughput(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let fixtures = WasmFixtures::new();

    let mut group = c.benchmark_group("real_wasm_throughput");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);
    group.throughput(Throughput::Elements(THROUGHPUT_ELEMENTS));

    for shape in [Shape::SourceSink, Shape::SourceProcessorSink] {
        let spec = RunSpec::new(shape, THROUGHPUT_ELEMENTS, PAYLOAD_BYTES as u64);
        validate(&rt, &fixtures, spec);

        group.bench_function(shape.label(), |b| {
            b.to_async(&rt)
                .iter_custom(|iters| timed_runs(&fixtures, spec, iters));
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Instantiation: the startup cost the hot-path groups exclude
// ---------------------------------------------------------------------------

fn bench_instantiation(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let fixtures = WasmFixtures::new();

    let mut group = c.benchmark_group("real_wasm_instantiation");
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(MEASUREMENT_TIME);

    // Zero elements: the source returns `None` on its first pull, so draining
    // the flow after the clock stops costs essentially nothing and the
    // measurement is instantiation alone.
    for shape in [Shape::SourceSink, Shape::SourceProcessorSink] {
        let spec = RunSpec::new(shape, 0, PAYLOAD_BYTES as u64);
        validate(&rt, &fixtures, spec);

        group.bench_function(shape.label(), |b| {
            b.to_async(&rt).iter_custom(|iters| {
                let fixtures = &fixtures;
                async move {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        // Measured: instantiate + `lifecycle.init` for every
                        // stage. Compilation is already cached on this engine,
                        // so this is what pipeline startup costs once
                        // components are warm — the README's "sub-second
                        // startup for cached components" claim.
                        let start = Instant::now();
                        let flow = fixtures.build_flow(&spec).await;
                        total += start.elapsed();

                        // Untimed: drain and retire, so the next iteration
                        // starts from the same clean state.
                        let flow_id = flow.flow_id();
                        let (state, _stats) = flow.run().await;
                        assert_eq!(state, FlowState::Completed);
                        fixtures.retire_flow(flow_id);
                    }
                    total
                }
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_latency,
    bench_payload_scaling,
    bench_throughput,
    bench_instantiation
);
criterion_main!(benches);
