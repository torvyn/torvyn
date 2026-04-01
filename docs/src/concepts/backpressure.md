# Backpressure

## What Is Backpressure and Why It Matters

Backpressure is the mechanism by which a slow consumer tells a fast producer to slow down. In any streaming system where stages process data at different speeds, backpressure prevents unbounded queue growth, memory exhaustion, and cascading failures.

Without backpressure, a producer that outpaces its consumer fills an ever-growing queue until the system runs out of memory. With backpressure, the queue has a bounded capacity, and when that capacity is reached, the producer is suspended until the consumer catches up. Memory usage becomes deterministic and bounded.

In Torvyn, backpressure is not optional. It is built into the stream semantics and the reactor's scheduling model. Every stream connection between components has a bounded queue with a configurable backpressure policy. The runtime enforces these policies automatically — components do not need to implement backpressure logic themselves.

## How Torvyn Implements Backpressure

Torvyn uses a credit-based demand model, inspired by the Reactive Streams specification's `request(n)` pattern and TCP's sliding window. Each stream maintains a demand counter: the number of elements the consumer is willing to accept.

When a consumer processes an element, it replenishes one demand credit. When a producer enqueues an element, it consumes one demand credit. A producer with zero demand credits must not produce — this is the backpressure trigger.

When a flow starts, the reactor grants each stream an initial demand equal to the stream's queue capacity. This allows the pipeline to fill up without waiting for explicit demand signals, avoiding a cold-start latency penalty.

Demand propagation follows the pipeline graph from consumer to producer. In a multi-stage pipeline (A → B → C → D), if D (the sink) is slow, the backpressure cascades through the entire pipeline: queue C→D fills, C is suspended, queue B→C fills, B is suspended, queue A→B fills, A (the source) is suspended. The entire pipeline is now backpressured, with bounded queue depths at every stage.

### Backpressure State Machine

The following diagram shows the backpressure state transitions for a single stream:

```mermaid
stateDiagram-v2
    [*] --> Normal: Flow starts — initial demand granted
    Normal --> HighWatermark: Queue reaches capacity
    HighWatermark --> Paused: Producer suspended

    Paused --> LowWatermark: Consumer drains below low watermark
    LowWatermark --> Normal: Producer resumed — demand replenished

    Normal --> Normal: Elements flowing — demand available

    note right of Normal
        Producer has demand credits.
        Elements flow freely.
    end note

    note right of HighWatermark
        Queue at capacity.
        BackpressureEvent::Triggered emitted.
    end note

    note left of Paused
        Producer not scheduled.
        Consumer draining queue.
    end note

    note left of LowWatermark
        Queue below low watermark.
        BackpressureEvent::Relieved emitted.
    end note
```

### Demand Propagation in Multi-Stage Pipelines

In a multi-stage pipeline, backpressure cascades upstream through the entire graph:

```mermaid
graph LR
    A["Source"] -->|"Queue A→B"| B["Processor 1"]
    B -->|"Queue B→C"| C["Processor 2"]
    C -->|"Queue C→D"| D["Sink"]

    style D fill:#E04E2D,stroke:#c0412a,color:#fff
    style C fill:#CA8A04,stroke:#a87003,color:#fff
    style B fill:#CA8A04,stroke:#a87003,color:#fff
    style A fill:#CA8A04,stroke:#a87003,color:#fff
```

When the Sink (D) is slow: Queue C→D fills → C is suspended → Queue B→C fills → B is suspended → Queue A→B fills → Source (A) is suspended. The entire pipeline is backpressured with bounded queue depths at every stage.

### The High/Low Watermark Mechanism

Backpressure uses hysteresis to prevent rapid oscillation between pressured and unpressured states:

- **Backpressure activates** when the queue reaches capacity (the high watermark, effectively 100%).
- **Backpressure deactivates** when the queue drops below the low watermark (default: 50% of capacity).

Without this hysteresis, a system where the consumer is only slightly slower than the producer would oscillate between backpressured and normal on every single element. The watermark gap provides a stability band.

```mermaid
graph LR
    subgraph WatermarkBand["Queue Depth Over Time"]
        direction TB
        HW["High Watermark (100%) ── Backpressure activates"]
        Band["Hysteresis Band<br/><small>Producer remains paused<br/>while queue drains</small>"]
        LW["Low Watermark (50%) ── Backpressure deactivates"]
        Normal["Normal operation zone<br/><small>Producer has demand credits</small>"]
    end

    HW --- Band
    Band --- LW
    LW --- Normal

    style HW fill:#DC2626,stroke:#B91C1C,color:#fff
    style Band fill:#D97706,stroke:#B45309,color:#fff
    style LW fill:#2563EB,stroke:#1D4ED8,color:#fff
    style Normal fill:#16A34A,stroke:#15803D,color:#fff
```

The sequence of events during a backpressure episode:

1. The producer component returns a new element.
2. The flow driver attempts to enqueue the element into the downstream stream's queue.
3. The queue is at capacity. Backpressure is triggered.
4. An observability event is emitted: `BackpressureEvent::Triggered`.
5. The producer is no longer eligible for scheduling. The flow driver focuses on executing downstream stages to drain the queue.
6. When the consumer processes enough elements for the queue to drop below the low watermark, the stream exits backpressure.
7. An observability event is emitted: `BackpressureEvent::Relieved`.
8. The producer is eligible for scheduling again.

```mermaid
sequenceDiagram
    participant Prod as Producer
    participant Q as Stream Queue
    participant FD as Flow Driver
    participant Obs as Observability
    participant Cons as Consumer

    Prod->>Q: enqueue element
    Note over Q: Queue at capacity (high watermark)
    Q->>Obs: BackpressureEvent::Triggered
    Q-->>FD: backpressure active

    FD->>FD: Suspend producer scheduling

    loop Consumer drains queue
        FD->>Cons: invoke process/push
        Cons-->>FD: result
        FD->>Q: dequeue element
    end

    Note over Q: Queue below low watermark
    Q->>Obs: BackpressureEvent::Relieved
    Q-->>FD: backpressure relieved

    FD->>FD: Resume producer scheduling
    Prod->>Q: enqueue element
    Note over Q: Normal flow resumes
```

### Fan-Out and Fan-In Behavior

```mermaid
graph TD
    subgraph FanOut["Fan-Out: One Producer → Multiple Consumers"]
        direction LR
        P1["Producer<br/><small>effective demand =<br/>min(demand A, demand B)</small>"]
        P1 -->|"Stream A"| C1["Consumer A<br/><small>fast</small>"]
        P1 -->|"Stream B"| C2["Consumer B<br/><small>slow</small>"]
    end

    subgraph FanIn["Fan-In: Multiple Producers → One Consumer"]
        direction LR
        PA["Producer A"] -->|"Stream A<br/><small>independent demand</small>"| Merge["Consumer<br/><small>merge policy:<br/>equal or priority</small>"]
        PB["Producer B"] -->|"Stream B<br/><small>independent demand</small>"| Merge
    end

    style P1 fill:#2563EB,stroke:#1D4ED8,color:#fff
    style C1 fill:#16A34A,stroke:#15803D,color:#fff
    style C2 fill:#DC2626,stroke:#B91C1C,color:#fff
    style PA fill:#2563EB,stroke:#1D4ED8,color:#fff
    style PB fill:#2563EB,stroke:#1D4ED8,color:#fff
    style Merge fill:#7C3AED,stroke:#6D28D9,color:#fff
```

For fan-out topologies (one producer, multiple consumers), the producer's effective demand is the minimum across all downstream streams by default. This ensures the producer does not outrun the slowest consumer. An alternative `IndependentPerBranch` mode allows faster consumers to pull ahead.

For fan-in topologies (multiple producers, one consumer), each upstream stream maintains independent demand. The consumer grants demand based on its merge policy (equal allocation across branches or priority-based).

## Configuring Backpressure Policies

Each stream in a pipeline can be configured with a `BackpressurePolicy` that dictates behavior when the queue is full:

| Policy | Behavior | Data Loss | Use Case |
|--------|----------|-----------|----------|
| `Block` (default) | Suspend the producer until the consumer frees capacity. | None | Correctness-critical pipelines where every element must be processed. |
| `DropOldest` | Remove the oldest element in the queue to make room. | Yes (oldest) | Real-time workloads where freshness matters more than completeness (e.g., live sensor data). |
| `DropNewest` | Reject the new element. The producer continues. | Yes (newest) | Rate-limiting scenarios where the latest burst can be safely discarded. |
| `Error` | Return an error to the producer, propagated as a `ProcessError`. | None (but stops) | Pipelines where backpressure indicates a fundamental problem that should halt processing. |
| `RateLimit { max_elements_per_second }` | Delay the producer to maintain a maximum throughput. | None | Pipelines that need predictable throughput without suspension. |

Policies are configured per-stream in the pipeline definition within `Torvyn.toml`:

```toml
[runtime.backpressure]
default_queue_depth = 64
default_policy = "block"
low_watermark_ratio = 0.5

# Override for a specific stream
[flow.main.edges.transform-to-sink.backpressure]
queue_depth = 256
policy = "drop-oldest"
low_watermark_ratio = 0.25
```

## Observing Backpressure in Production

Torvyn exposes several metrics and diagnostic tools for understanding backpressure behavior:

**Per-stream metrics:**
- `stream.backpressure.events` — Total number of backpressure episodes on this stream.
- `stream.backpressure.duration_ns` — Total time spent in backpressure.
- `stream.queue.current_depth` — Current queue depth (a gauge).
- `stream.queue.peak_depth` — Maximum queue depth observed.

**In `torvyn bench` reports:** The scheduling section shows total backpressure events and queue peak across all streams. A pipeline with zero backpressure events under sustained load typically means the source is slower than the pipeline's processing capacity. Frequent backpressure events indicate a consumer bottleneck.

**In `torvyn trace` output:** With `--show-backpressure`, trace output highlights backpressure events inline with element processing, showing which stream triggered, how long the producer was suspended, and how many elements were drained before the pressure was relieved.

**Via the inspection API:** The `GET /flows/{flow_id}` endpoint returns current queue depths and backpressure state for every stream in the flow.

## Common Backpressure Patterns and Anti-Patterns

**Pattern: End-to-end bounded memory.** With the `Block` policy, total pipeline memory is deterministic: `Σ(queue_capacity × max_element_size)` for all streams. This is the recommended default for production pipelines where correctness matters more than drop tolerance.

**Pattern: Fresh-data preference.** For live data feeds (sensor streams, market data), use `DropOldest` to ensure the consumer always processes the most recent data when it falls behind.

**Pattern: Backpressure-driven autoscaling.** Monitor `stream.backpressure.duration_ns` over time. Sustained backpressure on a specific stream indicates that the downstream component is the bottleneck. This metric can drive operational decisions about resource allocation.

**Anti-pattern: Unbounded queues.** Setting `queue_depth` to an extremely large value (e.g., 1,000,000) effectively disables backpressure and returns to the failure mode of unbounded queue growth. If you find yourself setting very large queue depths, reconsider whether the pipeline topology or component performance should be addressed instead.

**Anti-pattern: Ignoring backpressure metrics.** Backpressure events are not errors — they are a healthy signal that the system is self-regulating. However, persistent backpressure indicates a capacity imbalance. Monitor and investigate pipelines where backpressure events are sustained.

**Anti-pattern: Over-aggressive watermarks.** Setting the low watermark very high (e.g., 0.95) reduces the hysteresis band and can cause rapid oscillation. The default of 0.5 provides a stable equilibrium for most workloads.
