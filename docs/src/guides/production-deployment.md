# Production Deployment Guide

This guide covers deploying Torvyn pipelines in production environments. It assumes familiarity with Torvyn's concepts (contracts, components, flows, capabilities) and prior experience running pipelines locally with `torvyn run`.

## Resource Sizing and Capacity Planning

### Memory Budget

Torvyn's memory consumption has three components:

1. **Host overhead:** Resource tables, stream queues, metric structures, trace buffers, and runtime bookkeeping. Typically 10–50 MiB depending on the number of active flows and components.

2. **Buffer pool reservation:** Pre-allocated buffer pools. With default settings, the pool reservation is approximately 53 MiB (Small: ~1 MiB, Medium: ~4 MiB, Large: ~16 MiB, Huge: ~32 MiB on-demand). Adjust pool sizes based on workload: if your pipeline processes mostly small messages, reduce Large and Huge pools; if it processes large binary payloads, increase Large pool allocation.

3. **Component linear memory:** Each Wasm component instance has its own linear memory, starting small and growing up to the configured limit (`max_memory_per_component`, default 64 MiB). Total component memory = number of active component instances × their actual memory usage. For a pipeline with 5 components at 20 MiB each, budget 100 MiB.

**Formula:** `total_memory ≈ host_overhead + pool_reservation + (active_components × avg_component_memory)`

### CPU Budget

Torvyn uses Tokio's multi-threaded runtime. Set `worker_threads` to the number of CPU cores available to the process. Each flow driver consumes one Tokio task; Tokio distributes tasks across worker threads. For workloads with many concurrent flows, ensure `worker_threads` ≥ number of flows that must make progress simultaneously.

Wasmtime's fuel mechanism provides CPU time limiting per component invocation. The default `fuel_per_invocation` of 1,000,000 fuel units is approximately 1–10 ms of CPU time depending on instruction mix. Adjust based on component complexity.

### Queue Depth Sizing

The default queue depth of 64 elements per stream is suitable for most workloads. Increase it for bursty sources (to absorb bursts without triggering backpressure) or decrease it for latency-sensitive pipelines (smaller queues reduce maximum queuing delay).

Total queue memory = `Σ(queue_depth × max_element_handle_size)` per stream. Since queues hold handle references (not payload bytes), queue memory is small relative to buffer pool memory.

## Monitoring and Alerting Setup

### Prometheus Integration

Enable the Prometheus metrics endpoint:

```toml
[observability]
metrics_enabled = true
prometheus_enabled = true
```

The `/metrics` endpoint is served on the inspection API (default: Unix domain socket at `$TORVYN_STATE_DIR/torvyn.sock`, or localhost TCP if configured). Configure your Prometheus scrape target accordingly.

### Trace Export

For production trace export, configure OTLP gRPC:

```toml
[observability.tracing]
level = "production"
tracing_exporter = "otlp-grpc"
tracing_endpoint = "http://your-otel-collector:4317"
sample_rate = 0.01
error_promote = true
latency_promote_threshold_ms = 10
```

### Recommended Grafana Dashboards

Build dashboards around these key metrics: `flow.elements.total` rate (throughput), `flow.latency` percentiles, `stream.backpressure.duration_ns` (health), `pool.available` per tier (capacity), `component.processing_time` per component (bottleneck identification), and `system.memory.total` (resource utilization).

## Security Hardening Checklist

- [ ] Set `default_capability_policy = "deny-all"` in `[security]`.
- [ ] Grant each component the minimum required capabilities. Review grants against component manifests.
- [ ] Enable audit logging: `audit_enabled = true`.
- [ ] Bind the inspection API to localhost only (the default). Do not expose it to the network without authentication.
- [ ] If exposing the inspection API over TCP, configure token-based authentication.
- [ ] Verify component artifacts are signed before deploying (`torvyn inspect --show capabilities` on every artifact).
- [ ] Set appropriate `max_memory_per_component` limits to prevent memory exhaustion by any single component.
- [ ] Set appropriate `fuel_per_invocation` limits to prevent CPU monopolization.
- [ ] Review and pin component versions in pipeline definitions. Do not use mutable tags (like `latest`) for production components.

## Operational Runbook

### Pipeline is not processing elements

1. Check flow state: `GET /flows` on the inspection API. Look for flows in `Failed` or `Paused` state.
2. Check for component errors: review `component.errors` metrics. A `fatal` error stops the flow.
3. Check for backpressure: if `stream.backpressure.duration_ns` is high, the pipeline may be stalled on a slow sink.
4. Check resource availability: if `pool.exhaustion_events` is increasing, buffer pools may be exhausted.

### Latency is increasing

1. Check per-component latency: `component.processing_time` histograms identify the slow stage.
2. Check queue depths: `stream.queue.current_depth` shows where data is accumulating.
3. Check backpressure: sustained backpressure on a specific stream indicates the downstream component is the bottleneck.
4. Check system resources: CPU saturation or memory pressure can increase latency across all components.

### Memory usage is growing

1. Check `component.memory_current` for each component. A steadily increasing gauge suggests a memory leak in component code.
2. Check `pool.available` per tier. Decreasing availability without corresponding increase in throughput suggests buffers are not being returned to the pool.
3. Check for stale flows: flows that have stopped processing but have not been cleaned up may hold resource references.

## Upgrade Procedures

1. **Test the upgrade locally.** Run the new version with your pipeline using `torvyn run` and `torvyn bench`. Compare benchmark results against your baseline.
2. **Verify contract compatibility.** If the new version includes contract changes, run `torvyn link` against all your component artifacts.
3. **Rolling upgrade (multi-instance).** If running multiple Torvyn instances behind a load balancer, upgrade instances one at a time. Drain active flows on an instance before upgrading.
4. **In-place upgrade (single instance).** Stop the running pipeline (`torvyn run` responds to SIGTERM with graceful shutdown — flows drain, components teardown, resources are reclaimed). Replace the binary. Restart.
