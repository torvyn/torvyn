# Torvyn Benchmark Suite

Performance benchmarks for the Torvyn streaming runtime.

## Two paths, and why the distinction matters

This suite measures two different things, and conflating them produces
numbers that are technically true and practically misleading.

| Path | What it executes | What it tells you |
|---|---|---|
| **Real Wasm** (`--features real-wasm`) | `cargo component`-built guests through `WasmtimeEngine` + `WasmtimeInvoker`, real buffer pool, real Canonical ABI marshalling, real per-element copies | **What a Torvyn pipeline costs.** Quote these numbers. |
| **Mock invoker** (default) | Bounded-queue transfer, demand-driven scheduling, copy-ledger accounting — no WebAssembly executed, no payload bytes moved | The runtime's *overhead floor*: how much of the real cost is Torvyn's own machinery rather than guest execution. |

The mock path is genuinely useful — it isolates scheduler and queue
regressions from guest-execution noise, and it is where the reactor's own
cost shows up. It is not the cost of running a pipeline, and no number
derived from it should be presented as such.

Before the real-Wasm suite existed, every published figure came from the mock
path. Those figures are still here, relabelled.

## Running Benchmarks

```bash
# Mock-invoker suite only — no extra toolchain needed
cargo bench -p torvyn-benchmarks

# Everything, including the real-Wasm suite (what CI runs)
cargo bench -p torvyn-benchmarks --features real-wasm

# A single benchmark target
cargo bench -p torvyn-benchmarks --features real-wasm --bench real_wasm
cargo bench -p torvyn-benchmarks --bench latency

# Filter by criterion pattern
cargo bench -p torvyn-benchmarks --features real-wasm -- "payload_scaling"
```

The `real-wasm` feature needs the WebAssembly Component toolchain:

```bash
rustup target add wasm32-wasip2
cargo install cargo-component --locked
```

The integration crate's `build.rs` compiles the guest fixtures automatically.
Without the toolchain it emits a `cargo:warning` and skips them, so the
default `cargo bench` still works — the real-Wasm arms simply do not run.

## Benchmark Descriptions

### Real-Wasm suite — `real_wasm` (feature `real-wasm`)

Drives three `cargo component`-built guests through the production engine,
invoker, buffer pool, and flow driver. Per element, a Source → Processor →
Sink flow runs three guest calls and records four payload copies: the source
writes its output buffer, the processor reads its input and writes a fresh
output, and the sink reads the payload. Source → Sink drops the processor and
its two copies.

| Group | Measures |
|---|---|
| `real_wasm_latency` | Per-element cost for both shapes at 100 / 1 000 / 10 000 elements, 256-byte payload |
| `real_wasm_payload_scaling` | Source → Processor → Sink, 1 000 elements, payloads of 8 B / 256 B / 4 KiB / 64 KiB — spanning the `Small`, `Medium`, and `Large` pool tiers |
| `real_wasm_throughput` | Sustained elements/second for both shapes, 10 000 elements |
| `real_wasm_instantiation` | Pipeline startup on a warm engine: instantiate + `lifecycle.init` for every stage, compilation already cached |

**Self-validating.** Every configuration runs once through
`WasmFixtures::validate` before it is measured, asserting clean completion,
the exact element count, the exact copy count and copied-byte total for the
shape, and that the buffer pool returns to zero outstanding buffers. A
configuration that cannot satisfy those invariants fails the benchmark
instead of reporting a fast number for a run that dropped work.

### Mock-invoker suite

#### `latency` — Per-Element Latency

End-to-end latency for flowing N elements through a Source → Sink topology.

**Configurations:** 100, 1,000, 10,000 elements.

#### `throughput` — Sustained Elements/Second

Maximum sustained throughput for two topologies:
- **Source → Sink:** Direct pass-through (baseline)
- **Source → Processor → Sink:** Single-stage identity transformation

**Configuration:** 100,000 elements per iteration.

#### `copy_accounting` — Copy Ledger Overhead

Measures the copy-accounting infrastructure's own cost:
- Ledger operation overhead (recording 1,000 elements × 2 copies each)
- Source → Sink flow execution
- Source → Processor → Sink flow execution

Note that the mock invoker fabricates buffer handles rather than allocating
real buffers, so this measures the *ledger*, not the copies. Real copy costs
are in `real_wasm_payload_scaling`.

#### `memory` — Peak Memory Under Load

Verifies bounded memory growth under backpressure:
- Small queue depth (16) with slow sink to force backpressure
- Default queue depth with 100K elements for throughput baseline

Backpressure should prevent unbounded queue buildup (no OOM).

### `comparison/grpc_comparison` — gRPC Localhost Baseline

Three arms, side by side:

| Arm | Executes |
|---|---|
| `torvyn_real_wasm` | Source → Processor → Sink through real Wasm, 256-byte payload (feature-gated) |
| `torvyn_mock_invoker` | Source → Sink through the mock invoker — the overhead floor |
| `grpc_unary_localhost` | Tonic unary echo over loopback TCP: protobuf encode/decode, HTTP/2 framing, kernel round trip, 256 bytes |

Only `torvyn_real_wasm` may be quoted as "Torvyn vs. gRPC". The mock arm
crosses no component boundary and moves no payload bytes, so a ratio against
it compares Torvyn's scheduling overhead to gRPC's *entire* transport cost.

The real-Wasm arm uses the three-stage shape because a gRPC unary echo is
client-send → server-handler → client-receive, and Source → Processor → Sink
is its closest analogue. It is also the more conservative choice: the
three-stage flow costs more than the two-stage one, so the comparison is not
picking Torvyn's cheapest shape.

The gRPC server is in-process on a random `127.0.0.1` port. The protobuf
compiler is bundled hermetically via `protoc-bin-vendored`, so `cargo bench`
works without system protobuf tooling.

`TCP_NODELAY` is set explicitly on both ends. This matters more than it
sounds: `serve_with_incoming` hands tonic already-accepted sockets, so
tonic's own `tcp_nodelay` setting never reaches them, and Nagle's algorithm
on the server plus the client's delayed-ACK timer stalls every round trip
until the timer fires. Before this was fixed the baseline measured a flat
~41 ms per call on Linux CI runners — the 40 ms Linux delayed-ACK timeout,
not gRPC — while the same code ran in ~53 µs on macOS. A baseline that is
accidentally 800× too slow makes the thing being compared against it look
correspondingly good, which is the opposite of what a baseline is for.

## Interpreting Results

Criterion produces HTML reports in `target/criterion/`. Open
`target/criterion/report/index.html` for the full report.

| Metric | What It Means |
|--------|---------------|
| **time** | Wall-clock time per iteration (lower is better) |
| **thrpt** | Throughput in elements/second — or bytes/second for the payload sweep (higher is better) |
| **change** | Percentage change from the previous run in the same working tree |

## Methodology

### What is inside the clock

For the real-Wasm hot-path groups, **only `RealFlow::run`**. Each iteration
instantiates a fresh set of guests outside the measured region (criterion's
`iter_custom`), so the reported time is per-element pipeline work, not
per-element work plus amortised startup. Instantiation is not free and is not
hidden: `real_wasm_instantiation` publishes it separately.

This mirrors the gRPC arm, where the server and client channel are created
once per group and each iteration measures only the calls. Both sides
therefore report steady-state cost against warm infrastructure.

The mock-invoker benchmarks keep their original structure, with
`build_driver` inside the timed region — for the mock engine that setup is
negligible next to the element loop. When comparing mock and real numbers,
prefer the 10 000-element rows, where per-iteration setup is negligible on
both sides.

### Statistical rigor

All benchmarks use criterion, with a 3 s warmup, linear-regression estimation
over bootstrap confidence intervals, and automatic outlier classification.
Per-target sampling:

| Target | Samples | Measurement window |
|---|---:|---:|
| `latency` | 50 | 10 s |
| `throughput` | 20 | 15 s |
| `real_wasm` (all four groups) | 20 | 8 s |
| `grpc_comparison` latency | 20 | 10 s |
| `grpc_comparison` throughput | 20 | 15 s |
| `copy_accounting`, `memory` | 100 (criterion default) | 5 s (default) |

The real-Wasm groups sample less than `latency` does because each iteration is
roughly an order of magnitude more expensive; the wall-clock budget is what
keeps the CI benchmark job inside its timeout. `grpc_comparison` samples less
than `latency` for the same reason — it now runs three arms per element count,
and its 10 000-element gRPC arm is the slowest single benchmark in the suite.

### Reproducibility

1. Close other applications; benchmark on an idle machine.
2. Disable CPU frequency scaling where possible.
3. Run more than once and compare reports.
4. `cargo bench -- --save-baseline <name>` then
   `cargo bench -- --baseline <name>` to compare across changes.

## Phase 0 Targets

| Benchmark | Target | Notes |
|-----------|--------|-------|
| Per-element runtime overhead | < 5 µs | Excludes component execution time |
| Source → Sink throughput | > 1 M elem/s | Mock invoker, no serialization |
| Source → Processor → Sink throughput | > 500 K elem/s | Identity processor |
| Copy accounting overhead | < 100 ns per record | Atomic operations only |
| Pipeline startup | Sub-second for cached components | Wasmtime compilation cached by `ComponentTypeId` |
| Memory (backpressure) | Bounded by queue depth | No OOM under sustained load |

The throughput targets were written against the mock-invoker path and are met
there. The real-Wasm path is roughly an order of magnitude slower per element,
which is the cost of actually executing sandboxed guests and moving bytes
across the Canonical ABI; see the measured numbers below.

## Measured Baseline (Phase 0)

### Run conditions

| | |
|---|---|
| CPU | Apple M5 Max (18 cores), `aarch64-apple-darwin` |
| OS | macOS 26.4 (Darwin 25.x) |
| Rust | 1.97.0 stable |
| `cargo-component` | 0.21.1 |
| Profile | `bench` (inherits `release`: LTO, single codegen unit) |
| Command | `cargo bench -p torvyn-benchmarks --features real-wasm` |
| Conditions | Single-tenant developer machine, light background load |

Every figure below is criterion's **median point estimate** from one full run.
Numbers will differ on Linux CI runners, on other hardware, and under load.
They are representative figures, not guaranteed SLOs. Reproduce with the
command above and compare against your own hardware.

### Real Wasm — what a pipeline costs

Three `cargo component`-built guests, 256-byte payloads, per-iteration
instantiation excluded (see *What is inside the clock*).

| Topology | 100 elem | 1 000 elem | 10 000 elem | Steady-state per element |
|---|---:|---:|---:|---:|
| Source → Sink (2 copies/elem) | 281 µs | 2.65 ms | 26.6 ms | **~2.66 µs** |
| Source → Processor → Sink (4 copies/elem) | 486 µs | 4.52 ms | 45.0 ms | **~4.50 µs** |

| Topology | Sustained throughput |
|---|---:|
| Source → Sink | **387 K elem/s** |
| Source → Processor → Sink | **228 K elem/s** |

Pipeline startup, compilation already cached:

| Stages | Instantiate + `lifecycle.init` |
|---|---:|
| 2 (Source → Sink) | 84.8 µs |
| 3 (Source → Processor → Sink) | 125 µs |

### Real Wasm — payload scaling

Source → Processor → Sink, 1 000 elements, four copies per element. "Copy
traffic" is the total payload bytes the resource manager actually moved
(`4 × payload × 1 000`), which is what the copy ledger counts.

| Payload | Pool tier | Per element | Copy traffic | Aggregate copy bandwidth |
|---:|---|---:|---:|---:|
| 8 B | `Small` | 4.40 µs | 32 KB | — (overhead-dominated) |
| 256 B | `Small` | 4.43 µs | 1.02 MB | 231 MB/s |
| 4 KiB | `Medium` | 5.01 µs | 16.4 MB | 3.27 GB/s |
| 64 KiB | `Large` | 16.2 µs | 262 MB | 16.2 GB/s |

**The headline result of this sweep:** per-element cost is essentially flat
from 8 B to 4 KiB. Growing the payload 512× adds 0.61 µs — 14% — because the
copies are running out of cache at memory bandwidth while a fixed ~4.4 µs of
guest-call and scheduling cost dominates. Only at 64 KiB do the four copies
become the bill, and there they sustain 16 GB/s.

So the four-copies-per-element design is not what limits small-element
throughput; boundary crossings are. That is an actionable finding, and it is
the number this suite existed to produce.

### Mock invoker — the runtime's overhead floor

No WebAssembly executed, no payload bytes moved. This is the cost of the
queue, the scheduler, and the copy ledger alone.

| Benchmark | Median | Per element |
|---|---:|---:|
| `throughput` Source → Sink (100 K elements) | 47.3 ms | 473 ns → 2.11 M elem/s |
| `throughput` Source → Processor → Sink (100 K elements) | 75.7 ms | 757 ns → 1.32 M elem/s |
| `latency` Source → Sink (10 K elements) | 4.31 ms | 431 ns |
| `copy_accounting` 1 000 ledger records | 12.3 µs | 12.3 ns per record |

Real Wasm costs **5.5× the floor** for Source → Sink and **5.8×** for
Source → Processor → Sink. That multiple is the price of sandboxed execution
and real byte movement.

### Torvyn vs. gRPC unary localhost

All three arms in one criterion group: same Tokio runtime, same element
counts, and — for the two arms that carry a payload — the same 256 bytes.
Figures are the 10 000-element rows.

| Arm | Per element | Throughput | vs. gRPC |
|---|---:|---:|---:|
| `torvyn_real_wasm` (Source → Processor → Sink) | 4.40 µs | 227 K elem/s | **10.6× faster** |
| `torvyn_mock_invoker` (overhead floor) | 425 ns | 2.35 M elem/s | 110× — *not a like-for-like ratio* |
| `grpc_unary_localhost` | 46.7 µs | 21.4 K elem/s | 1× |

The throughput group agrees: 4.33 µs vs 46.3 µs, a 10.7× ratio.

**`10.6×` is the number to quote.** The 110× figure compares Torvyn's
scheduling overhead against gRPC's entire transport cost — protobuf
encode/decode, HTTP/2 framing, and a loopback round trip — while executing no
component code and moving no bytes. It is a real measurement of a real thing,
and it is not "Torvyn vs. gRPC".

### Targets vs. measured

| Target | Measured | Verdict |
|---|---:|---|
| Per-element runtime overhead < 5 µs (excludes component execution) | 757 ns | Met, 6.6× under. This is the mock path, which is precisely "runtime overhead excluding component execution". |
| Copy accounting overhead < 100 ns per record | 12.3 ns | Met, 8.1× under |
| Pipeline startup sub-second for cached components | 125 µs | Met, ~8 000× under |
| Exactly 4 copies per element (Source → Processor → Sink) | 4 | Met, and asserted by every real-Wasm benchmark before it is measured |
| Source → Sink throughput > 1 M elem/s | 2.11 M mock / **387 K real** | Met on the overhead floor; **not met on the real-Wasm path** |
| Source → Processor → Sink throughput > 500 K elem/s | 1.32 M mock / **228 K real** | Met on the overhead floor; **not met on the real-Wasm path** |

The two throughput targets were written against the mock-invoker path and are
stated in this repository without that qualification. On the path that
actually runs WebAssembly, single-flow throughput is 387 K and 228 K
elements/second. Whether to restate the targets against the real path or to
treat 1 M / 500 K as an optimisation goal is a project decision; publishing
the measurement is not.

Worth noting: the architecture documentation's own estimate for the Wasm
boundary — "~5-15 µs, ~200 K-500 K elem/s" in
`docs/src/examples/benchmark-comparison.md` — matches these measurements. The
design docs predicted this correctly; only the published headline figures did
not reflect it.

### How to reproduce

```bash
# Full run, both paths (~15 minutes including a cold LTO build)
cargo bench -p torvyn-benchmarks --features real-wasm

# Then check nothing regressed
cargo run -p torvyn-benchmarks --release --features real-wasm --bin check-thresholds

# Just the real-Wasm suite
cargo bench -p torvyn-benchmarks --features real-wasm --bench real_wasm

# Narrow to one group — the fastest way to sanity-check a change
cargo bench -p torvyn-benchmarks --features real-wasm --bench real_wasm -- \
  real_wasm_payload_scaling
```

Note that every group in this suite sets its own sample size and measurement
window in code, which takes precedence over criterion's `--sample-size` and
`--measurement-time` flags. To shorten a run, narrow it with `--bench` and a
filter rather than passing those flags. Set `CRITERION_HOME` to keep an
exploratory run from overwriting a baseline you care about.

Criterion HTML reports land in `target/criterion/`; open
`target/criterion/report/index.html` for distributions, percentiles, and
change-vs-baseline plots.

## Regression gate

`cargo bench` on its own does not fail on a regression. The `check-thresholds`
binary does:

```bash
cargo bench -p torvyn-benchmarks --features real-wasm
cargo run  -p torvyn-benchmarks --release --features real-wasm --bin check-thresholds
```

It reads criterion's `estimates.json` output, compares each benchmark's median
against the ceiling committed in [`thresholds.json`](thresholds.json), and
exits non-zero if any benchmark is missing or over budget. A missing benchmark
is a failure too — that is what catches a CI run that silently skipped the
real-Wasm arms.

CI runs both steps in the `Benchmark` job on pushes to `main`, so a regression
is caught post-merge rather than on the pull request; the full criterion
reports are uploaded as an artifact whether the gate passes or fails.

**The ceilings are order-of-magnitude detectors, not SLOs.** They are set
several times above the numbers a developer machine produces, because they
have to pass on shared CI runners whose per-core throughput varies widely. A
30% drift will not trip them; a structural regression will. The measured
tables above are the real baseline to compare against. Raise a ceiling only
alongside a deliberate, explained change in what the runtime does.

## Adding a Benchmark

1. Create `benches/<name>.rs` (or add a group to an existing file).
2. Register it in `Cargo.toml`:
   ```toml
   [[bench]]
   name = "<name>"
   harness = false
   # add this if it drives real Wasm:
   # required-features = ["real-wasm"]
   ```
3. Use `criterion_group!` / `criterion_main!`.
4. For real-Wasm benchmarks, drive the harness in
   [`src/real_wasm.rs`](src/real_wasm.rs) and time via `timed_runs` so every
   benchmark measures the same region — and call `WasmFixtures::validate` on
   each configuration first.
5. Add a ceiling to `thresholds.json` if the benchmark should gate CI.
6. Document it in this README.
