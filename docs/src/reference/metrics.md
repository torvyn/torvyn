# Metrics Catalog

All Torvyn metrics are pre-allocated at flow creation time. Counters are `AtomicU64` with `Relaxed` ordering on the hot path. Histograms use fixed-bucket boundaries with logarithmic distribution from 100 ns to 10 s.

## Per-Flow Metrics

| Metric Name | Type | Unit | Labels | Description |
|-------------|------|------|--------|-------------|
| `flow.elements.total` | Counter | count | `flow_id` | Total stream elements processed. |
| `flow.elements.errors` | Counter | count | `flow_id` | Total elements that produced errors. |
| `flow.latency` | Histogram | ns | `flow_id` | End-to-end latency per element (source entry to sink exit). |
| `flow.throughput` | Derived | elements/s | `flow_id` | Computed from element count and wall time during export. |
| `flow.copies.total` | Counter | count | `flow_id` | Total buffer copy operations. |
| `flow.copies.bytes` | Counter | bytes | `flow_id` | Total bytes copied across all copy operations. |
| `flow.active_duration` | Gauge | ns | `flow_id` | Wall time since flow started. |
| `flow.state` | Gauge (enum) | — | `flow_id` | Current flow lifecycle state. |

## Per-Component Metrics

| Metric Name | Type | Unit | Labels | Description |
|-------------|------|------|--------|-------------|
| `component.invocations` | Counter | count | `flow_id`, `component_id` | Total invocations of this component. |
| `component.errors` | Counter | count | `flow_id`, `component_id` | Total error returns. |
| `component.processing_time` | Histogram | ns | `flow_id`, `component_id` | Wall time per invocation (excludes queue wait). |
| `component.fuel_consumed` | Counter | units | `flow_id`, `component_id` | Wasm fuel consumed (if fuel metering enabled). |
| `component.memory_current` | Gauge | bytes | `flow_id`, `component_id` | Current Wasm linear memory size. |

## Per-Stream Metrics

| Metric Name | Type | Unit | Labels | Description |
|-------------|------|------|--------|-------------|
| `stream.elements.transferred` | Counter | count | `flow_id`, `stream_id` | Total elements transferred through this stream. |
| `stream.backpressure.events` | Counter | count | `flow_id`, `stream_id` | Total backpressure activation events. |
| `stream.backpressure.duration_ns` | Counter | ns | `flow_id`, `stream_id` | Total time spent in backpressure. |
| `stream.queue.current_depth` | Gauge | count | `flow_id`, `stream_id` | Current queue depth. |
| `stream.queue.peak_depth` | Gauge | count | `flow_id`, `stream_id` | Maximum queue depth observed. |
| `stream.queue.wait_time` | Histogram | ns | `flow_id`, `stream_id` | Time each element spent waiting in the queue. |

## Resource Pool Metrics

| Metric Name | Type | Unit | Labels | Description |
|-------------|------|------|--------|-------------|
| `pool.capacity` | Gauge | count | `tier` | Total slots in this pool tier. |
| `pool.available` | Gauge | count | `tier` | Free buffers currently available. |
| `pool.allocated` | Counter | count | `tier` | Total buffers allocated since startup. |
| `pool.returned` | Counter | count | `tier` | Total buffers returned since startup. |
| `pool.fallback_count` | Counter | count | `tier` | Allocations that fell back to system allocator. |
| `pool.exhaustion_events` | Counter | count | `tier` | Times the free list was empty when allocation was requested. |
| `pool.reuse_rate` | Derived | ratio | `tier` | `returned / allocated` (computed during export). |

## Per-Capability Metrics

| Metric Name | Type | Unit | Labels | Description |
|-------------|------|------|--------|-------------|
| `capability.exercises` | Counter | count | `component_id`, `capability` | Times a capability was exercised. |
| `capability.denials` | Counter | count | `component_id`, `capability` | Times a capability was denied. |

## System-Level Metrics

| Metric Name | Type | Unit | Description |
|-------------|------|------|-------------|
| `system.flows.active` | Gauge | count | Currently active flows. |
| `system.components.active` | Gauge | count | Currently instantiated components. |
| `system.memory.total` | Gauge | bytes | Total memory (host + all linear memories). |
| `system.memory.host` | Gauge | bytes | Host-side memory (tables, queues, metrics). |
| `system.scheduler.wakeups` | Counter | count | Total scheduler wakeup events. |
| `system.scheduler.idle_ns` | Counter | ns | Time spent idle (no work available). |
| `system.spans_dropped` | Counter | count | Trace spans dropped due to export backpressure. |

## Querying Metrics

**Prometheus:** Scrape `http://localhost:<port>/metrics` (or the Unix domain socket at `$TORVYN_STATE_DIR/torvyn.sock`). All metrics are exported in Prometheus text exposition format.

**OTLP:** Configure `[observability] otlp_metrics_enabled = true` and `otlp_export_interval_s` to push metrics to an OpenTelemetry Collector, Grafana Cloud, or any OTLP-compatible backend.

**`torvyn bench`:** Benchmark reports include all metrics as computed deltas over the benchmark window.

## Alerting Recommendations

| Condition | Metric | Threshold | Meaning |
|-----------|--------|-----------|---------|
| Error rate spike | `flow.elements.errors` rate | > 1% of total | Components are failing. Investigate error logs. |
| Sustained backpressure | `stream.backpressure.duration_ns` rate | > 50% of wall time | Consumer cannot keep up. Scale or optimize downstream. |
| Pool exhaustion | `pool.exhaustion_events` rate | > 0 sustained | Buffer pool is undersized. Increase pool configuration. |
| High copy amplification | `flow.copies.bytes` / `flow.throughput * avg_element_size` | > 3.0 per stage | More copies than expected. Investigate component data access patterns. |
| Memory growth | `component.memory_current` | Sustained increase | Possible memory leak in component. Investigate component logic. |
| Tail latency | `flow.latency` p99 | > 10× p50 | Occasional slow processing. Check for backpressure, GC, or I/O pauses. |
