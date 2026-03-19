# Torvyn Roadmap

This document outlines Torvyn's development plan from the current Phase 0 through long-term Phase 4. Timelines are estimated ranges, not commitments. Priorities may shift based on community feedback, implementation discoveries, and ecosystem changes.

**Legend:** Done · In progress · Planned · Exploring

---

## Phase 0 — Foundation (Estimated: 10-14 weeks)

**Goal:** A working, benchmarked, two-component pipeline (Source to Sink) with basic backpressure and tracing. Rust-only.

| Deliverable | Status | Notes |
|-------------|--------|-------|
| `torvyn-types`: shared identity types, error enums | Done | Universal leaf crate, no dependencies |
| `torvyn-contracts`: WIT packages, `wit-parser` validation | Done | `torvyn:streaming@0.1.0` |
| `torvyn-engine`: Wasmtime integration, `ComponentInvoker` | Done | Includes resource type integration spike |
| `torvyn-resources`: buffer pool, ownership state machine, copy accounting | Done | Small + Medium tiers |
| `torvyn-reactor`: single-flow driver, FIFO scheduling, bounded queues | Done | High/low watermark backpressure |
| `torvyn-observability`: counters, histograms, basic OTLP export | Done | Production level only |
| `torvyn-linker` + `torvyn-pipeline`: two-component linking, config-driven topology | Done | Minimal viable linker |
| `torvyn-host`: runtime binary | Done | Thin orchestration shell |
| `torvyn-cli`: `init`, `check`, `run` | Done | Core developer workflow |
| Benchmark suite: Source-to-Sink latency/throughput vs. gRPC localhost | Done | First published numbers |
| CI pipeline: build, test, clippy, rustfmt, MSRV, benchmark regression | In progress | GitHub Actions |
| Documentation: getting started guide, first-ten-minutes tutorial | Planned | Written during Phase 0, not after |

---

## Phase 1 — Multi-Stage Pipelines (Estimated: 8-12 weeks after Phase 0)

**Goal:** Multi-stage pipelines with full scheduling, fairness, cancellation, capabilities, and a second language.

| Deliverable | Status | Notes |
|-------------|--------|-------|
| Multi-flow scheduling, weighted fair queuing | Planned | `torvyn-reactor` extension |
| Cancellation propagation, timeout enforcement | Planned | Tiered cancellation protocol |
| Leased buffer state, Large + Huge buffer tiers, memory budgets | Planned | `torvyn-resources` extension |
| Capability taxonomy, `CapabilityGuard`, audit logging | Planned | `torvyn-security` |
| Extended WIT: `torvyn:filtering@0.1.0`, `torvyn:aggregation@0.1.0` | Planned | Additional component roles |
| Full multi-component linking with capability resolution | Planned | `torvyn-linker` |
| Diagnostic-level observability, per-element spans | Planned | `torvyn-observability` |
| CLI: `torvyn link`, `torvyn bench`, `torvyn trace`, `torvyn inspect` | Planned | Expanded developer workflow |
| Full benchmark suite: 10-stage pipeline, fairness under contention | Planned | Published methodology |
| Second language support (Go or Python) | Planned | Proof of polyglot composition |

---

## Phase 2 — Packaging and Distribution (Estimated: 6-10 weeks after Phase 1)

**Goal:** OCI-compatible packaging, signing, distribution, and early multi-tenant support.

| Deliverable | Status | Notes |
|-------------|--------|-------|
| OCI artifact format, `torvyn pack` | Planned | `torvyn-packaging` |
| Sigstore signing, `torvyn publish` | Planned | Provenance and trust |
| `torvyn doctor`, artifact inspection | Planned | Operational diagnostics |
| WASI 0.3 experimental support | Exploring | Behind feature flag |
| Multi-tenant isolation (tenant-scoped resource partitioning) | Exploring | Design pending |

---

## Phase 3 — Ecosystem (Estimated: ongoing after Phase 2)

**Goal:** Component registry, reusable component library, and community ecosystem growth.

| Deliverable | Status | Notes |
|-------------|--------|-------|
| Component registry protocol | Exploring | Signed artifacts, contract metadata, trust signals |
| Standard component library (transforms, filters, adapters) | Exploring | Community contributions welcome |
| Additional language support (Zig, C, JavaScript) | Exploring | Based on community demand |
| IDE integration and language server | Exploring | WIT authoring support |
| Adaptive scheduling based on runtime flow patterns | Exploring | Tail latency optimization |

---

## Phase 4 — Platform (Long-term)

**Goal:** Broader platform capabilities for distributed and enterprise use cases.

| Deliverable | Status | Notes |
|-------------|--------|-------|
| Edge-to-cloud component migration | Exploring | Preserve contracts and traceability |
| Hardware-aware transport (GPU, SmartNIC) | Exploring | Research phase |
| AI-assisted pipeline composition | Exploring | Expose component metadata to AI tooling |
| Policy-driven runtime governance | Exploring | Declarative execution policies for enterprise |
| Formal verification of ownership and flow invariants | Exploring | Model checking integration |

---

## How to Influence the Roadmap

The roadmap is not fixed. Community input directly shapes priorities.

**Request a feature:** Open a [Feature Request](https://github.com/torvyn/torvyn/issues/new?template=feature_request.yml) issue. Describe the problem, your use case, and the impact. Well-articulated requests with real-world context carry the most weight.

**Reprioritize an item:** Start a [Discussion](https://github.com/torvyn/torvyn/discussions/categories/ideas) with your reasoning. If multiple community members share the need, it influences scheduling.

**Contribute directly:** Some roadmap items are marked as good candidates for community contribution. Check the [good first issue](https://github.com/torvyn/torvyn/labels/good%20first%20issue) and [help wanted](https://github.com/torvyn/torvyn/labels/help%20wanted) labels.

**What we prioritize:** Correctness over features. Developer experience over internal elegance. Measured performance over theoretical performance. Real use cases over speculative ones.
