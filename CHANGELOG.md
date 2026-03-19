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
- WIT contract definitions: `torvyn:streaming@0.1.0` package with split `buffer` / `mutable-buffer` resource model.
- Project scaffolding: `torvyn init` generates component projects with WIT contracts, Cargo configuration, and starter implementations.
- CI pipeline: build, test, lint (`clippy`), format check (`rustfmt`), MSRV verification, benchmark regression detection.

### Changed
- Nothing yet — this is the initial release.

### Deprecated
- Nothing yet.

### Removed
- Nothing yet.

### Fixed
- Nothing yet.

### Security
- Nothing yet.

---

## [0.1.0] — Unreleased

Phase 0 initial release. See the [Added] section above for the complete feature set.

Target: first working Source-to-Sink pipeline with backpressure, tracing, and benchmarks.

[Unreleased]: https://github.com/torvyn/torvyn/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/torvyn/torvyn/releases/tag/v0.1.0
