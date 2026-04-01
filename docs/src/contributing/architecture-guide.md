# Architecture Guide

This document explains how the Torvyn codebase is organized, what each crate does, how data flows through the system, and where to look when you want to change something. It is written for experienced Rust developers who are evaluating the codebase for the first time.

## Crate Dependency Graph

Torvyn is structured as a Cargo workspace with 14 crates. Dependencies flow strictly downward — there are no circular dependencies. `torvyn-types` is the universal leaf: every other crate depends on it.

```
torvyn-cli ─────────────────────────────────────────────────────────┐
torvyn-host ────────────────────────────────────────────────────────┤
  ↑                                                                 │
torvyn-pipeline ── torvyn-packaging                                 │
  ↑                    ↑                                            │
torvyn-linker ──── torvyn-reactor                                   │
  ↑                    ↑                                            │
torvyn-resources ─ torvyn-security                                  │
  ↑                    ↑                                            │
torvyn-engine ─── torvyn-observability ── torvyn-config ── torvyn-contracts
  ↑                    ↑                      ↑               ↑     │
  └────────────────────┴──────────────────────┴───────────────┘     │
                            torvyn-types ◄──────────────────────────┘
```

Reading from bottom to top:

1. **`torvyn-types`** sits at the base. It defines all shared identity types, error enums, state machines, and constants. No internal dependencies, no unsafe code.
2. **`torvyn-contracts`**, **`torvyn-config`**, **`torvyn-engine`**, and **`torvyn-observability`** form the second tier. They depend only on `torvyn-types` (and, in some cases, on each other within this tier).
3. **`torvyn-resources`** and **`torvyn-security`** form the third tier.
4. **`torvyn-linker`** and **`torvyn-reactor`** form the fourth tier. These are the core composition and execution subsystems.
5. **`torvyn-pipeline`** and **`torvyn-packaging`** form the fifth tier.
6. **`torvyn-host`** and **`torvyn-cli`** sit at the top. They are thin orchestration shells that wire together the lower crates.

## What Each Crate Does

### `torvyn-types` — Shared Foundation Types
**Path:** `crates/torvyn-types/`
**Unsafe code:** Forbidden (`#![forbid(unsafe_code)]`)

The universal leaf dependency. Defines all identity types (`ComponentId`, `FlowId`, `StreamId`, `ResourceId`, `BufferHandle`, `TraceId`, `SpanId`), all shared error enums (`ProcessError`, `TorvynError`, `ContractError`, `LinkError`, `ResourceError`, `ReactorError`, `ConfigError`, `SecurityError`, `PackagingError`), domain enumerations (`ComponentRole`, `BackpressureSignal`, `BackpressurePolicy`, `ObservabilityLevel`), state machines (`FlowState`, `ResourceState`), shared records (`ElementMeta`, `TransferRecord`, `TraceContext`), the `EventSink` trait for observability, and project-wide constants. Contains 48 public items total. Zero external dependencies beyond `serde`.

### `torvyn-contracts` — WIT Contracts & Validation
**Path:** `crates/torvyn-contracts/`

Owns the canonical WIT package files that define Torvyn's component interfaces (`torvyn:streaming@0.1.0`, `torvyn:lifecycle@0.1.0`, `torvyn:capabilities@0.1.0`). Provides WIT parsing (wrapping `wit-parser` behind a trait for API isolation), contract validation, semantic compatibility checking between contract versions, and static linking verification. This is the foundation of Torvyn's contract-first architecture — every component interaction is defined through the WIT definitions owned by this crate.

### `torvyn-config` — Configuration Parsing & Schemas
**Path:** `crates/torvyn-config/`
**Unsafe code:** Forbidden

Implements the two-configuration-context model: component manifests (`Torvyn.toml` per component project) and pipeline definitions (topology, per-component overrides, scheduling, runtime settings). Handles TOML/JSON parsing, environment variable interpolation, configuration merging and layering, and cross-field semantic validation. All configuration in the Torvyn project flows through this crate.

### `torvyn-engine` — Wasm Engine Abstraction
**Path:** `crates/torvyn-engine/`

Abstracts over the WebAssembly runtime. Defines the `WasmEngine` trait (compile, instantiate, configure) and provides a `WasmtimeEngine` implementation that wraps Wasmtime. Also defines the `ComponentInvoker` trait with typed methods (`invoke_pull`, `invoke_process`, `invoke_push`) that the reactor uses to call into components on the hot path. Manages a `CompiledComponentCache` keyed by `ComponentTypeId` (SHA-256 of the component binary) for compilation deduplication.

### `torvyn-resources` — Buffer Pools & Ownership
**Path:** `crates/torvyn-resources/`
**Unsafe code:** Isolated to the `buffer` module (allocation/deallocation), with `// SAFETY:` comments on every unsafe block.

The central resource registry. Manages tiered buffer pools (Small, Medium, Large, Huge), the resource ownership state machine (`Pooled → Owned → Borrowed → Transit → Pooled`), copy accounting (the `CopyLedger` that tracks every payload copy with its reason), per-component memory budgets (`BudgetRegistry`), and the `ResourceTable` that maps generational `ResourceId` handles to actual buffer entries. This crate is where Torvyn's ownership-aware design is enforced at runtime.

### `torvyn-reactor` — Stream Scheduling & Backpressure
**Path:** `crates/torvyn-reactor/`

The execution subsystem for stream-driven flow. Implements the task-per-flow model (one Tokio task per pipeline execution), bounded queues between pipeline stages, demand-driven scheduling with high/low watermark backpressure, cooperative yield (yield after N elements or M microseconds), cancellation propagation, and timeout enforcement. Defines the `SchedulingPolicy` trait with implementations for FIFO and weighted fair queuing. This is the beating heart of Torvyn's reactive execution.

### `torvyn-observability` — Tracing, Metrics & Diagnostics
**Path:** `crates/torvyn-observability/`

Implements the three-level observability system: Off (zero overhead), Production (counters, histograms, flow-level traces with a budget of 500ns per element), and Diagnostic (per-element spans, full resource lifecycle tracing). Provides the `EventSink` implementation that the hot path uses for non-blocking event recording, metrics collection handles, and OTLP trace export. All observability in Torvyn routes through this crate.

### `torvyn-security` — Capability Model & Sandboxing
**Path:** `crates/torvyn-security/`
**Unsafe code:** Forbidden

Defines the capability taxonomy (what permissions a component can request), capability declaration and resolution (matching component requirements against operator grants), sandbox configuration (WASI permissions, fuel limits, memory limits), runtime enforcement guards, and audit logging. Implements the deny-all-by-default security model: components receive only the capabilities explicitly granted to them.

### `torvyn-linker` — Component Linking & Compatibility
**Path:** `crates/torvyn-linker/`

Performs static linking verification: checks that components in a pipeline have compatible WIT interfaces, resolves capability grants, detects cyclic dependencies, and produces a `LinkedPipeline` that the pipeline crate uses for instantiation. Generates rich diagnostic reports (`LinkReport` with `LinkDiagnostic` entries) when linking fails, so developers get actionable error messages.

### `torvyn-pipeline` — Pipeline Topology & Instantiation
**Path:** `crates/torvyn-pipeline/`

Constructs pipeline topologies from configuration, validates topology constraints (source must be first, sink must be last, all edges are connected), and orchestrates component instantiation through an `InstantiationContext` that coordinates the engine, resources, security, and observability subsystems. Produces a `PipelineHandle` that the host uses to manage the running pipeline.

### `torvyn-packaging` — OCI Artifacts & Distribution
**Path:** `crates/torvyn-packaging/`

Assembles components into OCI-compatible artifacts (`.torvyn` archives containing the Wasm binary, manifest, contract metadata, and provenance). Handles artifact signing (via a `SigningProvider` trait, with a Sigstore implementation planned for Phase 2), content-addressed storage, local caching, and registry push/pull operations (via a `RegistryClient` trait that abstracts over direct OCI API calls and CLI fallbacks).

### `torvyn-host` — Runtime Entry Point
**Path:** `crates/torvyn-host/`

A thin orchestration shell. Wires together the engine, resources, reactor, observability, and security subsystems into a running `TorvynHost`. Manages the startup sequence (parse config → validate contracts → link → compile → instantiate → start flow), graceful shutdown with configurable drain timeouts, and runtime inspection. This crate contains minimal logic of its own — it delegates to the subsystem crates.

### `torvyn-cli` — Developer CLI
**Path:** `crates/torvyn-cli/`
**Unsafe code:** Forbidden

Produces the `torvyn` binary. Implements all developer-facing commands: `init` (project scaffolding), `check` (contract and config validation), `link` (static compatibility verification), `run` (pipeline execution with diagnostics), `trace` (execution with full tracing), `bench` (latency, throughput, queue pressure, copy behavior measurement), `pack` (OCI artifact assembly), `publish` (registry upload), `inspect` (artifact metadata inspection), and `doctor` (environment diagnostics). Built with `clap` for argument parsing and `miette` for rich error display.

## Key Types Glossary

These are the 20 types you will encounter most often when working in the codebase. Understanding them is essential for navigating the code.

| Type | Crate | What It Is |
|------|-------|-----------|
| `ComponentId` | `torvyn-types` | Alias for `ComponentInstanceId`. Runtime identity for a component instance, assigned at instantiation. A `u64`, monotonically increasing, never reused. |
| `ComponentTypeId` | `torvyn-types` | Content-addressed identity for a compiled component artifact. A `[u8; 32]` SHA-256 digest. Used for compilation caching. |
| `FlowId` | `torvyn-types` | Unique identifier for a flow (a running pipeline instance). A `u64`. |
| `StreamId` | `torvyn-types` | Identifies a specific stream (queue) between two stages in a flow. A `u64`. |
| `ResourceId` | `torvyn-types` | Generational index into the resource table: `{ index: u32, generation: u32 }`. The generation field prevents use-after-free on recycled slots. |
| `BufferHandle` | `torvyn-types` | Opaque wrapper around `ResourceId`. This is what components receive as a handle to host-managed buffers. |
| `ProcessError` | `torvyn-types` | Rust mapping of the WIT `process-error` variant. Five variants: `InvalidInput`, `Unavailable`, `Internal`, `DeadlineExceeded`, `Fatal`. |
| `FlowState` | `torvyn-types` | State machine for flow lifecycle. Eight states: `Created → Validated → Instantiated → Running → Draining → Completed`, with `Suspended` and `Failed` as alternative terminal/intermediate states. |
| `ResourceState` | `torvyn-types` | State machine for buffer ownership. States: `Pooled`, `Owned`, `Borrowed`, `Transit`, `Dropped`. |
| `ElementMeta` | `torvyn-types` | Metadata attached to every stream element: sequence number, timestamp, content type. |
| `ComponentRole` | `torvyn-types` | Enum: `Source`, `Processor`, `Sink`, `Filter`, `Router`. |
| `BackpressureSignal` | `torvyn-types` | Signal returned by sink components: `Accept`, `Throttle`, `Pause`. |
| `TraceContext` | `torvyn-types` | W3C-compatible trace and span IDs for distributed tracing. |
| `WasmtimeEngine` | `torvyn-engine` | Concrete implementation of the `WasmEngine` trait wrapping Wasmtime. |
| `CompiledComponent` | `torvyn-engine` | A compiled Wasm component ready for instantiation. Cached by `ComponentTypeId`. |
| `ComponentInstance` | `torvyn-engine` | A live, instantiated component with its Wasmtime store and import bindings. |
| `ResourceTable` | `torvyn-resources` | The generational-arena data structure that maps `ResourceId` to `ResourceEntry`. |
| `BufferPoolSet` | `torvyn-resources` | Manages the tiered buffer pools (Small, Medium, Large, Huge). |
| `BoundedQueue<T>` | `torvyn-reactor` | The inter-stage queue with capacity limits, used for backpressure enforcement. |
| `TorvynHost` | `torvyn-host` | The top-level runtime struct that coordinates all subsystems. |

## Key Traits Glossary

These are the 10 traits that define the major abstraction boundaries in the codebase.

| Trait | Crate | Purpose | Key Implementors |
|-------|-------|---------|-----------------|
| `WasmEngine` | `torvyn-engine` | Abstraction over the Wasm runtime. Methods: compile, instantiate, configure engine. | `WasmtimeEngine` |
| `ComponentInvoker` | `torvyn-engine` | Typed hot-path invocation: `invoke_pull`, `invoke_process`, `invoke_push`. The reactor calls these to execute component logic. | `WasmtimeInvoker` |
| `EventSink` | `torvyn-types` | Non-blocking trait for recording observability events on the hot path. Must not allocate. | `ObservabilityCollector`, `NoopEventSink` |
| `SchedulingPolicy` | `torvyn-reactor` | Determines intra-flow stage execution order. | `DemandDrivenPolicy`, `FifoPolicy`, `WeightedFairPolicy` |
| `ResourceManager` | `torvyn-resources` | Buffer lifecycle: allocate, transfer ownership, create borrows, release, reclaim. | `DefaultResourceManager` |
| `SandboxConfigurator` | `torvyn-security` | Produces `SandboxConfig` from a component manifest and operator capability grants. | `DefaultSandboxConfigurator` |
| `AuditSink` | `torvyn-security` | Records security-relevant events (capability checks, sandbox violations). | `FileAuditSink`, `EventSinkAdapter` |
| `WitParser` | `torvyn-contracts` | Abstracts over WIT file parsing. Isolates `wit-parser` API churn behind a stable internal interface. | `WitParserImpl` |
| `RegistryClient` | `torvyn-packaging` | Abstracts OCI registry operations (push, pull, tag, list). | Direct OCI client impl, CLI fallback impl |
| `SigningProvider` | `torvyn-packaging` | Abstracts artifact signing (Sigstore planned, stub for Phase 0). | `StubSigningProvider` |

## Hot Path Walkthrough: Following a Stream Element

This walkthrough traces a single stream element through a Source → Processor → Sink pipeline. This is the hot path — the code that runs for every element and where performance matters most.

**Step 1: Source produces an element.** The reactor's flow driver task calls `ComponentInvoker::invoke_pull()` on the source component. The source writes data into a `mutable-buffer` resource and returns an `output-element` containing the `BufferHandle` and `ElementMeta`.

**Step 2: Ownership transfer.** The resource manager transitions the buffer from `Owned` (by the source component) to `Transit`. The copy ledger records this transfer.

**Step 3: Enqueue.** The reactor enqueues the element into the `BoundedQueue` between the source stage and the processor stage. The reactor assigns the canonical `sequence` number and `timestamp-ns` to the `ElementMeta` at this point.

**Step 4: Schedule processor.** The scheduling policy checks whether the processor stage has input available AND downstream capacity (the queue to the sink is not full). If both conditions hold, the processor is scheduled to run.

**Step 5: Processor invocation.** The host constructs a `borrow<buffer>` and `borrow<flow-context>` for the input element. The reactor calls `ComponentInvoker::invoke_process()`, passing the borrowed references. The processor reads the input buffer, writes output into a new `mutable-buffer`, and returns a `process-result`.

**Step 6: Post-processing ownership.** The resource manager ends the borrows, transfers the output buffer to `Transit`, and releases the input buffer back to the pool (`Owned → Pooled`). The copy ledger records both the read and the write.

**Step 7: Enqueue to sink.** The output element is enqueued into the processor→sink `BoundedQueue`.

**Step 8: Sink invocation.** The host constructs borrows for the sink. The reactor calls `ComponentInvoker::invoke_push()`. The sink reads the buffer and returns a `BackpressureSignal` (`Accept`, `Throttle`, or `Pause`).

**Step 9: Cleanup and accounting.** The resource manager releases the buffer back to the pool. The observability layer records spans and updates counters/histograms. Per element, this three-stage pipeline produces exactly 4 payload copies: source writes (1), processor reads (1) + writes (1), sink reads (1). All copies are instrumented.

**Step 10: Backpressure propagation.** If the sink returns `Throttle` or `Pause`, the reactor propagates demand signals upstream. If the source→processor queue reaches its high watermark, the source is paused until the queue drains to the low watermark.

## Cold Path Walkthrough: Pipeline Startup

This walkthrough follows the startup sequence from `torvyn run` to a running pipeline.

**Step 1: CLI parses arguments.** The `torvyn-cli` crate parses the `run` command and its flags using `clap`.

**Step 2: Load configuration.** `torvyn-config` loads the pipeline definition (either from `Torvyn.toml` or a standalone `pipeline.toml`), performs environment variable interpolation, merges layered configs, and validates the result.

**Step 3: Validate contracts.** `torvyn-contracts` loads the WIT package files referenced by each component in the pipeline. The validator checks that all WIT definitions parse correctly and that interface versions are compatible.

**Step 4: Link components.** `torvyn-linker` performs static linking: it verifies that every component's imported interfaces are satisfied by another component's exports (or by the host), resolves capability grants from `torvyn-security`, and checks for cyclic dependencies. The output is a `LinkedPipeline` or a `LinkReport` with diagnostics.

**Step 5: Compile Wasm.** `torvyn-engine` compiles each component's Wasm binary using Wasmtime. Compilation results are cached by `ComponentTypeId` (SHA-256 of the binary), so recompilation is skipped for unchanged components.

**Step 6: Configure sandboxes.** `torvyn-security` produces a `SandboxConfig` for each component based on its capability manifest and the operator's capability grants. This includes WASI permissions, fuel limits, and memory limits.

**Step 7: Instantiate components.** `torvyn-pipeline` orchestrates instantiation through `InstantiationContext`. For each component: create a Wasmtime store with the sandbox configuration, instantiate the compiled component, bind host-provided imports (resource manager, observability hooks), and call `lifecycle.init(config)` if the component exports the lifecycle interface.

**Step 8: Construct topology.** `torvyn-pipeline` creates the `PipelineTopology` — the graph of stages connected by `BoundedQueue` instances — and registers it with the reactor.

**Step 9: Start flow.** `torvyn-reactor` spawns a Tokio task for the flow driver. The flow transitions through `Created → Validated → Instantiated → Running`. The pipeline is now processing elements.

## "If You Want to Change X, Look in Y"

This table maps common modification intents to the relevant crate and files.

| If you want to... | Look in... |
|-------------------|-----------|
| Add a new identity type | `torvyn-types/src/identity.rs` |
| Add a new error variant | `torvyn-types/src/error.rs`, then add a `From` impl for `TorvynError` |
| Modify a WIT interface | `torvyn-contracts/wit/` for the `.wit` files, then `torvyn-contracts/src/validator.rs` for validation logic |
| Change configuration schema | `torvyn-config/src/manifest.rs` (component manifest) or `torvyn-config/src/pipeline.rs` (pipeline definition) |
| Change how components are compiled or instantiated | `torvyn-engine/src/` — `WasmtimeEngine` for compilation, `WasmtimeInvoker` for invocation |
| Modify buffer allocation or pooling | `torvyn-resources/src/pool.rs` for pool logic, `torvyn-resources/src/table.rs` for the resource table |
| Change ownership state transitions | `torvyn-types/src/state.rs` for the `ResourceState` machine |
| Modify scheduling or backpressure behavior | `torvyn-reactor/` — `BoundedQueue` for queue logic, `DemandDrivenPolicy` for scheduling |
| Add a new metric or trace span | `torvyn-observability/` for the collectors; update `EventSink` in `torvyn-types` if adding a new event kind |
| Add or change a CLI command | `torvyn-cli/src/` — one module per command |
| Modify capability checking | `torvyn-security/` for the capability taxonomy and enforcement guards |
| Change the linking/compatibility algorithm | `torvyn-linker/` — `PipelineLinker` for the main algorithm, `LinkDiagnostic` for error reporting |
| Change the OCI artifact format | `torvyn-packaging/src/artifact.rs` for format, `torvyn-packaging/src/manifest.rs` for metadata |
| Add a new example pipeline | `examples/` directory at the repository root |

## Concurrency Model

Torvyn uses Tokio's multi-threaded work-stealing runtime. There are no custom OS threads.

The key concurrency primitives are: one Tokio task per flow (the "flow driver"), one reactor coordinator task for flow lifecycle management, background tasks for observability export, and `spawn_blocking` for filesystem I/O (component loading, config parsing).

Inter-flow fairness comes from Tokio's work-stealing scheduler. Intra-flow fairness comes from the reactor's cooperative yield mechanism: a flow driver yields back to Tokio after processing N elements or M microseconds, whichever comes first. Wasmtime's fuel mechanism provides cooperative preemption within Wasm execution.
