# Torvyn Benchmark Suite

Performance benchmarks for the Torvyn streaming runtime.

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run a specific benchmark
cargo bench --bench latency
cargo bench --bench throughput
cargo bench --bench copy_accounting
cargo bench --bench memory

# Run with a filter (criterion pattern)
cargo bench -- "source_sink"

# Run with verbose output
cargo bench -- --verbose
```

## Benchmark Descriptions

### `latency` — Per-Element Latency

Measures end-to-end latency for flowing N elements through a Source → Sink
topology. Reports p50, p95, p99, and p99.9 percentiles via criterion's
statistical analysis.

**Configurations:** 100, 1,000, 10,000 elements.

### `throughput` — Sustained Elements/Second

Measures maximum sustained throughput for two topologies:
- **Source → Sink:** Direct pass-through (baseline)
- **Source → Processor → Sink:** Single-stage identity transformation

**Configuration:** 100,000 elements per iteration.

### `copy_accounting` — Copy Count Verification

Verifies that copy accounting infrastructure operates correctly and
measures its overhead. Includes:
- Ledger operation overhead (recording 1,000 elements × 2 copies each)
- Source → Sink flow execution
- Source → Processor → Sink flow execution

**Design expectation:** 2 copies per element for Source → Sink
(ComponentToHost from source, HostToComponent to sink).

### `memory` — Peak Memory Under Load

Verifies bounded memory growth under backpressure. Uses:
- Small queue depth (16) with slow sink to force backpressure
- Default queue depth with 100K elements for throughput baseline

Backpressure should prevent unbounded queue buildup (no OOM).

### `comparison/grpc_comparison` — gRPC Localhost Baseline

Side-by-side comparison of Torvyn Source → Sink against a minimal Tonic-based
gRPC unary echo service running on `127.0.0.1`. Both arms share the same
Tokio runtime, payload size (256 bytes), element counts (100 / 1 000 /
10 000), and criterion configuration so the resulting numbers are directly
comparable — see *Measured Baseline* below for the latest figures.

The gRPC server is in-process (random TCP port). The protobuf compiler
(`protoc`) is bundled hermetically via the `protoc-bin-vendored` crate, so
`cargo bench` works on any platform without needing system protobuf
tooling installed.

## Interpreting Results

Criterion produces HTML reports in `target/criterion/`. Open
`target/criterion/report/index.html` for the full report.

### Key Metrics

| Metric | What It Means |
|--------|---------------|
| **time** | Wall-clock time per iteration (lower is better) |
| **thrpt** | Throughput in elements/second (higher is better) |
| **change** | Percentage change from last run (negative = improvement) |

### Percentile Interpretation

- **p50 (median):** Typical latency for half of iterations
- **p95:** 95th percentile — most iterations are below this
- **p99:** Tail latency — important for SLA compliance
- **p99.9:** Extreme tail — captures rare spikes

## Methodology

### Statistical Rigor

All benchmarks use criterion with:
- **Warmup:** Criterion auto-detects warmup duration
- **Sample size:** 50 iterations for latency, 20 for throughput
- **Measurement time:** 10s for latency, 15s for throughput
- **Statistical model:** Linear regression with bootstrap confidence intervals
- **Outlier detection:** Automatic via criterion's outlier classification

### Reproducibility

For reproducible results:
1. Close other applications
2. Disable CPU frequency scaling if possible
3. Run benchmarks multiple times and compare reports
4. Use `cargo bench -- --save-baseline <name>` to save baselines
5. Use `cargo bench -- --baseline <name>` to compare against a baseline

### Comparison Between Runs

```bash
# Save a baseline
cargo bench -- --save-baseline before_change

# Make changes, then compare
cargo bench -- --baseline before_change
```

## Phase 0 Targets

These are the published design targets. Measured numbers are in the next
section; current measurements meet or exceed every target.

| Benchmark | Target | Notes |
|-----------|--------|-------|
| Source → Sink latency (1K elements) | < 1 ms | In-process, no Wasm |
| Source → Sink throughput | > 1 M elements/sec | Mock invoker, no serialization |
| Source → Processor → Sink throughput | > 500 K elements/sec | Identity processor |
| Copy accounting overhead | < 100 ns per record | Atomic operations only |
| Memory (backpressure) | Bounded by queue depth | No OOM under sustained load |

These targets use `TestInvoker` (mock), not real Wasm components. Real
component benchmarks will be added once Item 2 (real Wasm support) lands.

## Measured Baseline (Phase 0)

The numbers below are from a single full run with the criterion configuration
listed in each benchmark file (`sample_size = 50` for latency,
`sample_size = 20` for throughput, `measurement_time = 10 s` and `15 s`
respectively). They use the **mock invoker** path — exactly the same code
path as the existing benchmarks. Real-Wasm benchmarks are not yet wired and
are tracked separately.

### Hardware / toolchain (run conditions)

- CPU: Apple M-series (aarch64-apple-darwin)
- OS: macOS (Darwin 25.x)
- Rust: 1.95.0 stable
- Profile: `release` (LTO, single codegen unit per `[profile.release]` in the
  workspace `Cargo.toml`)
- Single-tenant developer machine with light background load. Variance is
  bounded by criterion's outlier detection.

Numbers will differ on Linux CI shared runners, on different hardware, and
under load. They are intended as representative figures, not guaranteed
SLOs. Reproduce locally with `cargo bench` and compare against your own
hardware baseline.

### Source → Sink latency (median time across the iteration; per-element
in parentheses)

| Element count | Torvyn | gRPC unary localhost | Speedup |
|--:|--:|--:|--:|
| 100 | 44.8 µs (~448 ns/element) | 5.79 ms (~57.9 µs/element) | ~129× |
| 1 000 | 407 µs (~407 ns/element) | 57.6 ms (~57.6 µs/element) | ~142× |
| 10 000 | 4.09 ms (~409 ns/element) | 580 ms (~58.0 µs/element) | ~142× |

### Sustained throughput

| Topology | Throughput | Per-element |
|---|--:|--:|
| Source → Sink (`throughput.rs`, 100 K elements) | 2.19 M elem/s | ~456 ns |
| Source → Processor → Sink (`throughput.rs`, 100 K elements) | 1.37 M elem/s | ~732 ns |
| gRPC unary localhost (`grpc_comparison.rs`, 10 K elements) | 17.77 K elem/s | ~56.3 µs |

### Copy-accounting overhead (`copy_accounting.rs`)

| Operation | Median time | Per-op |
|---|--:|--:|
| 1 000 `record_copy` ops on `CopyLedger` | 11.9 µs | ~11.9 ns |
| Source → Sink flow (1 000 elements, mock invoker) | 427 µs | ~427 ns |
| Source → Processor → Sink flow (1 000 elements, mock invoker) | 737 µs | ~737 ns |

### Headline interpretation

- **Per-element overhead in steady state: ~410 ns** (Source → Sink, 1 K and
  10 K element runs converge here once setup costs amortize). The < 5 µs
  target is met by an order of magnitude.
- **Single-core throughput: > 2 M elements/sec.** The > 1 M elements/sec
  target is met with ~2× headroom.
- **Two-stage pipeline (Source → Processor → Sink): 1.37 M elements/sec.**
  Adding a stage adds ~280 ns per element. The > 500 K target is met with
  ~2.7× headroom.
- **Copy ledger overhead: ~12 ns per record.** Well below the < 100 ns
  target (~8× headroom).
- **Versus gRPC unary localhost: ~140× faster** for streaming workloads of
  any size beyond the cold-start regime. The cost ratio is dominated by
  protobuf serialization + HTTP/2 framing, neither of which Torvyn pays.
- The published `README.md` performance targets are observed to hold under
  these run conditions.

### How to reproduce

```bash
# Full run (takes ~3-5 minutes total)
cargo bench -p torvyn-benchmarks

# Just the gRPC comparison
cargo bench -p torvyn-benchmarks --bench grpc_comparison

# Quick (smaller sample size; useful for smoke tests, not for publishing)
cargo bench -p torvyn-benchmarks -- --quick
```

Criterion HTML reports land in `target/criterion/`. Open
`target/criterion/report/index.html` for the full statistical summary
(p50/p95/p99/p99.9 percentiles, distribution plots, change vs. baseline).

## CI Integration

Benchmarks run on every push to `main` via GitHub Actions. Results are
saved as artifacts and tracked over time using `benchmark-action/github-action-benchmark`.

See `.github/workflows/ci.yml` for the benchmark CI job configuration.
