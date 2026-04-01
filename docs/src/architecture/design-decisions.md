# Design Decisions

This document indexes the most significant architectural decisions in Torvyn, their rationale, alternatives that were considered, and the trade-offs accepted. Each decision is traceable to a specific section of the HLI design documents.

## ADR Index

| ID | Decision | HLI Source | Status |
|----|----------|------------|--------|
| ADR-001 | Buffer as WIT resource, not `list<u8>` | Doc 01 §3.2.1 | Accepted |
| ADR-002 | Split buffer and mutable-buffer types | Doc 01 §3.2.2 | Accepted |
| ADR-003 | Task-per-flow, not task-per-component | Doc 04 §1.3 | Accepted |
| ADR-004 | Layer reactor on Tokio, not custom executor | Doc 04 §1.2 | Accepted |
| ADR-005 | Deny-all-by-default capability model | Doc 06 §1.5 | Accepted |
| ADR-006 | Capabilities in manifest, not in WIT | Doc 01 §5.1 | Accepted |
| ADR-007 | Pre-allocated metrics, not dynamic registry | Doc 05 §3.1 | Accepted |
| ADR-008 | Embedded observability, not sidecar | Doc 05 §1.5 | Accepted |
| ADR-009 | OCI-native artifact format | Doc 08 §1.2 | Accepted |
| ADR-010 | WASI 0.2 now, migration path to 0.3 | Doc 01 §4 | Accepted |
| ADR-011 | Credit-based demand model for backpressure | Doc 04 §4.1 | Accepted |
| ADR-012 | Global tiered buffer pools, not per-flow | Doc 03 §5.1 | Accepted |
| ADR-013 | Monolithic CLI binary, not plugin-based | Doc 07 §1.2 | Accepted |
| ADR-014 | TOML for configuration, not YAML | Doc 07 §4.1 | Accepted |
| ADR-015 | High/low watermark hysteresis for backpressure | Doc 04 §5.3 | Accepted |

## Key Decisions in Detail

### ADR-001: Buffer as WIT Resource

**Decision:** Buffers are WIT `resource` types with opaque handles, not `list<u8>` value types.

**Rationale:** If buffers were `list<u8>`, every cross-component transfer would require copying the full byte content into and out of component linear memory. With resource handles, the host can transfer ownership by moving a handle (an integer) while the payload bytes remain in host memory. This is the foundation of Torvyn's ownership-aware transfer model.

**Alternative rejected:** Embedding payload bytes as `list<u8>` in `stream-element`. Rejected because it would force a full copy at every component boundary.

**Alternative rejected:** Making buffer a `record` with inline bytes. Rejected because records are value types in WIT and are always copied in full across component boundaries.

### ADR-002: Split Buffer and Mutable-Buffer

**Decision:** Separate `buffer` (immutable, read-only) and `mutable-buffer` (writable, single-owner) resource types.

**Rationale:** The split enforces a clear write-then-freeze lifecycle. A component obtains a mutable buffer from the host, writes data, calls `freeze()` to convert it to immutable, and returns it. This avoids copy-on-write complexity and makes mutation boundaries explicit. A buffer is either being written (mutable, single owner) or being read (immutable, can be borrowed by multiple readers).

**Alternative rejected:** A single `buffer` with both read and write methods and runtime mutability tracking. Rejected because it complicates the resource manager's invariant checking.

### ADR-003: Task-Per-Flow

**Decision:** Each active flow gets one Tokio task (the flow driver). Stages within a flow execute sequentially within that task.

**Rationale:** Task-per-component would create excessive task overhead for large pipelines (20+ components × hundreds of flows = thousands of Tokio tasks). Task-per-flow gives the reactor control over intra-flow scheduling, reduces task switching overhead, and keeps related work cache-local. Tokio handles inter-flow scheduling; the reactor handles intra-flow scheduling.

**Trade-off accepted:** A single flow cannot utilize multiple OS threads simultaneously for different stages.

### ADR-004: Layer on Tokio

**Decision:** The reactor is a domain-specific scheduling layer on top of Tokio, not a replacement for it.

**Rationale:** Building a custom async runtime would be enormous engineering effort with no corresponding benefit. Tokio provides mature I/O polling, timers, work-stealing, and task infrastructure. What Tokio does not provide is stream-level scheduling policy, demand propagation, backpressure enforcement, or fairness across Wasm components. That is the reactor's domain.

### ADR-010: WASI 0.2 Now, Migration Path to 0.3

**Decision:** Torvyn's Phase 0 and Phase 1 target WASI 0.2. WASI 0.3 migration is planned for Phase 2 or later.

**Rationale:** WASI 0.3 introduces native `stream<T>`, `future<T>`, and `async func` — directly aligned with Torvyn's streaming model. However, as of early 2026, WASI 0.3 is in preview and not yet stable. The contract design is structured so that migration is straightforward: `source.pull()` maps to `async func`, `sink.push()` maps to `async func`, and the explicit backpressure signal enum may become unnecessary with native stream pressure. The semantic meaning of interfaces does not change during migration — only the mechanism changes.

## Proposing New Architectural Changes

Significant architectural changes to Torvyn follow an RFC (Request for Comments) process:

1. Open a GitHub issue tagged `rfc` describing the proposed change, motivation, and alternatives considered.
2. Write a design document in the `docs/rfcs/` directory following the ADR template: Decision, Context, Rationale, Alternatives Rejected, Trade-offs Accepted.
3. The RFC is discussed publicly for a minimum of 14 days.
4. Maintainers approve, request changes, or reject the RFC based on alignment with Torvyn's design principles and technical merit.
5. Approved RFCs are added to this index with a link to the implementation tracking issue.
