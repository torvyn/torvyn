# WIT Interface Reference

This is the complete reference for all Torvyn WIT interfaces. All interfaces are defined in the `torvyn:streaming@0.1.0` package unless otherwise noted.

## Core Types (`torvyn:streaming/types`)

### `resource buffer`

A host-managed immutable byte buffer. Buffers exist in host memory, not in component linear memory. Components interact with buffers through opaque handles.

| Method | Signature | Ownership | Description |
|--------|-----------|-----------|-------------|
| `size` | `func() -> u64` | Read-only (borrow) | Returns the byte length of the buffer contents. |
| `content-type` | `func() -> string` | Read-only (borrow) | Returns a content-type hint (e.g., `"application/json"`). Empty string if unset. |
| `read` | `func(offset: u64, len: u64) -> list<u8>` | Read-only (borrow). **Triggers a measured copy** from host to component memory. | Read up to `len` bytes starting at `offset`. Returns fewer bytes if buffer is shorter than `offset+len`. |
| `read-all` | `func() -> list<u8>` | Read-only (borrow). **Triggers a measured copy.** | Read the entire buffer contents. Equivalent to `read(0, self.size())`. |

**Performance note:** `read` and `read-all` copy data from host memory into component linear memory. The resource manager records this as a `PayloadRead` copy event. Components that only need metadata should use `size()` and `content-type()` instead.

### `resource mutable-buffer`

A writable buffer obtained from the host. Single-owner. Must be frozen into an immutable `buffer` before returning to the host.

| Method | Signature | Ownership | Description |
|--------|-----------|-----------|-------------|
| `write` | `func(offset: u64, bytes: list<u8>) -> result<_, buffer-error>` | Write (own). **Triggers a measured copy** from component to host memory. | Write bytes at offset. Extends buffer if necessary up to capacity. |
| `append` | `func(bytes: list<u8>) -> result<_, buffer-error>` | Write (own). **Triggers a measured copy.** | Append bytes to the end of current content. |
| `size` | `func() -> u64` | Read-only (own) | Current byte length of written content. |
| `capacity` | `func() -> u64` | Read-only (own) | Maximum capacity of this buffer. |
| `set-content-type` | `func(content-type: string)` | Write (own) | Set the content-type hint. |
| `freeze` | `func() -> buffer` | **Consumes** the mutable-buffer handle. Returns an owned immutable `buffer`. | Finalize into an immutable buffer. After this call, the mutable-buffer handle is invalid. |

### `resource flow-context`

Carries trace correlation, deadline, and pipeline-scoped metadata. Created by the runtime, passed to components with each stream element.

| Method | Signature | Description |
|--------|-----------|-------------|
| `trace-id` | `func() -> string` | W3C Trace ID (hex-encoded, 32 chars). Empty if tracing disabled. |
| `span-id` | `func() -> string` | Current span ID (hex-encoded, 16 chars). Empty if tracing disabled. |
| `deadline-ns` | `func() -> u64` | Remaining deadline in nanoseconds. 0 means no deadline set. |
| `flow-id` | `func() -> string` | Unique flow identifier (opaque string). |

### `record element-meta`

| Field | Type | Description |
|-------|------|-------------|
| `sequence` | `u64` | Monotonic sequence number within the flow. Assigned by the runtime. |
| `timestamp-ns` | `u64` | Wall-clock timestamp (ns since Unix epoch). Assigned by the runtime. |
| `content-type` | `string` | Content type of the payload. |

### `record stream-element`

The fundamental unit of data flow. Passed to `processor.process()` and `sink.push()`.

| Field | Type | Ownership | Description |
|-------|------|-----------|-------------|
| `meta` | `element-meta` | Copied (small record) | Element metadata. |
| `payload` | `borrow<buffer>` | Borrowed. Must not be stored beyond the function call. | Reference to the payload buffer. |
| `context` | `borrow<flow-context>` | Borrowed. | Reference to the flow context. |

### `record output-element`

Produced by components that create new data. Returned from `processor.process()` and `source.pull()`.

| Field | Type | Ownership | Description |
|-------|------|-----------|-------------|
| `meta` | `element-meta` | Copied. `sequence` and `timestamp-ns` are advisory — the runtime may overwrite them. | Output metadata. |
| `payload` | `buffer` | **Owned.** Ownership transfers from the component to the runtime. | Output payload buffer. |

### `variant process-result`

| Case | Payload | Description |
|------|---------|-------------|
| `emit` | `output-element` | The component produced output. The buffer in `output-element` is owned by the runtime after the call returns. |
| `drop` | (none) | The component consumed the input but produced no output. Not an error — used for filtering, deduplication, aggregation. |

### `variant process-error`

| Case | Payload | Runtime Behavior |
|------|---------|------------------|
| `invalid-input` | `string` | The input element was malformed. Error policy applies (skip, retry, terminate). |
| `unavailable` | `string` | A required resource or service was unavailable. May trigger circuit-breaker logic. |
| `internal` | `string` | Unexpected internal error. Use sparingly. |
| `deadline-exceeded` | (none) | The processing deadline has passed. Feeds into timeout accounting. |
| `fatal` | `string` | The component is permanently unable to process further elements. Triggers teardown. |

### `variant buffer-error`

| Case | Description |
|------|-------------|
| `capacity-exceeded` | Write would exceed the buffer's capacity limit. |
| `out-of-bounds` | Offset is beyond current bounds. |
| `allocation-failed` | `string` — Host-side allocation failure. |

### `enum backpressure-signal`

| Case | Description |
|------|-------------|
| `ready` | Consumer is ready to accept more data. |
| `pause` | Consumer requests the producer to pause. |

## Core Interfaces

### `interface buffer-allocator`

Imported by components that produce output (processors, sources).

| Function | Signature | Description |
|----------|-----------|-------------|
| `allocate` | `func(capacity-hint: u64) -> result<mutable-buffer, buffer-error>` | Request a new mutable buffer. The host may allocate larger than requested but never smaller. Returns error if memory budget is exceeded. |
| `clone-into-mutable` | `func(source: borrow<buffer>) -> result<mutable-buffer, buffer-error>` | Request a mutable buffer initialized with a copy of an existing buffer's contents. The source buffer is borrowed, not consumed. |

### `interface processor`

Exported by transform components.

| Function | Signature | Description |
|----------|-----------|-------------|
| `process` | `func(input: stream-element) -> result<process-result, process-error>` | Process a single stream element. Input is borrowed; output (if `emit`) is owned by the runtime. Called once per element, not concurrently. |

### `interface source`

Exported by data-producing components.

| Function | Signature | Description |
|----------|-----------|-------------|
| `pull` | `func() -> result<option<output-element>, process-error>` | Pull the next element. `ok(some(element))`: data available. `ok(none)`: source exhausted. `err(error)`: production error. Output buffer is owned by the runtime. |
| `notify-backpressure` | `func(signal: backpressure-signal)` | Receive a backpressure signal from the downstream pipeline. Called by the runtime between `pull()` invocations. |

### `interface sink`

Exported by data-consuming components.

| Function | Signature | Description |
|----------|-----------|-------------|
| `push` | `func(element: stream-element) -> result<backpressure-signal, process-error>` | Push an element into the sink. Returns `ready` (accept more) or `pause` (slow down). Input is borrowed — sink must copy payload bytes during this call if it needs to buffer them. |
| `complete` | `func() -> result<_, process-error>` | Signal that no more elements will arrive. Sink should flush any buffered data. |

### `interface lifecycle`

Optional. Exported by components that need initialization or cleanup.

| Function | Signature | Description |
|----------|-----------|-------------|
| `init` | `func(config: string) -> result<_, process-error>` | Called once after instantiation, before stream processing. Configuration string is component-specific (JSON recommended). Error prevents pipeline startup. |
| `teardown` | `func()` | Called once during shutdown. Best-effort — the runtime may skip this on forced termination. |

## Extension Interfaces

### `interface filter` (`torvyn:filtering@0.1.0`)

| Function | Signature | Description |
|----------|-----------|-------------|
| `evaluate` | `func(element: stream-element) -> result<bool, process-error>` | Accept (`true`) or reject (`false`) an element. Input is borrowed. No output buffer allocation — filters are extremely cheap. |

### `interface router` (`torvyn:filtering@0.1.0`)

| Function | Signature | Description |
|----------|-----------|-------------|
| `route` | `func(element: stream-element) -> result<list<string>, process-error>` | Return output port names for this element. Empty list = drop. Multiple names = fan-out. Port names must match topology configuration. |

### `interface aggregator` (`torvyn:aggregation@0.1.0`)

| Function | Signature | Description |
|----------|-----------|-------------|
| `ingest` | `func(element: stream-element) -> result<option<output-element>, process-error>` | Absorb an element into internal state. Optionally emit an aggregated result. |
| `flush` | `func() -> result<list<output-element>, process-error>` | Emit any remaining buffered results. Called when the upstream flow completes. |

## Standard Worlds

| World | Imports | Exports | Use Case |
|-------|---------|---------|----------|
| `transform` | `types`, `buffer-allocator` | `processor` | Stateless stream processor |
| `managed-transform` | `types`, `buffer-allocator` | `processor`, `lifecycle` | Processor with init/teardown |
| `data-source` | `types`, `buffer-allocator` | `source` | Data producer |
| `managed-source` | `types`, `buffer-allocator` | `source`, `lifecycle` | Source with init/teardown |
| `data-sink` | `types` | `sink` | Data consumer |
| `managed-sink` | `types` | `sink`, `lifecycle` | Sink with init/teardown |
| `content-filter` | `types` | `filter` | Accept/reject filter |
| `content-router` | `types` | `router` | Multi-port router |
| `stream-aggregator` | `types`, `buffer-allocator` | `aggregator`, `lifecycle` | Stateful aggregator |
