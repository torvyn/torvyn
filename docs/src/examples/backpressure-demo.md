# Backpressure Demo

## What It Demonstrates

A fast source producing 1,000 elements/sec connected to a deliberately slow sink processing 100 elements/sec. Demonstrates Torvyn's built-in backpressure mechanism: queue depth monitoring, watermark-based flow control, and the `BackpressureSignal` enum.

## Concepts Covered

- Backpressure activation and deactivation
- Queue depth and watermark behavior (high watermark = queue capacity, low watermark = 50% per `DEFAULT_LOW_WATERMARK_RATIO`)
- Default queue depth of 64 elements (per `DEFAULT_QUEUE_DEPTH`)
- `BackpressureSignal::Pause` and `BackpressureSignal::Ready`
- Observing backpressure via `torvyn trace`

## Source Component: `fast-source`

**`components/fast-source/src/lib.rs`**

```rust
//! Fast source — produces elements as quickly as possible.
//! Emits 1,000 numbered elements, respecting backpressure signals.

wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct FastSource {
    total: u64,
    produced: u64,
    paused: bool,
}

static mut STATE: Option<FastSource> = None;
fn state() -> &'static mut FastSource {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for FastSource {
    fn init(config: String) -> Result<(), ProcessError> {
        let total = config.trim().parse::<u64>().unwrap_or(1000);
        unsafe { STATE = Some(FastSource { total, produced: 0, paused: false }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl SourceGuest for FastSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();

        // Respect backpressure: if paused, return no element.
        // The runtime will poll again after backpressure clears.
        if s.paused {
            return Ok(None);
        }

        if s.produced >= s.total {
            return Ok(None);
        }

        let msg = format!("element-{}", s.produced);
        let bytes = msg.as_bytes();
        let buf = buffer_allocator::allocate(bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(bytes).map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("text/plain");
        let frozen = buf.freeze();
        s.produced += 1;

        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: s.produced - 1,
                timestamp_ns: 0,
                content_type: "text/plain".to_string(),
            },
            payload: frozen,
        }))
    }

    fn notify_backpressure(signal: BackpressureSignal) {
        let s = state();
        match signal {
            BackpressureSignal::Pause => {
                s.paused = true;
                // In a real source, you would also stop reading from
                // the external data source (network, file, etc.).
            }
            BackpressureSignal::Ready => {
                s.paused = false;
            }
        }
    }
}

export!(FastSource);
```

## Sink Component: `slow-sink`

**`components/slow-sink/src/lib.rs`**

```rust
//! Slow sink — simulates a slow consumer by introducing a delay
//! per element via busy-waiting (since WASI sleep is not available
//! in all environments).

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct SlowSink {
    received: u64,
    delay_iterations: u64,
}

static mut STATE: Option<SlowSink> = None;
fn state() -> &'static mut SlowSink {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for SlowSink {
    fn init(config: String) -> Result<(), ProcessError> {
        // delay_iterations controls how slow the sink is.
        // Higher = slower. Calibrate for your hardware.
        let delay = config.trim().parse::<u64>().unwrap_or(100_000);
        unsafe { STATE = Some(SlowSink { received: 0, delay_iterations: delay }); }
        Ok(())
    }
    fn teardown() {
        let s = state();
        println!("[slow-sink] Processed {} elements total.", s.received);
        unsafe { STATE = None; }
    }
}

impl SinkGuest for SlowSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let s = state();

        // Simulate slow processing with a busy loop.
        // In a real sink, this delay would come from I/O (database writes,
        // network calls, disk writes, etc.).
        let mut acc: u64 = 0;
        for i in 0..s.delay_iterations {
            acc = acc.wrapping_add(i);
        }
        // Prevent the optimizer from removing the loop.
        if acc == u64::MAX { println!("{acc}"); }

        let bytes = element.payload.read_all();
        let text = String::from_utf8_lossy(&bytes);
        if s.received % 100 == 0 {
            println!("[slow-sink] seq={}: {} (every 100th logged)", element.meta.sequence, text);
        }

        s.received += 1;
        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        println!("[slow-sink] Stream complete.");
        Ok(())
    }
}

export!(SlowSink);
```

## Pipeline Configuration

**`Torvyn.toml`**

```toml
[torvyn]
name = "backpressure-demo"
version = "0.1.0"
contract_version = "0.1.0"
description = "Demonstrates backpressure between a fast source and slow sink"

[[component]]
name = "fast-source"
path = "components/fast-source"

[[component]]
name = "slow-sink"
path = "components/slow-sink"

[flow.main]
description = "Fast source → Slow sink (backpressure active)"

# Override queue depth to a small value so backpressure activates quickly.
default_queue_depth = 16

[flow.main.nodes.source]
component = "fast-source"
interface = "torvyn:streaming/source"
config = "1000"

[flow.main.nodes.sink]
component = "slow-sink"
interface = "torvyn:streaming/sink"
config = "100000"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }
```

## How to Observe Backpressure

Run with tracing to see backpressure events in real time:

```bash
$ torvyn trace flow.main
```

The trace output will include entries like:

```
[trace] flow=main stream=source→sink queue_depth=16/16 backpressure=ACTIVATED
[trace] flow=main stream=source→sink source notified: PAUSE
[trace] flow=main stream=source→sink queue_depth=8/16  backpressure=DEACTIVATED (low watermark)
[trace] flow=main stream=source→sink source notified: READY
```

The default behavior: when the queue between source and sink fills to capacity (16 in this configuration), the runtime activates backpressure and sends `BackpressureSignal::Pause` to the source. When the queue drains to the low watermark (50% of capacity = 8 elements, per `DEFAULT_LOW_WATERMARK_RATIO`), the runtime deactivates backpressure and sends `BackpressureSignal::Ready`.

## Expected Output

```
$ torvyn run flow.main
[torvyn] Running flow 'main'

[slow-sink] seq=0: element-0 (every 100th logged)
[slow-sink] seq=100: element-100 (every 100th logged)
[slow-sink] seq=200: element-200 (every 100th logged)
...
[slow-sink] seq=900: element-900 (every 100th logged)
[slow-sink] Stream complete.
[slow-sink] Processed 1000 elements total.

[torvyn] Flow 'main' completed. 1000 elements processed.
[torvyn] Duration: ~10s | Backpressure activations: ~62 | Peak queue depth: 16
```

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| Queue depth (configured) | 16 elements |
| Low watermark | 8 elements (50% of 16) |
| Backpressure overhead | < 500 ns per signal |
| No unbounded queue growth | Queue never exceeds configured depth |

## Commentary

Backpressure is not optional in Torvyn — it is fundamental to the reactive streaming model (per the vision document, Section 5.3). Without backpressure, a fast producer paired with a slow consumer would cause unbounded memory growth. Torvyn's reactor enforces queue bounds and propagates demand signals automatically.

The `notify-backpressure` callback on the source interface gives components the opportunity to respond to flow control signals. A well-behaved source pauses its external data intake when it receives `Pause` and resumes when it receives `Ready`. The runtime handles the mechanics; the component decides the policy.

## Learn More

- [Architecture Guide: Backpressure Model](docs/architecture.md#backpressure) — Queue depths, watermarks, and policies
- [Architecture Guide: Reactor](docs/architecture.md#reactor) — How the scheduler manages flow control
- [CLI Reference: `torvyn trace`](docs/cli.md#torvyn-trace) — Tracing backpressure events
