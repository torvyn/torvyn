# AI Inference Pipelines

## The Problem

AI-native applications are no longer simple request-response systems. A modern inference pipeline may chain together token stream ingestion, embedding generation, retrieval-augmented generation (RAG) lookups, model scoring, policy filtering, content guard evaluation, caching, and downstream delivery — all on the same node, all with latency requirements measured in milliseconds.

Teams building these pipelines face a difficult choice. Running all stages in a single process provides low latency but sacrifices isolation: a bug in a policy filter can corrupt model output, and updating one stage risks destabilizing the entire system. Splitting stages into microservices provides isolation but introduces serialization overhead, network latency, and operational complexity that real-time workloads cannot absorb.

The result is that most teams build ad hoc in-process glue code: custom thread pools, manual buffer management, bespoke back-pressure logic, and fragile error handling that is different for every pipeline.

## How Torvyn Solves It

Torvyn provides a structured alternative. Each inference stage — tokenizer, embedding encoder, retrieval stage, model adapter, policy filter, content guard, cache layer, delivery endpoint — is implemented as an isolated WebAssembly component with a typed contract.

**Isolation without serialization overhead.** Components run in sandboxed Wasm environments with their own linear memory. Data transfers between stages use host-managed buffers with explicit ownership. Where a stage only inspects metadata (routing, content-type checks, trace context), the payload buffer passes through without a copy.

**Built-in back-pressure.** When a model scoring stage is slower than the tokenizer upstream, Torvyn's reactor automatically propagates demand signals upstream, pausing producers until the bottleneck clears. Queue depths are bounded and configurable. No stage can overwhelm another.

**End-to-end tracing.** Every element carries a trace context through the entire pipeline. Torvyn's observability layer records per-stage latency, queue wait time, back-pressure events, copy counts, and resource ownership transfers — providing the visibility that production inference systems require.

**Modular updates.** Updating a policy filter or swapping a model adapter means recompiling a single Wasm component. Contracts guarantee compatibility. `torvyn link` validates that the updated component still satisfies the pipeline's interface requirements before it reaches production.

## Example Pipeline

```
Token Source → Embedding Encoder → RAG Retrieval → Model Scorer → Policy Filter → Content Guard → Response Sink
```

Each arrow is a contract-defined, back-pressure-aware stream with bounded queuing and host-managed resource transfer. The entire pipeline runs on a single node with single-digit microsecond reactor overhead per stream element.

## Performance Characteristics

Torvyn targets < 5us of reactor overhead per stream element and < 10us wakeup latency from data availability to consumer invocation. For inference pipelines where end-to-end latency budgets are measured in tens of milliseconds, Torvyn's scheduling overhead is a negligible fraction of the total.

Copy behavior is measurable: `torvyn bench` reports exactly how many bytes cross each component boundary and why, enabling data-informed optimization of the pipeline topology.

## Get Started

- [Quickstart guide](/docs/quickstart)
- [Tutorial: building an inference pipeline](/docs/tutorials/inference-pipeline)
- [Architecture guide: reactor and scheduling](/docs/architecture/reactor)
