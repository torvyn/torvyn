# Introducing Torvyn

*An ownership-aware reactive streaming runtime for safe, low-latency pipelines.*

---

We are releasing the first public version of Torvyn — a reactive streaming runtime built in Rust on the WebAssembly Component Model.

Torvyn is not a general-purpose framework. It is a focused runtime for a specific and painful category of problems: building low-latency streaming pipelines on the same machine or edge node, where teams need isolation without the overhead of microservice boundaries, and composability without the risks of in-process plugins.

## Why We Built This

The modern infrastructure landscape has a gap. Teams building streaming systems — AI inference pipelines, event processors, edge analytics, plugin platforms — are forced to choose between latency and safety, between composability and isolation, between polyglot flexibility and operational simplicity.

Traditional microservices solve the isolation problem but impose serialization, network stack traversal, and operational complexity at every boundary. In-process plugins solve the latency problem but sacrifice memory safety, language neutrality, and governance. Containers solve the packaging problem but add too much overhead for fine-grained, high-frequency composition.

These trade-offs are well-known. Most teams work around them with custom glue code, ad hoc buffer management, and application-specific back-pressure logic. Every team reinvents this infrastructure slightly differently.

Torvyn is our attempt at a common solution: a runtime that provides low-latency component composition with real isolation, explicit contracts, tracked resource ownership, built-in back-pressure, and production-grade observability.

## How It Works

A Torvyn pipeline is a directed graph of components connected by typed streams.

**Contracts come first.** Every component interaction is defined by a WIT interface. The contract specifies what data a component accepts, what it produces, what resources it uses, and what capabilities it requires. Contracts are versioned and machine-checkable.

**Components are sandboxed.** Each component is compiled to WebAssembly and runs in its own isolated environment. Components can be written in any language that targets the WebAssembly Component Model — Rust has first-class support today. The runtime manages component lifecycle, from instantiation to teardown.

**The host manages resources.** Data buffers are host-managed resources with explicit ownership. The resource manager tracks every allocation, borrow, transfer, and copy. Where components pass data without reading the payload, the transfer is zero-copy in the payload path. Where copies are necessary, they are recorded and reported.

**The reactor schedules reactively.** The reactor is a flow-aware scheduler that runs on top of Tokio. It uses a task-per-flow architecture with consumer-first, demand-driven scheduling. Back-pressure is built into the stream model. Each stream has a bounded queue with configurable overflow policies and hysteresis-based watermarks that prevent oscillation.

**Everything is observable.** Flow lifecycle transitions, back-pressure events, resource transfers, scheduling decisions, and copy operations all emit structured events. OpenTelemetry support is native.

## What Is Available Now

This initial release (Phase 0) includes:

- The core contract layer with WIT package definitions for streaming types, processor, source, and sink interfaces.
- The host runtime with component loading, linking, and lifecycle management via Wasmtime.
- The reactor with task-per-flow scheduling, demand-driven back-pressure, cooperative yielding, and fairness enforcement.
- The resource manager with buffer pooling, ownership tracking, and copy accounting.
- The observability layer with structured event emission and OpenTelemetry integration.
- The CLI with `torvyn init`, `torvyn check`, `torvyn link`, `torvyn run`, `torvyn trace`, and `torvyn bench` commands.
- A benchmark suite measuring passthrough latency, throughput saturation, back-pressure response time, and multi-flow fairness.

Rust is the supported component language in Phase 0. Additional language support is planned.

## What Is Coming Next

Phase 1 will expand the contract surface (filter, router, and aggregator interfaces), introduce supply chain security foundations (artifact signing design hooks), improve the packaging workflow, and add multi-flow orchestration features.

The roadmap is published and updated in the project repository.

## What This Is Not

Torvyn is not a distributed orchestrator, not a Kubernetes replacement, not a general-purpose service mesh, and not a serverless platform. It does not claim to make all data transfers zero-copy across all boundaries. It does not claim to replace all microservice architectures.

Torvyn is a focused runtime for safe, observable, low-latency streaming composition on the same node and at the edge. That is a specific and valuable space. We are building for that space deliberately.

## Current Status

Torvyn is in early development. The architecture is well-defined, the core subsystems are implemented, and the benchmark suite is running. It is not yet production-hardened. We are publishing it now because we believe the design is sound, the use cases are real, and the project benefits from community review and participation from this point forward.

We welcome feedback on the design, contributions to the codebase, and conversations with teams whose workloads might be a good fit.

## Get Involved

- **Read the docs:** [torvyn.dev/docs](/docs)
- **Browse the source:** [github.com/torvyn/torvyn](https://github.com/torvyn/torvyn)
- **Join the conversation:** [GitHub Discussions](https://github.com/torvyn/torvyn/discussions)
- **Contribute:** [CONTRIBUTING.md](https://github.com/torvyn/torvyn/blob/main/CONTRIBUTING.md)
