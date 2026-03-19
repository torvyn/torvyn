# Backpressure Demo

A fast source producing 1,000 elements connected to a deliberately slow sink. Demonstrates Torvyn's built-in backpressure mechanism: queue depth monitoring, watermark-based flow control, and the `BackpressureSignal` enum.

## What It Demonstrates

- Backpressure activation and deactivation
- Queue depth and watermark behavior
- `BackpressureSignal::Pause` and `BackpressureSignal::Ready`
- Observing backpressure via `torvyn trace`

## Pipeline Topology

```
fast-source --> slow-sink
```

The fast source produces elements as quickly as possible. The slow sink introduces artificial delay per element. When the queue between them fills up, the runtime activates backpressure.

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

## Observe Backpressure with Tracing

This is the most important way to run this example:

```bash
make trace
```

The trace output will include entries like:

```
[trace] flow=main stream=source->sink queue_depth=16/16 backpressure=ACTIVATED
[trace] flow=main stream=source->sink source notified: PAUSE
[trace] flow=main stream=source->sink queue_depth=8/16  backpressure=DEACTIVATED (low watermark)
[trace] flow=main stream=source->sink source notified: READY
```

## What to Observe

1. **Queue fills up:** The fast source fills the queue to its capacity (16 elements in this configuration).

2. **Backpressure activates:** When the queue hits the high watermark (100% capacity), the runtime sends `BackpressureSignal::Pause` to the source.

3. **Source pauses:** The fast source respects the signal and stops producing elements.

4. **Queue drains:** The slow sink continues consuming elements.

5. **Backpressure deactivates:** When the queue drains to the low watermark (50% of capacity = 8 elements), the runtime sends `BackpressureSignal::Ready`.

6. **Source resumes:** The cycle repeats until all 1,000 elements are processed.

## Expected Output

```
[slow-sink] seq=0: element-0 (every 100th logged)
[slow-sink] seq=100: element-100 (every 100th logged)
...
[slow-sink] seq=900: element-900 (every 100th logged)
[slow-sink] Stream complete.
[slow-sink] Processed 1000 elements total.
```

## Configuration

The pipeline uses a small queue depth (16) to trigger backpressure frequently:

```toml
[flow.main]
default_queue_depth = 16
```

The slow sink uses a busy-wait loop configured via `config = "100000"` (iteration count). Adjust this value to change the sink's speed relative to your hardware.

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| Queue depth (configured) | 16 elements |
| Low watermark | 8 elements (50% of 16) |
| Backpressure overhead | < 500 ns per signal |
| No unbounded queue growth | Queue never exceeds configured depth |

## Learn More

- [Architecture Guide: Backpressure Model](../../docs/architecture.md#backpressure)
- [Architecture Guide: Reactor](../../docs/architecture.md#reactor)
- [CLI Reference: `torvyn trace`](../../docs/cli.md#torvyn-trace)
