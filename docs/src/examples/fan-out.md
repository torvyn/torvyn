# Fan-Out

## What It Demonstrates

A pipeline with fan-out topology: a single source produces events, a router component inspects each event and directs it to one of two branches based on a routing criterion, and each branch has its own processor and sink.

## Concepts Covered

- The `torvyn:filtering/router` interface for multi-port dispatch
- Named output ports in pipeline configuration
- Fan-out topology (one-to-many routing)
- Branch-specific processing

## Router Component

**`components/event-router/src/lib.rs`**

```rust
//! Event router.
//!
//! Routes events to named output ports based on the "type" field.
//! Events with type "metric" go to port "metrics".
//! Events with type "log" go to port "logs".
//! Events with any other type are broadcast to both ports.
//!
//! The router interface returns a list of port name strings.
//! The runtime uses this list to forward the element's borrowed
//! buffer handle to each named downstream edge. No additional
//! buffer allocation occurs for fan-out — the same host buffer
//! is borrowed by each receiving component in sequence.

wit_bindgen::generate!({
    world: "content-router",
    path: "../../wit",
});

use exports::torvyn::filtering::router::Guest as RouterGuest;
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::types::*;

struct EventRouter;
static mut INITIALIZED: bool = false;

impl LifecycleGuest for EventRouter {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() { unsafe { INITIALIZED = false; } }
}

impl RouterGuest for EventRouter {
    fn route(element: StreamElement) -> Result<Vec<String>, ProcessError> {
        let bytes = element.payload.read_all();
        let text = String::from_utf8_lossy(&bytes);

        if text.contains("\"type\":\"metric\"") {
            Ok(vec!["metrics".to_string()])
        } else if text.contains("\"type\":\"log\"") {
            Ok(vec!["logs".to_string()])
        } else {
            // Unknown type: broadcast to both branches.
            Ok(vec!["metrics".to_string(), "logs".to_string()])
        }
    }
}

export!(EventRouter);
```

## Source Component: `event-source`

**`components/event-source/src/lib.rs`**

```rust
//! Event source for the fan-out example.
//! Produces a mix of metric and log events.

wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

const EVENTS: &[&str] = &[
    r#"{"type":"metric","name":"cpu_usage","value":72.5}"#,
    r#"{"type":"log","level":"info","message":"user login successful"}"#,
    r#"{"type":"metric","name":"mem_used","value":4096}"#,
    r#"{"type":"log","level":"warn","message":"disk usage above 80%"}"#,
];

struct EventSource { index: usize }
static mut STATE: Option<EventSource> = None;
fn state() -> &'static mut EventSource {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for EventSource {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { STATE = Some(EventSource { index: 0 }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl SourceGuest for EventSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();
        if s.index >= EVENTS.len() { return Ok(None); }
        let payload = EVENTS[s.index].as_bytes();
        let buf = buffer_allocator::allocate(payload.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(payload).map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
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

export!(EventSource);
```

## Branch Processors

**`components/metric-processor/src/lib.rs`**

```rust
//! Metric processor — adds a "processed_by":"metrics-pipeline" tag.

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::processor::Guest as ProcessorGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct MetricProcessor;
static mut INITIALIZED: bool = false;

impl LifecycleGuest for MetricProcessor {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() { unsafe { INITIALIZED = false; } }
}

impl ProcessorGuest for MetricProcessor {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        let bytes = input.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;

        let tagged = text.trim_end_matches('}').to_string()
            + ",\"processed_by\":\"metrics-pipeline\"}";

        let out_bytes = tagged.as_bytes();
        let buf = buffer_allocator::allocate(out_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(out_bytes).map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("application/json");
        let frozen = buf.freeze();

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

export!(MetricProcessor);
```

**`components/log-processor/src/lib.rs`**

```rust
//! Log processor — adds a "processed_by":"log-pipeline" tag.

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::processor::Guest as ProcessorGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct LogProcessor;
static mut INITIALIZED: bool = false;

impl LifecycleGuest for LogProcessor {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() { unsafe { INITIALIZED = false; } }
}

impl ProcessorGuest for LogProcessor {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        let bytes = input.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;

        let tagged = text.trim_end_matches('}').to_string()
            + ",\"processed_by\":\"log-pipeline\"}";

        let out_bytes = tagged.as_bytes();
        let buf = buffer_allocator::allocate(out_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(out_bytes).map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("application/json");
        let frozen = buf.freeze();

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

export!(LogProcessor);
```

## Branch Sinks

**`components/metric-sink/src/lib.rs`**

```rust
//! Metric sink — prints received metric events.

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct MetricSink;
static mut INITIALIZED: bool = false;

impl LifecycleGuest for MetricSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() { unsafe { INITIALIZED = false; } }
}

impl SinkGuest for MetricSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let bytes = element.payload.read_all();
        let text = String::from_utf8_lossy(&bytes);
        println!("[metric-sink] Received metric: {}", text);
        Ok(BackpressureSignal::Ready)
    }
    fn complete() -> Result<(), ProcessError> {
        println!("[metric-sink] Stream complete.");
        Ok(())
    }
}

export!(MetricSink);
```

**`components/log-sink/src/lib.rs`**

```rust
//! Log sink — prints received log events.

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct LogSink;
static mut INITIALIZED: bool = false;

impl LifecycleGuest for LogSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() { unsafe { INITIALIZED = false; } }
}

impl SinkGuest for LogSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let bytes = element.payload.read_all();
        let text = String::from_utf8_lossy(&bytes);
        println!("[log-sink] Received log: {}", text);
        Ok(BackpressureSignal::Ready)
    }
    fn complete() -> Result<(), ProcessError> {
        println!("[log-sink] Stream complete.");
        Ok(())
    }
}

export!(LogSink);
```

## Pipeline Configuration

**`Torvyn.toml`**

```toml
[torvyn]
name = "fan-out"
version = "0.1.0"
contract_version = "0.1.0"
description = "Fan-out pipeline with router"

[[component]]
name = "event-source"
path = "components/event-source"

[[component]]
name = "event-router"
path = "components/event-router"

[[component]]
name = "metric-processor"
path = "components/metric-processor"

[[component]]
name = "log-processor"
path = "components/log-processor"

[[component]]
name = "metric-sink"
path = "components/metric-sink"

[[component]]
name = "log-sink"
path = "components/log-sink"

[flow.main]
description = "Source → Router → (Metrics branch, Logs branch)"

[flow.main.nodes.source]
component = "event-source"
interface = "torvyn:streaming/source"

[flow.main.nodes.router]
component = "event-router"
interface = "torvyn:filtering/router"

[flow.main.nodes.metric-proc]
component = "metric-processor"
interface = "torvyn:streaming/processor"

[flow.main.nodes.log-proc]
component = "log-processor"
interface = "torvyn:streaming/processor"

[flow.main.nodes.metric-sink]
component = "metric-sink"
interface = "torvyn:streaming/sink"

[flow.main.nodes.log-sink]
component = "log-sink"
interface = "torvyn:streaming/sink"

# Source feeds the router.
[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "router", port = "input" }

# Router "metrics" port feeds the metrics branch.
[[flow.main.edges]]
from = { node = "router", port = "metrics" }
to = { node = "metric-proc", port = "input" }

# Router "logs" port feeds the logs branch.
[[flow.main.edges]]
from = { node = "router", port = "logs" }
to = { node = "log-proc", port = "input" }

# Each branch flows to its sink.
[[flow.main.edges]]
from = { node = "metric-proc", port = "output" }
to = { node = "metric-sink", port = "input" }

[[flow.main.edges]]
from = { node = "log-proc", port = "output" }
to = { node = "log-sink", port = "input" }
```

## Expected Output

```
$ torvyn run flow.main
[torvyn] Running flow 'main'

[metric-sink] Received metric: {"type":"metric","name":"cpu_usage","value":72.5}
[log-sink] Received log: {"type":"log","level":"info","message":"user login successful"}
[metric-sink] Received metric: {"type":"metric","name":"mem_used","value":4096}
[log-sink] Received log: {"type":"log","level":"warn","message":"disk usage above 80%"}

[torvyn] Flow 'main' completed. 4 events routed: 2 to metrics, 2 to logs.
```

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| Router overhead per element | < 15 us (read + string match) |
| Fan-out to N ports | No additional buffer allocation (borrow forwarding) |
| Independent backpressure | Each branch has its own queue and backpressure |

## Learn More

- [WIT Reference: `torvyn:filtering/router`](docs/wit-reference.md#torvyn-filtering-router)
- [Architecture Guide: Topology Patterns](docs/architecture.md#topology-patterns)
