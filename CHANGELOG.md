# Changelog

All notable changes to Torvyn are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Torvyn uses the following versioning policy:
- **Major** versions indicate breaking changes to WIT contracts or public Rust APIs.
- **Minor** versions add functionality in a backward-compatible manner.
- **Patch** versions contain backward-compatible bug fixes.
- **Pre-release** identifiers (e.g., `0.1.0-alpha.1`) signal that APIs are not yet stable.

---

## [Unreleased]

### Added
- `torvyn-types` crate: shared identity types (`ComponentId`, `FlowId`, `ResourceId`), `ProcessError` enum, `FlowState` state machine, `ComponentRole` enum, and all foundational constants.
- `torvyn-contracts` crate: WIT package definitions for `torvyn:streaming@0.1.0` including `types`, `source`, `processor`, `sink`, and `lifecycle` interfaces. `wit-parser` integration for contract validation.
- `torvyn-engine` crate: `WasmEngine` trait with `WasmtimeEngine` implementation. `ComponentInvoker` trait with typed `invoke_pull`, `invoke_process`, `invoke_push`, `invoke_init`, and `invoke_teardown` methods. Wasmtime resource type integration.
- `torvyn-resources` crate: resource table with generational indices, buffer pool (Small and Medium tiers), ownership state machine (Free to Owned to Transit to Borrowed to Free), copy accounting per element.
- `torvyn-observability` crate: Production-level counters and histograms with pre-allocated metric handles. Basic OTLP trace export. `EventSink` trait for hot-path event recording.
- `torvyn-reactor` crate: minimal single-flow driver, FIFO scheduling, bounded queue with high/low watermark backpressure, demand propagation, cooperative yield.
- `torvyn-linker` crate: two-component topology linking with WIT contract compatibility checking.
- `torvyn-pipeline` crate: pipeline topology construction and validation from TOML configuration.
- `torvyn-security` crate: deny-all-by-default capability model, `SandboxConfigurator` for per-component Wasm sandbox configuration.
- `torvyn-host` crate: runtime binary entry point — startup, pipeline instantiation, graceful shutdown.
- `torvyn-cli` crate: `torvyn init`, `torvyn check`, `torvyn run` commands.
- Benchmark suite: Source-to-Sink latency and throughput measurement, comparison harness for gRPC localhost baseline.
- Real-Wasm benchmark suite (`cargo bench -p torvyn-benchmarks --features real-wasm`): per-element latency and throughput for Source-to-Sink and Source-to-Processor-to-Sink through `WasmtimeEngine` and `WasmtimeInvoker`, a payload sweep across three buffer-pool tiers (8 B / 256 B / 4 KiB / 64 KiB), and pipeline instantiation cost on a warm engine. Every configuration is validated for clean completion, exact copy count, exact copied-byte total, and zero outstanding buffers before it is measured.
- Benchmark regression gate: `check-thresholds` compares criterion's medians against ceilings committed in `benches/thresholds.json` and fails CI on a regression.
- `echo-source` test component accepts a `payload_bytes` field in its `lifecycle.init` config (default 8, preserving the previous behaviour), which is what lets the benchmark sweep payload sizes.
- Per-element tracing. At `Diagnostic` level the reactor now records a span per component invocation, keyed by the element's origin sequence, into a per-flow ring buffer owned by the observability collector. `EventSink` gains `record_element_span` and `flow_trace_context`, both defaulting to no-ops so existing sinks are unaffected. Below `Diagnostic` the recording path costs one atomic load.
- `StreamElementRef::origin_sequence`: the sequence an element is assigned at the source, carried unchanged through every stage. The per-stream `sequence` is reassigned at each emit site, so it cannot identify an element end to end; this can, which is what lets a trace group a source's, a processor's, and a sink's spans into one element's journey.
- Components can read their flow's W3C trace context: `flow-context.trace-id()`, `span-id()`, and `flow-id()` return the real identifiers the host records the flow's spans under, so a component's own telemetry can be correlated with the host's trace. They previously returned empty strings.
- `HostBuilder::with_collector_config` supplies the observability collector's configuration directly, for callers needing settings the `[observability]` table does not expose.
- `FlowRecord::stages` records each stage's `ComponentId`, node name, and role, so a report can name a component rather than printing its positional id.
- WIT contract definitions: `torvyn:streaming@0.1.0` package with split `buffer` / `mutable-buffer` resource model.
- Project scaffolding: `torvyn init` generates component projects with WIT contracts, Cargo configuration, and starter implementations.
- CI pipeline: build, test, lint (`clippy`), format check (`rustfmt`), MSRV verification, benchmark regression detection.

### Changed
- Published performance numbers in `README.md` and `benches/README.md` are now measured on the real-Wasm path. The previous figures came from the mock-invoker path, which executes no WebAssembly and moves no payload bytes; they are retained, relabelled as the runtime's overhead floor, and no longer presented as the cost of running a pipeline.
- Span ids are derived from `(flow_id, component_id, element_sequence)` rather than generated from the current nanosecond. The previous generator seeded itself from `SystemTime::now`, so two spans created within the same nanosecond received identical ids — on the hot path, not a hypothetical. Derivation is also allocation-free and reads no clock.
- The gRPC comparison (`comparison/grpc_comparison.rs`) gained a real-Wasm arm carrying the same 256-byte payload as the gRPC arm, making it a like-for-like measurement. Its criterion groups were renamed to `torvyn_vs_grpc_latency` and `torvyn_vs_grpc_throughput`, with each arm named for what it actually executes.

### Deprecated
- Nothing yet.

### Removed
- Nothing yet.

### Fixed
- `torvyn trace` reported a hardcoded result of all zeros — zero elements, zero latency, zero copies, no spans — after successfully running the pipeline, and did so without an error or warning. It now reports the run: per-element spans grouped into each element's path through the pipeline, with real durations, copy counts, and backpressure figures. It also no longer starts the traced flow twice (it called `start_flow` and then `TorvynHost::run`, which starts every flow in the manifest).
- Per-component processing time was recorded as zero for every invocation of every component. `FlowDriver::execute_stage` passed `start.elapsed()` and `(start + duration).elapsed()` to `EventSink::record_invocation`, which measure time *since* an instant rather than points in time; the resulting `end_ns` was smaller than `start_ns`, so every sink computing `end_ns - start_ns` saturated to zero. Both are now absolute epoch timestamps, derived from a per-flow anchor so the hot path reads no wall clock. The bucketed percentile masked this — a histogram of nothing but zeros still reports a non-zero p95 by interpolating inside its first bucket — so the regression tests assert on the mean.
- `torvyn trace --show-buffers` was accepted and silently ignored, along with `--limit`, `--trace-format`, `--output-trace`, and `--show-backpressure`. The latter four now work; `--show-buffers` exits with an error explaining that the runtime does not retain payload bytes after an element is consumed.
- gRPC benchmark baseline: sockets accepted by the in-process echo server kept Nagle's algorithm enabled, because `serve_with_incoming` bypasses tonic's own socket configuration. Combined with the client's delayed-ACK timer this stalled every unary round trip until the timer fired — a flat ~41 ms per call on Linux CI runners, against ~53 us on macOS. The baseline was measuring a kernel timer rather than gRPC's transport cost, and the CI benchmark job timed out before finishing. `TCP_NODELAY` is now set explicitly on both ends.

### Security
- Nothing yet.

---

## [0.1.0] — Unreleased

Phase 0 initial release. See the [Added] section above for the complete feature set.

Target: first working Source-to-Sink pipeline with backpressure, tracing, and benchmarks.

[Unreleased]: https://github.com/torvyn/torvyn/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/torvyn/torvyn/releases/tag/v0.1.0
