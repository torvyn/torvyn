# Hello World

## What It Demonstrates

The simplest possible Torvyn pipeline: a source that produces "Hello, World!" messages and a sink that prints them. This example teaches the foundational concepts — WIT contracts, component implementation, pipeline configuration, and the `torvyn run` command.

## Concepts Covered

- Defining WIT contracts for source and sink components
- Implementing the `torvyn:streaming/source` interface
- Implementing the `torvyn:streaming/sink` interface
- Implementing the `torvyn:streaming/lifecycle` interface for initialization
- Configuring a two-node pipeline in `Torvyn.toml`
- Building and running with `torvyn run`

## File Listing

```
examples/hello-world/
├── Torvyn.toml
├── Makefile
├── README.md
├── wit/
│   └── torvyn-streaming/
│       ├── types.wit
│       ├── source.wit
│       ├── sink.wit
│       ├── lifecycle.wit
│       ├── buffer-allocator.wit
│       └── world.wit
├── components/
│   ├── hello-source/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs
│   └── hello-sink/
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs
└── expected-output.txt
```

## WIT Contracts

The example uses the canonical `torvyn:streaming@0.1.0` contracts. The relevant interfaces are included in the `wit/` directory so the example is self-contained.

**`wit/torvyn-streaming/types.wit`**

```wit
/// Core types for Torvyn streaming — subset for this example.
///
/// See the full types.wit in the Torvyn repository for complete documentation.

package torvyn:streaming@0.1.0;

interface types {
    resource buffer {
        size: func() -> u64;
        content-type: func() -> string;
        read: func(offset: u64, len: u64) -> list<u8>;
        read-all: func() -> list<u8>;
    }

    resource mutable-buffer {
        write: func(offset: u64, bytes: list<u8>) -> result<_, buffer-error>;
        append: func(bytes: list<u8>) -> result<_, buffer-error>;
        size: func() -> u64;
        capacity: func() -> u64;
        set-content-type: func(content-type: string);
        freeze: func() -> buffer;
    }

    variant buffer-error {
        capacity-exceeded,
        out-of-bounds,
        allocation-failed(string),
    }

    resource flow-context {
        trace-id: func() -> string;
        span-id: func() -> string;
        deadline-ns: func() -> u64;
        flow-id: func() -> string;
    }

    record element-meta {
        sequence: u64,
        timestamp-ns: u64,
        content-type: string,
    }

    record stream-element {
        meta: element-meta,
        payload: borrow<buffer>,
        context: borrow<flow-context>,
    }

    record output-element {
        meta: element-meta,
        payload: buffer,
    }

    variant process-error {
        invalid-input(string),
        unavailable(string),
        internal(string),
        deadline-exceeded,
        fatal(string),
    }

    enum backpressure-signal {
        ready,
        pause,
    }
}
```

**`wit/torvyn-streaming/source.wit`**

```wit
package torvyn:streaming@0.1.0;

interface source {
    use types.{output-element, process-error, backpressure-signal};

    pull: func() -> result<option<output-element>, process-error>;
    notify-backpressure: func(signal: backpressure-signal);
}
```

**`wit/torvyn-streaming/sink.wit`**

```wit
package torvyn:streaming@0.1.0;

interface sink {
    use types.{stream-element, process-error, backpressure-signal};

    push: func(element: stream-element) -> result<backpressure-signal, process-error>;
    complete: func() -> result<_, process-error>;
}
```

**`wit/torvyn-streaming/lifecycle.wit`**

```wit
package torvyn:streaming@0.1.0;

interface lifecycle {
    use types.{process-error};

    init: func(config: string) -> result<_, process-error>;
    teardown: func();
}
```

**`wit/torvyn-streaming/buffer-allocator.wit`**

```wit
package torvyn:streaming@0.1.0;

interface buffer-allocator {
    use types.{mutable-buffer, buffer-error, buffer};

    allocate: func(capacity-hint: u64) -> result<mutable-buffer, buffer-error>;
    clone-into-mutable: func(source: borrow<buffer>) -> result<mutable-buffer, buffer-error>;
}
```

**`wit/torvyn-streaming/world.wit`**

```wit
package torvyn:streaming@0.1.0;

world data-source {
    import types;
    import buffer-allocator;

    export source;
    export lifecycle;
}

world data-sink {
    import types;

    export sink;
    export lifecycle;
}
```

## Source Component: `hello-source`

**`components/hello-source/Cargo.toml`**

```toml
[package]
name = "hello-source"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "torvyn:streaming"
world = "data-source"
```

**`components/hello-source/src/lib.rs`**

```rust
//! Hello World source component.
//!
//! Produces a configurable number of "Hello, World!" messages,
//! then signals stream exhaustion.

// Generate bindings from the WIT contract.
// This creates Rust types and traits matching the WIT interfaces.
wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::{BackpressureSignal, OutputElement, ElementMeta, ProcessError};

/// Component state. Holds configuration and tracks how many
/// messages have been produced.
struct HelloSource {
    /// Total messages to produce before signaling exhaustion.
    total_messages: u64,
    /// Messages produced so far.
    produced: u64,
}

// Global mutable state. In a Wasm component, there is exactly one
// instance of this state per component instantiation. The host
// guarantees no concurrent access (no reentrancy).
static mut STATE: Option<HelloSource> = None;

fn state() -> &'static mut HelloSource {
    unsafe { STATE.as_mut().expect("component not initialized") }
}

impl LifecycleGuest for HelloSource {
    /// Initialize the source. Accepts an optional JSON config string
    /// specifying `{"count": N}`. Defaults to 5 messages.
    fn init(config: String) -> Result<(), ProcessError> {
        let total = if config.is_empty() {
            5
        } else {
            // Simple manual parsing to avoid pulling in serde for a demo.
            // In production, use serde_json.
            config
                .trim()
                .strip_prefix("{\"count\":")
                .and_then(|s| s.strip_suffix('}'))
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(5)
        };

        unsafe {
            STATE = Some(HelloSource {
                total_messages: total,
                produced: 0,
            });
        }
        Ok(())
    }

    fn teardown() {
        unsafe { STATE = None; }
    }
}

impl SourceGuest for HelloSource {
    /// Pull the next element. Returns None when all messages are produced.
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();

        if s.produced >= s.total_messages {
            // Stream exhausted. The runtime will call complete() on
            // downstream sinks and transition the flow to Draining.
            return Ok(None);
        }

        // Format the message payload.
        let message = format!("Hello, World! (message {})", s.produced + 1);
        let payload_bytes = message.as_bytes();

        // Allocate a buffer from the host's buffer pool.
        // The host manages the memory — the component never directly
        // allocates host-side buffers.
        let mut_buf = buffer_allocator::allocate(payload_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("buffer allocation failed: {e:?}")))?;

        // Write the payload into the mutable buffer.
        mut_buf.append(payload_bytes)
            .map_err(|e| ProcessError::Internal(format!("buffer write failed: {e:?}")))?;

        // Set content type for downstream consumers.
        mut_buf.set_content_type("text/plain");

        // Freeze the mutable buffer into an immutable buffer.
        // Ownership of the buffer transfers to the runtime when
        // we return it inside the OutputElement.
        let frozen = mut_buf.freeze();

        s.produced += 1;

        Ok(Some(OutputElement {
            meta: ElementMeta {
                // The runtime overwrites sequence and timestamp-ns (per C01-4).
                // These values are advisory.
                sequence: s.produced - 1,
                timestamp_ns: 0,
                content_type: "text/plain".to_string(),
            },
            payload: frozen,
        }))
    }

    fn notify_backpressure(_signal: BackpressureSignal) {
        // This simple source ignores backpressure signals.
        // A production source would pause or slow its data generation.
    }
}

// Register the component with the Wasm component model.
export!(HelloSource);
```

## Sink Component: `hello-sink`

**`components/hello-sink/Cargo.toml`**

```toml
[package]
name = "hello-sink"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "torvyn:streaming"
world = "data-sink"
```

**`components/hello-sink/src/lib.rs`**

```rust
//! Hello World sink component.
//!
//! Receives stream elements and prints their payload contents
//! to the component's standard output (captured by the host).

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::{BackpressureSignal, ProcessError, StreamElement};

struct HelloSink {
    received: u64,
}

static mut STATE: Option<HelloSink> = None;

fn state() -> &'static mut HelloSink {
    unsafe { STATE.as_mut().expect("component not initialized") }
}

impl LifecycleGuest for HelloSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe {
            STATE = Some(HelloSink { received: 0 });
        }
        Ok(())
    }

    fn teardown() {
        let s = state();
        // Print summary on teardown.
        println!("[hello-sink] Received {} messages total.", s.received);
        unsafe { STATE = None; }
    }
}

impl SinkGuest for HelloSink {
    /// Receive a stream element and print its payload.
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let s = state();

        // Read the payload bytes from the borrowed buffer handle.
        // This copies data from host memory into component linear memory.
        // The resource manager records this copy for observability.
        let payload_bytes = element.payload.read_all();

        // Convert bytes to a UTF-8 string.
        let text = String::from_utf8(payload_bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("payload is not UTF-8: {e}")))?;

        println!("[hello-sink] seq={}: {}", element.meta.sequence, text);

        s.received += 1;

        // Signal that we are ready for the next element.
        // Returning BackpressureSignal::Pause would tell the runtime
        // to stop delivering elements until we are ready.
        Ok(BackpressureSignal::Ready)
    }

    /// Called when the upstream source is exhausted.
    fn complete() -> Result<(), ProcessError> {
        println!("[hello-sink] Stream complete.");
        Ok(())
    }
}

export!(HelloSink);
```

## Pipeline Configuration

**`Torvyn.toml`**

```toml
[torvyn]
name = "hello-world"
version = "0.1.0"
contract_version = "0.1.0"
description = "The simplest possible Torvyn pipeline"

# Declare the two components in this workspace.
[[component]]
name = "hello-source"
path = "components/hello-source"

[[component]]
name = "hello-sink"
path = "components/hello-sink"

# Define the pipeline flow.
[flow.main]
description = "Source produces greetings, sink prints them"

[flow.main.nodes.source]
component = "hello-source"
interface = "torvyn:streaming/source"
config = '{"count": 5}'

[flow.main.nodes.sink]
component = "hello-sink"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }
```

## Makefile

```makefile
.PHONY: build run clean check

# Build both components to WebAssembly.
build:
	torvyn check
	cd components/hello-source && cargo component build --release
	cd components/hello-sink && cargo component build --release

# Run the pipeline.
run: build
	torvyn run flow.main

# Run with tracing enabled to see flow lifecycle events.
trace: build
	torvyn trace flow.main

# Validate contracts and topology without running.
check:
	torvyn check
	torvyn link flow.main

# Benchmark this pipeline (latency, throughput, copies).
bench: build
	torvyn bench flow.main --iterations 1000

clean:
	cd components/hello-source && cargo clean
	cd components/hello-sink && cargo clean
```

## Expected Output

**`expected-output.txt`**

```
$ torvyn run flow.main
[torvyn] Loading flow 'main' from Torvyn.toml
[torvyn] Validating contracts...  ok
[torvyn] Linking components...    ok (2 components, 1 edge)
[torvyn] Instantiating...         ok
[torvyn] Running flow 'main'

[hello-sink] seq=0: Hello, World! (message 1)
[hello-sink] seq=1: Hello, World! (message 2)
[hello-sink] seq=2: Hello, World! (message 3)
[hello-sink] seq=3: Hello, World! (message 4)
[hello-sink] seq=4: Hello, World! (message 5)
[hello-sink] Stream complete.
[hello-sink] Received 5 messages total.

[torvyn] Flow 'main' completed. 5 elements processed.
[torvyn] Duration: 12ms | Copies: 5 | Peak queue depth: 1
```

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| End-to-end latency per element | < 50 us (includes Wasm boundary crossing) |
| Host-side overhead per element | < 5 us (per `MAX_HOT_PATH_NS` constant) |
| Copies per element | 1 (sink reads payload from host buffer) |
| Component instantiation | < 10 ms per component |
| Memory per component | < 2 MB linear memory |

## Commentary

This example demonstrates the fundamental Torvyn data flow model:

1. **Contract-first:** Before writing any code, the WIT interfaces define the exact shape of source and sink interactions. The `source.pull()` function returns an `output-element` with an owned buffer. The `sink.push()` function receives a `stream-element` with a borrowed buffer. This ownership distinction is not incidental — it is central to Torvyn's design.

2. **Host-managed buffers:** The source does not allocate memory directly. It requests a `mutable-buffer` from the host via `buffer-allocator.allocate()`, writes into it, then freezes it. The host controls the buffer pool, tracks ownership, and records every copy.

3. **Lifecycle management:** Both components implement the `lifecycle` interface. The runtime calls `init()` before any stream processing and `teardown()` after the flow completes. Component state lives in the component's linear memory and is isolated from all other components.

4. **Flow state machine:** The flow transitions through `Created -> Validated -> Instantiated -> Running -> Draining -> Completed`. You can observe these transitions by running with `torvyn trace`.

## Learn More

- [Architecture Guide: Component Model](docs/architecture.md#component-model) — How WIT contracts map to runtime behavior
- [Architecture Guide: Resource Manager](docs/architecture.md#resource-manager) — How buffer ownership and pooling work
- [CLI Reference: `torvyn run`](docs/cli.md#torvyn-run) — All run options and flags
