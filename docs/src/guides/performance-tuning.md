# Performance Tuning Guide

This guide explains how to optimize Torvyn pipeline performance. It assumes you have a working pipeline and want to improve throughput, reduce latency, or lower resource consumption.

## Start with `torvyn bench`

Before tuning anything, establish a baseline. Run:

```
torvyn bench --duration 30s --warmup 5s --baseline my-baseline
```

This produces a comprehensive report covering throughput, latency percentiles, per-component breakdown, resource utilization, and scheduling statistics. Save the result as a named baseline for comparison after each change.

After each tuning adjustment, re-run the benchmark and compare:

```
torvyn bench --duration 30s --warmup 5s --compare .torvyn/bench/my-baseline.json
```

## Reading and Interpreting Benchmark Reports

**Throughput section:** Elements per second and bytes per second. If throughput is lower than expected, the bottleneck is either a slow component, queue contention, or insufficient CPU threads.

**Latency section:** p50, p90, p95, p99, p999, and max. A large gap between p50 and p99 indicates tail latency variance — occasional slow processing caused by backpressure episodes, fuel exhaustion, or component-internal causes.

**Per-component latency:** Identifies the slowest stage. The component with the highest p50 latency is usually the throughput bottleneck.

**Resources section:** Buffer allocations, pool reuse rate, total copies, and peak memory. A low pool reuse rate (< 80%) suggests the pool is undersized. High copy counts with a high copy-amplification ratio suggest unnecessary data access patterns.

**Scheduling section:** Total wakeups, backpressure events, and queue peak. Frequent backpressure events indicate a consumer bottleneck. A queue peak near capacity suggests the queue is appropriately sized (the pipeline is backpressure-aware). A queue peak far below capacity suggests the producer is slower than the consumer.

## Identifying Bottlenecks

### CPU-bound

**Symptom:** High throughput but high per-component latency. `component.processing_time` is large relative to total end-to-end latency. The component is spending most of its time in computation.

**Remedy:** Optimize component code. Reduce the computational cost per element. Consider whether the component can operate on metadata alone (avoiding payload read/write copies). If the component must be CPU-intensive, ensure `fuel_per_invocation` is set high enough to avoid premature preemption.

### Memory-bound

**Symptom:** High `pool.fallback_count` (buffers allocated outside the pool) or frequent `pool.exhaustion_events`. Component `memory_current` is near the configured limit.

**Remedy:** Increase buffer pool sizes for the relevant tier. Increase `max_memory_per_component` if component linear memory is constrained. Check whether components are holding buffers longer than necessary.

### Backpressure-bound

**Symptom:** `stream.backpressure.events` is high on a specific stream. `stream.backpressure.duration_ns` is a significant fraction of total flow time.

**Remedy:** The downstream component on the backpressured stream is the bottleneck. Optimize that component's processing time. Alternatively, increase the queue depth on the affected stream to absorb bursts (this trades memory for latency stability, not for throughput).

### Copy-bound

**Symptom:** High `flow.copies.bytes` relative to the data volume. Per-component copy counts are higher than expected (more than 2 copies per stage for a transform).

**Remedy:** Review component data access patterns. If a filter or router is calling `buffer.read-all()` when it only needs metadata, change it to use `buffer.size()` and `buffer.content-type()` instead. If a processor reads the entire payload but only modifies a small portion, the current "new-buffer" pattern is correct (there is no copy-on-write in Phase 0), but the read copy is still necessary.

## Tuning Configuration

### Queue Depths

Increase `default_queue_depth` (or per-stream `queue_depth`) to absorb bursts from bursty sources. Decrease it for latency-sensitive pipelines where you prefer backpressure over queuing delay. The default of 64 is a balanced starting point.

### Pool Sizes

If `pool.fallback_count` is non-zero for a tier, increase that tier's pool size. The goal is for the pool to handle steady-state allocation without falling back to the system allocator. For bursty workloads, size the pool for peak, not average.

### Fuel Budgets

If `component.fuel_consumed` is frequently hitting the `fuel_per_invocation` limit (observable as `E0401 ComponentTimeout` errors or as elevated tail latency), increase the fuel budget. Setting fuel to 0 disables fuel metering, which removes the per-instruction overhead but also removes CPU time protection.

### Yield Frequency

The reactor yields to Tokio after processing a batch of elements (default: 32) or after a time quantum (default: 100 microseconds). Increasing the batch size improves throughput (fewer context switches) but can increase latency for other flows. Decreasing it improves inter-flow fairness at the cost of throughput.

### Worker Threads

Set `worker_threads` to match available CPU cores. More threads than cores provides no benefit and adds context-switch overhead. Fewer threads than concurrent flows may cause scheduling delays.

## Pipeline Topology Optimization

**Minimize pipeline depth.** Each component boundary adds Wasm invocation overhead (~100-500 ns) and potentially two payload copies. If two adjacent stages can be combined into a single component without sacrificing modularity or reusability, the combined version will be faster.

**Put filters early.** Filters that reject elements are extremely cheap (no buffer allocation, no payload copy). Placing filters early in the pipeline reduces the amount of data processed by downstream stages.

**Use metadata-only routing.** If routing decisions can be made from element metadata alone (content-type, size, sequence number), the router operates on the handle-pass fast path with zero payload copies.

**Consider batch processing for high-throughput workloads.** Processing elements one at a time incurs per-call overhead for each Wasm invocation. A future `batch-processor` interface that processes `list<stream-element>` could amortize this cost. This is planned for Phase 1+ based on benchmark data.

## Advanced: Component-Level Performance Profiling

For deep investigation of a specific component's performance:

1. **Use `torvyn trace --limit 100 --show-buffers`** to see exactly what buffer operations the component performs per element. Look for unnecessary `read-all` calls, oversized buffer allocations, or multiple small writes that could be consolidated into a single write.

2. **Check Wasm fuel consumption.** High fuel per invocation relative to processing time suggests the component has an inefficient inner loop. Consider optimizing the source code or the compilation settings (optimization level, LTO).

3. **Profile the component outside Torvyn.** Compile the component as a native binary (not Wasm) and profile it with standard tools (`perf`, Instruments, etc.) to identify hot functions. Optimize those functions, then recompile to Wasm and re-benchmark in Torvyn.

4. **Measure copy overhead separately.** Run the pipeline with a pass-through component (one that returns the input unchanged, using the `drop` variant) to measure the baseline cost of pipeline orchestration without component computation. The difference between this baseline and the actual pipeline is the component computation cost.
