# Hello World Pipeline

The simplest possible Torvyn pipeline: a source that produces "Hello, World!" messages and a sink that prints them.

## What It Demonstrates

- Defining WIT contracts for source and sink components
- Implementing the `torvyn:streaming/source` interface
- Implementing the `torvyn:streaming/sink` interface
- Implementing the `torvyn:streaming/lifecycle` interface for initialization
- Configuring a two-node pipeline in `Torvyn.toml`
- Building and running with `torvyn run`

## Pipeline Topology

```
hello-source --> hello-sink
```

## Prerequisites

- Rust toolchain (1.75+)
- `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
- `cargo-component`: `cargo install cargo-component`
- Torvyn CLI: `cargo install torvyn-cli`

## Build

```bash
make build
```

Or manually:

```bash
cd components/hello-source && cargo component build --release
cd components/hello-sink && cargo component build --release
```

## Run

```bash
make run
```

Or manually:

```bash
torvyn run flow.main
```

## Expected Output

```
[hello-sink] seq=0: Hello, World! (message 1)
[hello-sink] seq=1: Hello, World! (message 2)
[hello-sink] seq=2: Hello, World! (message 3)
[hello-sink] seq=3: Hello, World! (message 4)
[hello-sink] seq=4: Hello, World! (message 5)
[hello-sink] Stream complete.
[hello-sink] Received 5 messages total.
```

## Run with Tracing

To see flow lifecycle events (Created, Validated, Running, Draining, Completed):

```bash
make trace
```

## Key Concepts

1. **Contract-first:** WIT interfaces define the exact shape of source and sink interactions. The `source.pull()` function returns an `output-element` with an owned buffer. The `sink.push()` function receives a `stream-element` with a borrowed buffer.

2. **Host-managed buffers:** The source does not allocate memory directly. It requests a `mutable-buffer` from the host via `buffer-allocator.allocate()`, writes into it, then freezes it. The host controls the buffer pool, tracks ownership, and records every copy.

3. **Lifecycle management:** Both components implement the `lifecycle` interface. The runtime calls `init()` before stream processing and `teardown()` after the flow completes.

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| End-to-end latency per element | < 50 us |
| Host-side overhead per element | < 5 us |
| Copies per element | 1 (sink reads payload from host buffer) |
| Component instantiation | < 10 ms per component |

## Learn More

- [Architecture Guide](../../docs/architecture.md) -- Component model and resource management
- [CLI Reference](../../docs/cli.md) -- `torvyn run`, `torvyn trace`, `torvyn bench`
