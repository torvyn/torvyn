<p align="center">
  <strong>Torvyn</strong>
</p>

<p align="center">
  <em>Ownership-aware reactive streaming runtime for the WebAssembly Component Model</em>
</p>

<p align="center">
  <a href="#quickstart">Quickstart</a> &middot;
  <a href="#architecture">Architecture</a> &middot;
  <a href="#documentation">Docs</a> &middot;
  <a href="#contributing">Contributing</a>
</p>

---

Torvyn is a high-performance reactive streaming runtime written in Rust that enables safe composition of polyglot, sandboxed streaming components on the [WebAssembly Component Model](https://component-model.bytecodealliance.org/). It is designed for workloads where low latency, strong isolation, cross-language portability, and production observability must coexist without compromise.

**The core proposition:** compose typed, sandboxed streaming components on the same node with lower overhead than microservices, stronger isolation than in-process plugins, and full observability built in.

## Why Torvyn

Modern streaming architectures force teams into painful tradeoffs. Microservices provide isolation but impose serialization, network overhead, and operational complexity — even when services are co-located. In-process plugins provide speed but sacrifice safety, sandboxing, and language neutrality. Containers provide packaging standards but are too heavy for fine-grained, low-latency pipelines with dozens of stages.

Torvyn eliminates these tradeoffs through five integrated pillars:

| Pillar | What it means |
|---|---|
| **Contract-first composition** | Every component boundary is defined through [WIT](https://component-model.bytecodealliance.org/design/wit.html) interfaces — explicit, versioned, and machine-checkable. No hidden assumptions. |
| **Ownership-aware transport** | Host-managed buffers with explicit ownership states (Owned, Borrowed, Leased, Pooled). Every copy is bounded, measurable, and observable. |
| **Reactive backpressure** | Credit-based demand propagation, bounded queues with watermark hysteresis, and cancellation as a first-class primitive. |
| **Polyglot sandboxing** | WebAssembly Component Model isolation. Components in Rust, Go, Python, C, Zig, or any language that compiles to Wasm — safely composed in a single runtime. |
| **Production observability** | Per-flow, per-component, per-stream metrics and tracing. OpenTelemetry-native. Three configurable levels: Off, Production (&lt;500ns/element), Diagnostic (&lt;2&mu;s/element). |

## Use Cases

- **Same-node streaming pipelines** — ultra-low-latency data transformation, filtering, enrichment
- **AI inference composition** — token streams, RAG stages, policy filters, content guards with traceable stages
- **Edge-local processing** — event transformation and stream processing at the edge
- **Plugin ecosystems with real sandboxing** — extend applications with untrusted components safely
- **High-frequency service chaining** — replace localhost microservice calls with in-runtime component composition

## Quickstart

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2021)
- [Wasmtime](https://wasmtime.dev/) dependencies (pulled automatically via Cargo)

### Build

```bash
git clone https://github.com/torvyn/torvyn.git
cd torvyn
cargo build
```

### Test

```bash
cargo test
```

### Lint

```bash
cargo clippy -- -D warnings
```

## Architecture

Torvyn is structured as a layered workspace of focused crates. Each layer depends only on layers below it, enforcing a clean dependency graph.

```
                          ┌─────────────┐
                          │  torvyn-cli  │   Developer CLI
                          └──────┬──────┘
                                 │
                          ┌──────┴──────┐
                          │ torvyn-host │   Runtime orchestration
                          └──────┬──────┘
                                 │
              ┌──────────────────┼──────────────────┐
              │                  │                  │
     ┌────────┴────────┐ ┌──────┴──────┐ ┌─────────┴─────────┐
     │ torvyn-pipeline  │ │torvyn-linker│ │ torvyn-packaging   │
     │ Topology & inst. │ │ Linking &   │ │ OCI artifacts &    │
     │                  │ │ resolution  │ │ distribution       │
     └────────┬────────┘ └──────┬──────┘ └───────────────────┘
              │                 │
   ┌──────────┴─────────────────┴──────────────────────┐
   │                                                    │
   │  ┌──────────────┐  ┌───────────────┐  ┌─────────┐ │
   │  │torvyn-reactor│  │torvyn-security│  │torvyn-  │ │
   │  │Scheduling &  │  │Capabilities & │  │observa- │ │
   │  │backpressure  │  │audit          │  │bility   │ │
   │  └──────┬───────┘  └───────────────┘  └─────────┘ │
   │         │                                          │
   │  ┌──────┴───────┐  ┌───────────────┐              │
   │  │torvyn-engine │  │torvyn-        │              │
   │  │Wasm execution│  │resources      │              │
   │  │& compilation │  │Buffers &      │              │
   │  └──────────────┘  │ownership      │              │
   │                    └───────────────┘              │
   │                                                    │
   │  ┌──────────────┐  ┌───────────────┐              │
   │  │torvyn-config │  │torvyn-        │              │
   │  │Manifests &   │  │contracts      │              │
   │  │pipeline defs │  │WIT validation │              │
   │  └──────────────┘  └───────────────┘              │
   │                                                    │
   │                 ┌──────────────┐                   │
   │                 │ torvyn-types │                   │
   │                 │ Foundation   │                   │
   │                 └──────────────┘                   │
   └───────────────────────────────────────────────────┘
```

### Crate Overview

| Crate | Purpose |
|---|---|
| `torvyn-types` | Universal foundation — identifiers, error types, state machines, domain enums, traits |
| `torvyn-contracts` | WIT interface definitions (`torvyn:streaming@0.1.0`), contract validation, version compatibility |
| `torvyn-config` | Component manifests (`Torvyn.toml`), pipeline definitions, environment interpolation, config merging |
| `torvyn-resources` | Host-managed buffer pools (4-tier Treiber stacks), ownership tracking, copy accounting, per-component memory budgets |
| `torvyn-engine` | Wasm execution abstraction over Wasmtime — compilation, instantiation, fuel management, component caching |
| `torvyn-reactor` | Async stream scheduler — flow lifecycle, demand-driven scheduling, bounded queues, backpressure with hysteresis |
| `torvyn-security` | Capability-based isolation (deny-all default), typed permissions, operator grants, audit logging |
| `torvyn-observability` | Metrics (counters, histograms, gauges), distributed tracing, structured diagnostic events, benchmark reporting |
| `torvyn-linker` | Static pipeline linking — interface resolution, capability matching, multi-error reporting |
| `torvyn-pipeline` | Pipeline topology construction, validation, and instantiation |
| `torvyn-packaging` | OCI artifact assembly, signing (Sigstore-compatible), registry push/pull |
| `torvyn-host` | Runtime orchestration — ties all subsystems into a unified lifecycle |
| `torvyn-cli` | Developer CLI — `init`, `check`, `link`, `run`, `trace`, `bench`, `pack`, `publish`, `doctor`, `inspect` |

### WIT Contracts

Torvyn components interact through typed WIT interfaces in the `torvyn:streaming@0.1.0` package:

```wit
// Core processing contract
interface processor {
    use types.{stream-element, process-result, process-error};
    process: func(element: stream-element) -> result<process-result, process-error>;
}

// Source — produces elements with backpressure awareness
interface source {
    use types.{output-element, process-error, backpressure-signal};
    pull: func() -> result<option<output-element>, process-error>;
    notify-backpressure: func(signal: backpressure-signal);
}

// Sink — consumes elements and signals capacity
interface sink {
    use types.{stream-element, backpressure-signal, process-error};
    push: func(element: stream-element) -> result<backpressure-signal, process-error>;
    complete: func() -> result<_, process-error>;
}
```

Resources use explicit ownership semantics: `buffer` (host-managed, immutable, borrowed across boundaries) and `mutable-buffer` (component-owned, writable, frozen before transfer). Flow metadata — trace context, deadlines, flow identity — is part of the contract, not sideband.

Extension interfaces cover filtering, routing, aggregation, windowing, and capability declarations.

## Key Design Decisions

**Ownership model over zero-copy promises.** The WebAssembly Component Model imposes real memory boundaries. Rather than making unrealistic zero-copy claims, Torvyn makes ownership explicit, copies bounded, and every transfer measurable. The copy ledger tracks which components copied how many bytes and why.

**Deny-all security.** Components start with no permissions. Every capability — filesystem, network, clock, resource pool access — must be explicitly granted by the operator. Capability checks on hot paths are zero-overhead (pre-resolved at instantiation). All exercises and denials are audit-logged.

**Consumer-first scheduling.** The reactor uses demand-driven scheduling: downstream consumers pull from upstream producers via credit-based flow control. Bounded queues with high/low watermark hysteresis prevent both overflow and oscillation.

**Engine abstraction.** The runtime is not hard-coupled to Wasmtime. `WasmEngine` and `ComponentInvoker` traits abstract the execution layer, with Wasmtime as the default (feature-gated) implementation.

**Observability as a core primitive.** Metrics, tracing, and diagnostics are not optional sidecars — they are built into the runtime with configurable overhead levels. Production mode adds less than 500 nanoseconds per element.

## Project Status

Torvyn is in **active development**. The core runtime — from the type system foundation through the CLI entry point — is implemented across 13 crates. The project is pre-release and APIs are not yet stable.

**What works today:**
- Complete type system and error model
- WIT contract definitions, parsing, and validation
- Configuration system with manifest loading, environment interpolation, and merging
- Buffer pool management with 4-tier allocation and ownership tracking
- Wasm engine abstraction with Wasmtime integration
- Reactive scheduler with backpressure, demand propagation, and flow lifecycle
- Capability-based security model with audit logging
- Observability infrastructure (metrics, tracing, events)
- Component linking and pipeline assembly
- OCI packaging primitives
- CLI scaffolding

**What's ahead:**
- End-to-end integration tests with real Wasm components
- Benchmark suite with baseline comparisons
- Example pipelines and component templates
- Documentation site
- Registry infrastructure
- Distributed execution (Phase 4)

## Documentation

Detailed design documents are in [`docs/design/`](docs/design/):

- [Vision Document](docs/design/torvyn_vision.md) — canonical project vision, problem statement, and technical thesis
- High-Level Implementation (HLI) documents covering contracts, runtime architecture, resources, scheduling, observability, security, CLI, packaging, and integration
- Low-Level Implementation (LLI) blueprints for each crate

## Building from Source

```bash
# Full workspace build
cargo build --workspace

# Build a specific crate
cargo build -p torvyn-reactor

# Run all tests
cargo test --workspace

# Run tests for a specific crate
cargo test -p torvyn-types

# Check for lint violations
cargo clippy --workspace -- -D warnings

# Generate documentation
cargo doc --workspace --no-deps --open
```

### Feature Flags

| Feature | Crate | Description |
|---|---|---|
| `wasmtime` | `torvyn-engine` | Enable Wasmtime-based execution (default) |
| `mock` | `torvyn-engine` | Enable mock engine for testing |
| `wit-parser` | `torvyn-contracts` | Enable WIT file parsing backend |
| `serde` | `torvyn-types` | Enable serialization support |

## Contributing

Contributions are welcome. Please read the contributing guidelines before submitting pull requests.

### Code Standards

- `#![deny(missing_docs)]` in every crate
- Zero Clippy warnings (`clippy::all = "deny"`)
- Every public function has at least one test
- Every error path has a test that triggers it
- State machines are tested for all valid and invalid transitions
- Every `unsafe` block has a `// SAFETY:` comment proving correctness

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

---

<p align="center">
  <sub>Built for the next generation of composable, observable, and safe streaming systems.</sub>
</p>
