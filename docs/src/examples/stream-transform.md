# Stream Transform

## What It Demonstrates

A three-stage pipeline: a source produces JSON events, a processor transforms them (adds a timestamp field, renames a field), and a sink writes the transformed output. This example teaches the processor interface, buffer allocation for output, and the ownership model across a transform boundary.

## Concepts Covered

- Implementing the `torvyn:streaming/processor` interface
- Reading borrowed input, allocating new output
- Buffer ownership transfer (borrow input → own output)
- JSON payload manipulation inside a Wasm component
- Three-node pipeline topology

## File Listing

```
examples/stream-transform/
├── Torvyn.toml
├── Makefile
├── README.md
├── wit/
│   └── torvyn-streaming/
│       └── ... (same canonical WIT files)
├── components/
│   ├── json-source/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── json-transform/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── json-sink/
│       ├── Cargo.toml
│       └── src/lib.rs
└── expected-output.txt
```

## Source Component: `json-source`

**`components/json-source/src/lib.rs`**

```rust
//! JSON event source.
//!
//! Produces a sequence of JSON objects representing user events.
//! Each event has a "user" field and an "action" field.

wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct JsonSource {
    events: Vec<String>,
    index: usize,
}

static mut STATE: Option<JsonSource> = None;
fn state() -> &'static mut JsonSource {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for JsonSource {
    fn init(_config: String) -> Result<(), ProcessError> {
        let events = vec![
            r#"{"user":"alice","action":"login"}"#.to_string(),
            r#"{"user":"bob","action":"purchase"}"#.to_string(),
            r#"{"user":"carol","action":"logout"}"#.to_string(),
        ];
        unsafe { STATE = Some(JsonSource { events, index: 0 }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl SourceGuest for JsonSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();
        if s.index >= s.events.len() {
            return Ok(None);
        }
        let payload = s.events[s.index].as_bytes();
        let buf = buffer_allocator::allocate(payload.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(payload)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("application/json");
        let frozen = buf.freeze();
        s.index += 1;
        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: (s.index - 1) as u64,
                timestamp_ns: 0,
                content_type: "application/json".to_string(),
            },
            payload: frozen,
        }))
    }
    fn notify_backpressure(_signal: BackpressureSignal) {}
}

export!(JsonSource);
```

## Processor Component: `json-transform`

**`components/json-transform/src/lib.rs`**

```rust
//! JSON transform processor.
//!
//! Reads each JSON event, adds a "processed_at" timestamp field,
//! and renames "user" to "username". Demonstrates the processor
//! interface's ownership model: input is borrowed, output is owned.

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::processor::Guest as ProcessorGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct JsonTransform;

static mut INITIALIZED: bool = false;

impl LifecycleGuest for JsonTransform {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() {
        unsafe { INITIALIZED = false; }
    }
}

impl ProcessorGuest for JsonTransform {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        // Step 1: Read the input payload (borrowed buffer → copy into
        // component linear memory). This is a measured copy.
        let input_bytes = input.payload.read_all();

        // Step 2: Parse and transform the JSON.
        // Using simple string manipulation to avoid pulling in serde.
        // In production code, use serde_json.
        let input_str = String::from_utf8(input_bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("not UTF-8: {e}")))?;

        // Rename "user" → "username" and add "processed_at".
        let transformed = input_str
            .replace("\"user\":", "\"username\":")
            .trim_end_matches('}')
            .to_string()
            + ",\"processed_at\":\"2025-01-15T10:30:00Z\"}";

        let out_bytes = transformed.as_bytes();

        // Step 3: Allocate a new output buffer from the host pool.
        let out_buf = buffer_allocator::allocate(out_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        out_buf.append(out_bytes)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        out_buf.set_content_type("application/json");

        // Step 4: Freeze and return. Ownership of the output buffer
        // transfers to the runtime via the OutputElement.
        let frozen = out_buf.freeze();

        Ok(ProcessResult::Emit(OutputElement {
            meta: ElementMeta {
                sequence: input.meta.sequence,
                timestamp_ns: 0,
                content_type: "application/json".to_string(),
            },
            payload: frozen,
        }))
    }
}

export!(JsonTransform);
```

## Sink Component: `json-sink`

**`components/json-sink/src/lib.rs`**

```rust
//! JSON sink — prints each transformed JSON event.

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct JsonSink;
static mut INITIALIZED: bool = false;

impl LifecycleGuest for JsonSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() { unsafe { INITIALIZED = false; } }
}

impl SinkGuest for JsonSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let bytes = element.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;
        println!("[json-sink] seq={}: {}", element.meta.sequence, text);
        Ok(BackpressureSignal::Ready)
    }
    fn complete() -> Result<(), ProcessError> {
        println!("[json-sink] Stream complete.");
        Ok(())
    }
}

export!(JsonSink);
```

## Pipeline Configuration

**`Torvyn.toml`**

```toml
[torvyn]
name = "stream-transform"
version = "0.1.0"
contract_version = "0.1.0"
description = "JSON event transformation pipeline"

[[component]]
name = "json-source"
path = "components/json-source"

[[component]]
name = "json-transform"
path = "components/json-transform"

[[component]]
name = "json-sink"
path = "components/json-sink"

[flow.main]
description = "Source → Transform → Sink"

[flow.main.nodes.source]
component = "json-source"
interface = "torvyn:streaming/source"

[flow.main.nodes.transform]
component = "json-transform"
interface = "torvyn:streaming/processor"

[flow.main.nodes.sink]
component = "json-sink"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "transform", port = "input" }

[[flow.main.edges]]
from = { node = "transform", port = "output" }
to = { node = "sink", port = "input" }
```

## Expected Output

```
$ torvyn run flow.main
[torvyn] Loading flow 'main' from Torvyn.toml
[torvyn] Validating contracts...  ok
[torvyn] Linking components...    ok (3 components, 2 edges)
[torvyn] Instantiating...         ok
[torvyn] Running flow 'main'

[json-sink] seq=0: {"username":"alice","action":"login","processed_at":"2025-01-15T10:30:00Z"}
[json-sink] seq=1: {"username":"bob","action":"purchase","processed_at":"2025-01-15T10:30:00Z"}
[json-sink] seq=2: {"username":"carol","action":"logout","processed_at":"2025-01-15T10:30:00Z"}
[json-sink] Stream complete.

[torvyn] Flow 'main' completed. 3 elements processed.
[torvyn] Duration: 8ms | Copies: 6 (3 reads + 3 writes) | Peak queue depth: 1
```

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| End-to-end latency per element | < 80 us (two Wasm boundary crossings) |
| Copies per element | 2 (processor reads input, writes output) |
| Processor overhead | < 5 us host-side per invocation |

## Commentary

The key teaching point is ownership transfer. The processor receives a `stream-element` with a **borrowed** buffer handle. It can read from this buffer, but the handle is only valid for the duration of the `process()` call. To produce output, the processor allocates a **new** mutable buffer, writes transformed data, freezes it, and returns it as an owned `output-element`. The runtime then takes ownership of the output buffer and delivers it downstream.

This two-copy pattern (read input, write output) is the normal case for processors that modify data. For pass-through processors that do not modify the payload, Torvyn's planned Phase 2 buffer view mechanism will allow forwarding without copying.

## Learn More

- [Architecture Guide: Ownership Model](docs/architecture.md#ownership-model) — Borrow vs. own semantics
- [Architecture Guide: Buffer Lifecycle](docs/architecture.md#buffer-lifecycle) — Allocate -> write -> freeze -> transfer
