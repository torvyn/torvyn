# Event-Driven Architectures

## The Problem

Event-driven systems are everywhere — order processing, fraud detection, notification delivery, audit logging, inventory synchronization, real-time analytics. These systems typically chain together multiple processing stages: validation, enrichment, routing, transformation, aggregation, and delivery.

The standard approach is to connect these stages through message brokers (Kafka, RabbitMQ, NATS) or service meshes, with each stage running as an independent microservice. This provides isolation and deployment flexibility, but for many event-driven workloads, the full cost of inter-service communication is unnecessary. When all stages run on the same node, teams still pay for serialization, network stack traversal, retry logic, and schema drift management — even though the data never leaves the machine.

For high-frequency event processing (thousands to hundreds of thousands of events per second), this overhead becomes a significant fraction of the total processing budget.

## How Torvyn Solves It

Torvyn replaces heavyweight inter-service boundaries with lightweight, contract-defined component boundaries. Each event processing stage is a Wasm component with a typed interface. Stages communicate through host-managed streams with bounded queues and built-in back-pressure.

**Typed contracts eliminate schema drift.** WIT contracts define the exact types, fields, and ownership semantics of every event crossing a component boundary. The `torvyn link` command validates compatibility between stages before deployment. Interface changes that break downstream consumers are caught at link time, not in production.

**Back-pressure prevents cascade failures.** When a downstream stage slows down (a delivery endpoint is temporarily unavailable, an enrichment service is under load), back-pressure propagates upstream through the demand model. Queues are bounded. Configurable overflow policies (block, drop-oldest, drop-newest, error, rate-limit) let you define the right behavior for each stage.

**Routing and filtering are first-class.** Torvyn's contract packages include dedicated filter and router interfaces. Filters make accept/reject decisions without allocating new buffers — they inspect metadata and return a boolean. Routers direct events to named output ports for fan-out topologies. Both are observable and type-safe.

**Aggregation with explicit flush semantics.** The aggregator interface provides `ingest` and `flush` functions, supporting windowed aggregation, accumulation, and stateful event processing with well-defined completion semantics.

## Example Pipeline

```
Event Source → Validator → Enricher → Router → [Branch A: Aggregator → Analytics Sink]
                                              → [Branch B: Transformer → Delivery Sink]
```

The router fans events to multiple downstream branches based on event type. Each branch applies its own processing logic. All branches share the same back-pressure model and observability framework.

## Performance Characteristics

For event-driven pipelines processing high-frequency event streams, Torvyn's per-element reactor overhead (target < 5us) and bounded queue depths ensure predictable performance. Back-pressure response time — from queue-full to producer suspension — is measured and reported. Copy accounting shows exactly where serialization overhead occurs, enabling targeted optimization.

## Get Started

- [Quickstart guide](/docs/quickstart)
- [Tutorial: building an event pipeline](/docs/tutorials/event-pipeline)
- [Architecture guide: contracts and WIT design](/docs/architecture/contracts)
