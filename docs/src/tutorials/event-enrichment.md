# Tutorial: Event Enrichment Pipeline

This tutorial builds a pipeline that reads events, enriches them with metadata, filters by criteria, and writes the enriched results. It demonstrates multi-stage processing, the buffer ownership model, and how Torvyn's resource manager tracks copies and allocations across the pipeline.

**What you will build:**

1. **Event Source** — generates structured event records.
2. **Enrichment Processor** — adds metadata (priority, category) to each event.
3. **Priority Filter** — passes only high-priority events.
4. **Event Sink** — writes enriched events to output.

**What you will learn:** Multi-stage pipelines with four components, buffer copy accounting across stages, resource lifecycle visibility, and how the resource manager's ownership tracking works in practice.

**Prerequisites:** Complete [Your First Pipeline](../getting-started/your-first-pipeline.md).

**Time required:** 25–35 minutes.

## Project Setup

```bash
torvyn init enrichment-pipeline --template empty
cd enrichment-pipeline
```

We start with the empty template and build everything from scratch.

Create the component directories:

```bash
mkdir -p components/event-source/wit components/event-source/src
mkdir -p components/enricher/wit components/enricher/src
mkdir -p components/priority-filter/wit components/priority-filter/src
mkdir -p components/event-sink/wit components/event-sink/src
```

## Step 1: Event Source

The source generates JSON-formatted event records.

### `components/event-source/Cargo.toml`

```toml
[package]
name = "event-source"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "event-source:component"
```

### `components/event-source/wit/world.wit`

```wit
package event-source:component;

world event-source {
    import torvyn:streaming/types@0.1.0;
    import torvyn:resources/buffer-ops@0.1.0;
    export torvyn:streaming/source@0.1.0;
}
```

### `components/event-source/src/lib.rs`

```rust
// Event source: generates JSON event records.

wit_bindgen::generate!({
    world: "event-source",
    path: "wit",
});

use exports::torvyn::streaming::source::Guest;
use torvyn::streaming::types::{OutputElement, ElementMeta, ProcessError, BackpressureSignal};
use torvyn::resources::buffer_ops;

struct EventSource;

static mut COUNTER: u64 = 0;

struct EventTemplate {
    event_type: &'static str,
    source: &'static str,
    severity: &'static str,
}

const EVENTS: &[EventTemplate] = &[
    EventTemplate { event_type: "login",         source: "auth-service",  severity: "info" },
    EventTemplate { event_type: "purchase",      source: "order-service", severity: "info" },
    EventTemplate { event_type: "error",         source: "api-gateway",   severity: "high" },
    EventTemplate { event_type: "login_failed",  source: "auth-service",  severity: "high" },
    EventTemplate { event_type: "page_view",     source: "web-frontend",  severity: "low" },
    EventTemplate { event_type: "deployment",    source: "ci-pipeline",   severity: "high" },
    EventTemplate { event_type: "health_check",  source: "monitor",       severity: "low" },
    EventTemplate { event_type: "rate_limited",  source: "api-gateway",   severity: "high" },
];

impl Guest for EventSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let count = unsafe {
            COUNTER += 1;
            COUNTER
        };

        if count > 50 {
            return Ok(None);
        }

        let template = &EVENTS[((count - 1) as usize) % EVENTS.len()];
        let json = format!(
            r#"{{"id":"evt_{:06}","type":"{}","source":"{}","severity":"{}"}}"#,
            count, template.event_type, template.source, template.severity
        );

        let buf = buffer_ops::allocate(json.len() as u64)
            .map_err(|_| ProcessError::Internal("allocation failed".into()))?;
        buf.append(json.as_bytes())
            .map_err(|_| ProcessError::Internal("write failed".into()))?;
        buf.set_content_type("application/json");

        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: 0,
                timestamp_ns: 0,
                content_type: "application/json".into(),
            },
            payload: buf.freeze(),
        }))
    }

    fn notify_backpressure(_signal: BackpressureSignal) {}
}

export!(EventSource);
```

## Step 2: Enrichment Processor

The enricher reads each event, parses the JSON, adds enrichment fields (a priority score and a category), and writes an enriched JSON record to a new buffer.

### `components/enricher/Cargo.toml`

```toml
[package]
name = "enricher"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "enricher:component"
```

### `components/enricher/wit/world.wit`

```wit
package enricher:component;

world enricher {
    import torvyn:streaming/types@0.1.0;
    import torvyn:resources/buffer-ops@0.1.0;
    export torvyn:streaming/processor@0.1.0;
}
```

### `components/enricher/src/lib.rs`

```rust
// Enrichment processor: adds priority score and category to events.

wit_bindgen::generate!({
    world: "enricher",
    path: "wit",
});

use exports::torvyn::streaming::processor::{Guest, ProcessResult};
use torvyn::streaming::types::{StreamElement, OutputElement, ElementMeta, ProcessError};
use torvyn::resources::buffer_ops;

struct Enricher;

impl Guest for Enricher {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        // Read the input event.
        // NOTE: This is a measured copy — the resource manager records it.
        let data = input.payload.read_all();
        let text = String::from_utf8_lossy(&data);

        // Simple enrichment: extract severity and assign a priority score.
        let priority = if text.contains(r#""severity":"high""#) {
            100
        } else if text.contains(r#""severity":"info""#) {
            50
        } else {
            10
        };

        let category = if text.contains(r#""source":"auth-service""#) {
            "security"
        } else if text.contains(r#""source":"order-service""#) {
            "business"
        } else if text.contains(r#""source":"ci-pipeline""#) {
            "operations"
        } else {
            "general"
        };

        // Build enriched JSON by appending fields.
        // In production, you would use a proper JSON library.
        let enriched = if text.ends_with('}') {
            format!(
                r#"{},"priority":{},"category":"{}"}}"#,
                &text[..text.len() - 1],
                priority,
                category
            )
        } else {
            text.to_string()
        };

        // Allocate a new buffer for the enriched output.
        // NOTE: This is a second allocation — the bench report will show it.
        let out_buf = buffer_ops::allocate(enriched.len() as u64)
            .map_err(|_| ProcessError::Internal("allocation failed".into()))?;
        out_buf.append(enriched.as_bytes())
            .map_err(|_| ProcessError::Internal("write failed".into()))?;
        out_buf.set_content_type("application/json");

        Ok(ProcessResult::Emit(OutputElement {
            meta: ElementMeta {
                sequence: input.meta.sequence,
                timestamp_ns: input.meta.timestamp_ns,
                content_type: "application/json".into(),
            },
            payload: out_buf.freeze(),
        }))
    }
}

export!(Enricher);
```

The comments in the code highlight copy and allocation events. The `read_all()` call is a copy from host memory into the component's linear memory. The `allocate()` call creates a new buffer. Both events are recorded by the resource manager and will appear in `torvyn trace` and `torvyn bench` output. This is Torvyn's copy accounting in action: copies are not hidden — they are measured and reported.

## Step 3: Priority Filter

The priority filter examines the enriched event and passes only high-priority events (priority >= 100).

### `components/priority-filter/Cargo.toml`

```toml
[package]
name = "priority-filter"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "priority-filter:component"
```

### `components/priority-filter/wit/world.wit`

```wit
package priority-filter:component;

world priority-filter {
    import torvyn:streaming/types@0.1.0;
    export torvyn:filtering/filter@0.1.0;
}
```

### `components/priority-filter/src/lib.rs`

```rust
// Priority filter: passes only high-priority events.

wit_bindgen::generate!({
    world: "priority-filter",
    path: "wit",
});

use exports::torvyn::filtering::filter::Guest;
use torvyn::streaming::types::{StreamElement, ProcessError};

struct PriorityFilter;

impl Guest for PriorityFilter {
    fn evaluate(input: &StreamElement) -> Result<bool, ProcessError> {
        let data = input.payload.read_all();
        let text = String::from_utf8_lossy(&data);

        // Pass only events with priority >= 100.
        Ok(text.contains(r#""priority":100"#))
    }
}

export!(PriorityFilter);
```

## Step 4: Event Sink

### `components/event-sink/Cargo.toml`

```toml
[package]
name = "event-sink"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "event-sink:component"
```

### `components/event-sink/wit/world.wit`

```wit
package event-sink:component;

world event-sink {
    import torvyn:streaming/types@0.1.0;
    export torvyn:streaming/sink@0.1.0;
}
```

### `components/event-sink/src/lib.rs`

```rust
// Event sink: prints enriched, filtered events.

wit_bindgen::generate!({
    world: "event-sink",
    path: "wit",
});

use exports::torvyn::streaming::sink::Guest;
use torvyn::streaming::types::{StreamElement, BackpressureSignal, ProcessError};

struct EventSink;

static mut EVENT_COUNT: u64 = 0;

impl Guest for EventSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let data = element.payload.read_all();
        let text = String::from_utf8_lossy(&data);

        unsafe { EVENT_COUNT += 1; }
        let count = unsafe { EVENT_COUNT };

        println!("[event {count}] {text}");

        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        let count = unsafe { EVENT_COUNT };
        println!("\n── Pipeline Complete ──");
        println!("High-priority events delivered: {count}");
        Ok(())
    }
}

export!(EventSink);
```

## Step 5: Pipeline Configuration

Create `Torvyn.toml`:

```toml
[torvyn]
name = "enrichment-pipeline"
version = "0.1.0"
description = "Event enrichment with priority filtering"
contract_version = "0.1.0"

[[component]]
name = "event-source"
path = "components/event-source"
language = "rust"

[[component]]
name = "enricher"
path = "components/enricher"
language = "rust"

[[component]]
name = "priority-filter"
path = "components/priority-filter"
language = "rust"

[[component]]
name = "event-sink"
path = "components/event-sink"
language = "rust"

[flow.main]
description = "Generate events → enrich → filter by priority → deliver"

[flow.main.nodes.source]
component = "file://./components/event-source/target/wasm32-wasip2/debug/event_source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.enricher]
component = "file://./components/enricher/target/wasm32-wasip2/debug/enricher.wasm"
interface = "torvyn:streaming/processor"

[flow.main.nodes.filter]
component = "file://./components/priority-filter/target/wasm32-wasip2/debug/priority_filter.wasm"
interface = "torvyn:filtering/filter"

[flow.main.nodes.sink]
component = "file://./components/event-sink/target/wasm32-wasip2/debug/event_sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "enricher", port = "input" }

[[flow.main.edges]]
from = { node = "enricher", port = "output" }
to = { node = "filter", port = "input" }

[[flow.main.edges]]
from = { node = "filter", port = "output" }
to = { node = "sink", port = "input" }
```

## Step 6: Build and Run

Build all components:

```bash
for component in event-source enricher priority-filter event-sink; do
    (cd "components/$component" && cargo component build --target wasm32-wasip2)
done
```

Validate and run:

```bash
torvyn check
torvyn link
torvyn run
```

Expected output shows only the high-priority events:

```
▶ Running flow "main"

[event 1] {"id":"evt_000003","type":"error","source":"api-gateway","severity":"high","priority":100,"category":"general"}
[event 2] {"id":"evt_000004","type":"login_failed","source":"auth-service","severity":"high","priority":100,"category":"security"}
[event 3] {"id":"evt_000006","type":"deployment","source":"ci-pipeline","severity":"high","priority":100,"category":"operations"}
[event 4] {"id":"evt_000008","type":"rate_limited","source":"api-gateway","severity":"high","priority":100,"category":"general"}
...

── Pipeline Complete ──
High-priority events delivered: 25
```

Of the 50 source events, only those with severity "high" pass the priority filter. Each has been enriched with a priority score and a category.

## Step 7: Observe Copy Accounting

Run `torvyn bench --duration 5s` and examine the Resources section of the report. You will see:

- **Buffer allocations** from the source (one per event) and the enricher (one per event — it creates a new buffer for enriched output).
- **Total copies** from the enricher's `read_all()` call and the filter's `read_all()` call.
- **Pool reuse rate** showing how effectively the runtime's buffer pool recycles allocations.

This is the resource ownership accounting described in the Torvyn design: every allocation, copy, and deallocation is tracked and reported. You can use this data to identify optimization opportunities — for example, if a processor could use buffer metadata instead of reading the full payload, it would eliminate a copy.

## Concepts Demonstrated

- **Four-stage pipeline** — source → processor → filter → sink.
- **Copy accounting** — every `read_all()` is a measured copy visible in benchmarks and traces.
- **Filter efficiency** — the priority filter drops events without allocating output buffers.
- **Enrichment pattern** — read input, compute new fields, write to a new buffer, transfer ownership.
- **Resource lifecycle visibility** — `torvyn bench` reports exactly how many buffers were allocated, reused, and copied.
