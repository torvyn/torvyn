# Designing the Reactor: Scheduling Challenges of Reactive Streaming

*A technical deep-dive into Torvyn's stream scheduling engine.*

---

The reactor is the execution heartbeat of the Torvyn runtime. It determines which components run, in what order, and how back-pressure propagates through the system. This post describes the key design decisions and the trade-offs behind them.

## The Scheduling Problem

A Torvyn pipeline is a directed graph of components connected by bounded-queue streams. At any moment, multiple components within a flow may be ready to execute: a source has data, a processor has both input available and output capacity, a sink has elements to consume. The scheduler must decide which component runs next.

The wrong scheduling order creates problems. Running producers before consumers fills queues, increases memory pressure, and triggers unnecessary back-pressure. Running consumers before producers drains queues faster and keeps the pipeline responsive. But a naive consumer-first policy can starve sources and reduce throughput.

## Task-Per-Flow, Not Task-Per-Component

The first major decision was the reactor's task architecture. We chose **task-per-flow**: each active pipeline execution gets its own Tokio async task. Within that task, the flow driver executes pipeline stages sequentially in dependency order.

The alternative — task-per-component — would assign each component its own Tokio task. For pipelines with many stages (20+ components per flow across hundreds of flows), this creates thousands of Tokio tasks, each with its own waker, stack, and scheduling overhead. More importantly, it gives up intra-flow scheduling control to Tokio's work-stealing scheduler, which knows nothing about stream dependencies or demand propagation.

Task-per-flow gives the reactor control over intra-flow scheduling policy. The flow driver can execute stages in dependency order, yield to Tokio between stages, and adjust scheduling based on back-pressure state — all within a single async task. Tokio handles inter-flow scheduling (distributing flow driver tasks across OS threads), while the reactor handles intra-flow scheduling (deciding which component runs next within a flow).

The trade-off is that a single flow cannot utilize multiple OS threads simultaneously for different stages. For Torvyn's v1 target of same-node pipelines, this is acceptable — the bottleneck is typically Wasm execution speed per stage, not parallelism within a single flow. The design is similar to how Apache Flink's task slots chain operators sequentially within a slot.

## Consumer-First, Demand-Driven Scheduling

Within a flow, the default scheduling policy is **demand-driven, consumer-first**. The scheduler starts from the sink (the terminal consumer), walks upstream looking for ready stages, and executes the first stage it finds with both input available and output capacity.

This pull-based traversal naturally prevents queue buildup. Work only happens when there is downstream capacity to consume the result. It follows the same principle as the Reactive Streams specification's `request(n)` pattern: consumers drive the pipeline by expressing demand.

## Credit-Based Demand Model

Each stream maintains a demand counter: the number of elements the consumer is willing to accept. Consumers replenish demand by processing elements. Producers consume demand by enqueuing elements. When demand reaches zero, the producer pauses — this is the back-pressure trigger.

Demand propagation follows the pipeline graph from consumer to producer. When a sink processes an element, the reactor increments demand on the sink's input stream, checks if the upstream processor can now produce, and if so, propagates demand further upstream until it reaches the source.

For fan-out topologies (one producer, multiple consumers), the producer's effective demand is the minimum across all downstream branches. This prevents the producer from outrunning the slowest consumer.

## Back-Pressure with Hysteresis

When a stream's queue reaches capacity under the default Block policy, the producer is suspended. But when should the producer resume? Resuming immediately when one element is consumed creates rapid oscillation between backpressured and normal states when the consumer is only slightly slower than the producer.

Torvyn uses high/low watermarks with hysteresis. Back-pressure activates when the queue reaches 100% capacity. It deactivates when the queue drops below 50% capacity (the low watermark, configurable per stream). This provides stability: the producer stays paused until the queue has drained substantially, then resumes and can produce a burst of elements before hitting capacity again.

Every state transition (back-pressure triggered, back-pressure relieved) emits a structured observability event with timestamp, stream identifier, queue depth, and duration.

## Cooperative Yielding and Fairness

A flow driver that never yields to Tokio starves other flows of CPU time. The reactor enforces cooperative yielding: after processing a configurable batch of elements (default: 32) or after a configurable time quantum (default: 100 microseconds), the flow driver yields to Tokio's scheduler.

Flow priority adjusts the yield frequency. Critical flows process up to 128 elements per yield cycle — approximately 4x the CPU share of Normal flows. Background flows process only 8 elements per yield. A hard ceiling (256 elements) prevents any flow from monopolizing a thread regardless of priority.

A watchdog in the reactor coordinator monitors yield timestamps. If any flow driver has not yielded within 10 milliseconds, it logs a warning. This is the safety net against pathological component behavior.

## Cancellation and Cleanup

Cancellation uses a tiered protocol. When a flow is cancelled — by operator command, downstream error, timeout, or resource exhaustion — the reactor first allows the current component invocation to complete (cooperative cancellation). If the invocation does not return within 1 second, it exhausts the component's Wasmtime fuel budget, causing a deterministic Wasm trap. If fuel exhaustion fails within 500 milliseconds (a pathological case), the host drops the component instance entirely.

After cancellation, all stream queues drain, resource handles are released, observability events are flushed, and the flow is removed from the reactor's active table. The maximum time from cancellation initiation to full cleanup is bounded and configurable (default: approximately 6.5 seconds).

## Performance Targets

The reactor's per-element overhead target is < 5 microseconds, covering scheduler decision, queue operations, demand accounting, back-pressure check, and observability event recording. The wakeup latency target is < 10 microseconds from data availability to consumer invocation. The reactor supports 1,000+ concurrent flows, with each flow driver task contributing approximately 256-512 bytes of Tokio task overhead.

These are engineering targets for Phase 0 benchmarks. Results will be published with full methodology.

## What We Are Still Deciding

Several design questions remain open: optimal default values for yield quantum, the best bounded queue implementation (VecDeque vs. custom ring buffer), the fan-in merge policy (first-available vs. round-robin), and whether intra-flow parallelism (executing independent stages concurrently within a flow) provides meaningful throughput improvements for realistic topologies. These decisions will be informed by benchmark data from Phase 0.
