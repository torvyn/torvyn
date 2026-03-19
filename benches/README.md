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

### `comparison/grpc_baseline` — gRPC Comparison (Stub)

Deferred to post-Phase 0. Will compare Torvyn Source → Sink performance
against an equivalent gRPC localhost unary RPC for the same payload size.

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

## Expected Baselines (Phase 0)

These are rough targets for Phase 0 on modern hardware (2024 desktop/laptop):

| Benchmark | Target | Notes |
|-----------|--------|-------|
| Source → Sink latency (1K elements) | < 1ms | In-process, no Wasm |
| Source → Sink throughput | > 1M elements/sec | Mock invoker, no serialization |
| Source → Processor → Sink throughput | > 500K elements/sec | Identity processor |
| Copy accounting overhead | < 100ns per record | Atomic operations only |
| Memory (backpressure) | Bounded by queue depth | No OOM under sustained load |

These targets use `TestInvoker` (mock), not real Wasm components. Real
component benchmarks will be added in Phase 1.

## CI Integration

Benchmarks run on every push to `main` via GitHub Actions. Results are
saved as artifacts and tracked over time using `benchmark-action/github-action-benchmark`.

See `.github/workflows/ci.yml` for the benchmark CI job configuration.
