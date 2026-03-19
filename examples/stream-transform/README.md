# Stream Transform Pipeline

A three-stage pipeline: a source produces JSON events, a processor transforms them (adds a timestamp field, renames a field), and a sink writes the transformed output.

## What It Demonstrates

- Implementing the `torvyn:streaming/processor` interface
- Reading borrowed input, allocating new output
- Buffer ownership transfer (borrow input -> own output)
- JSON payload manipulation inside a Wasm component
- Three-node pipeline topology

## Pipeline Topology

```
json-source --> json-transform --> json-sink
```

The source emits JSON user events. The transform processor renames the `"user"` field to `"username"` and adds a `"processed_at"` timestamp. The sink prints the final output.

## Prerequisites

- Rust toolchain (1.75+)
- `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
- `cargo-component`: `cargo install cargo-component`
- Torvyn CLI: `cargo install torvyn-cli`

## Build

```bash
make build
```

## Run

```bash
make run
```

## Expected Output

```
[json-sink] seq=0: {"username":"alice","action":"login","processed_at":"2025-01-15T10:30:00Z"}
[json-sink] seq=1: {"username":"bob","action":"purchase","processed_at":"2025-01-15T10:30:00Z"}
[json-sink] seq=2: {"username":"carol","action":"logout","processed_at":"2025-01-15T10:30:00Z"}
[json-sink] Stream complete.
```

## Key Concepts

### Ownership Transfer

The processor receives a `stream-element` with a **borrowed** buffer handle. It can read from this buffer, but the handle is only valid for the duration of the `process()` call. To produce output, the processor:

1. Reads input via `element.payload.read_all()` (measured copy)
2. Transforms the data in component linear memory
3. Allocates a **new** mutable buffer via `buffer_allocator::allocate()`
4. Writes transformed data and freezes it
5. Returns it as an owned `output-element`

This two-copy pattern (read input, write output) is the normal case for processors that modify data.

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| End-to-end latency per element | < 80 us (two Wasm boundary crossings) |
| Copies per element | 2 (processor reads input, writes output) |
| Processor overhead | < 5 us host-side per invocation |

## Learn More

- [Architecture Guide: Ownership Model](../../docs/architecture.md#ownership-model)
- [Architecture Guide: Buffer Lifecycle](../../docs/architecture.md#buffer-lifecycle)
