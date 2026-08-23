# Configuration Reference

Torvyn uses TOML for all configuration, consistent with the Rust ecosystem convention established by `Cargo.toml`. Configuration lives in `Torvyn.toml` at the project root.

## Configuration Merging Rules

Configuration values are resolved through a layered precedence model (highest precedence first):

1. **CLI flags** — `--config key=value` on `torvyn run` and other commands.
2. **Environment variables** — `TORVYN_` prefix + uppercase path (e.g., `TORVYN_RUNTIME_WORKER_THREADS`).
3. **Project manifest** — `Torvyn.toml` in the project root.
4. **Global user config** — `~/.config/torvyn/config.toml`.
5. **Built-in defaults** — Compiled into the binary.

## Component Manifest (`Torvyn.toml` — per component)

### `[torvyn]` — Project Metadata

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | Yes | — | Project name. Must be a valid identifier (lowercase, hyphens allowed). |
| `version` | string | Yes | — | Project version (semantic versioning, e.g., `"0.1.0"`). |
| `contract_version` | string | Yes | — | Torvyn contract version this project targets (e.g., `"0.1.0"`). |
| `description` | string | No | `""` | Human-readable project description. |
| `authors` | list of strings | No | `[]` | Author names and emails (e.g., `["Alice <alice@example.com>"]`). |
| `license` | string | No | `""` | SPDX license identifier (e.g., `"Apache-2.0"`). |
| `repository` | string | No | `""` | Source repository URL. |

### `[capabilities.required]` — Required Capabilities

Key-value pairs where keys are capability identifiers and values are booleans. Components will not link if required capabilities are not granted.

```toml
[capabilities.required]
wasi-filesystem-read = true
wasi-clocks = true
```

### `[capabilities.optional]` — Optional Capabilities

Capabilities that enhance functionality but are not required. The component must handle their absence gracefully.

### `[capabilities.torvyn]` — Torvyn-Specific Resource Requirements

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max-buffer-size` | string | `"16MiB"` | Maximum single buffer size this component needs. |
| `max-memory` | string | `"64MiB"` | Maximum Wasm linear memory this component needs. |
| `buffer-pool-access` | string | `"default"` | Named buffer pool to use. |

## Pipeline Manifest (`Torvyn.toml` — pipeline project)

### `[[component]]` — Component Declarations

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | Yes | — | Component name within this project. |
| `path` | string | Yes | — | Path to component source root (relative to project root). |
| `language` | string | No | `"rust"` | Implementation language. Values: `rust`, `go`, `python`, `zig`. |
| `build_command` | string | No | auto-detected | Custom build command override. |

### `[flow.<NAME>]` — Flow Definition

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `description` | string | No | `""` | Human-readable flow description. |

### `[flow.<NAME>.nodes.<NODE>]` — Component Nodes

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `component` | string | Yes | — | Path to compiled `.wasm` file or registry reference. |
| `interface` | string | Yes | — | Torvyn interface this component implements (e.g., `torvyn:streaming/processor`). |
| `config` | string | No | `""` | Configuration string passed to `lifecycle.init()`. JSON recommended. |

### `[[flow.<NAME>.edges]]` — Stream Connections

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `from.node` | string | Yes | Upstream node name. |
| `from.port` | string | Yes | Output port name (usually `"output"`). |
| `to.node` | string | Yes | Downstream node name. |
| `to.port` | string | Yes | Input port name (usually `"input"`). |

### `[runtime]` — Runtime Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `worker_threads` | integer | CPU count | Tokio worker thread count. |
| `max_memory_per_component` | string | `"16MiB"` | Wasm linear memory limit per component instance. A component that cannot reach its declared minimum under this cap is refused at instantiation. |
| `default_fuel_per_invocation` | integer | `1_000_000` | Wasmtime fuel budget per component call. `0` disables fuel metering entirely. |
| `component_init_timeout_ms` | integer | `5000` | Timeout for `lifecycle.init()` calls. |
| `component_teardown_timeout_ms` | integer | `2000` | Timeout for `lifecycle.teardown()` calls. |

Both apply to every component in the pipeline. A flow node overrides them for itself:

```toml
[flow.main.nodes.transform]
component = "transform"
interface = "torvyn:streaming/processor"
fuel_budget = 5_000_000     # this component alone gets more fuel; 0 = unlimited
max_memory = "64MiB"        # and a larger memory cap
```

A component that exhausts its fuel traps, which the runtime reports as `deadline-exceeded` — fuel is a proxy for CPU time, so that is the category it falls under. A component that cannot reach its declared minimum memory under its cap is refused at instantiation, and the error names both the cap and the size that was attempted.

### `[runtime.backpressure]` — Global Backpressure Defaults

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_queue_depth` | integer | `64` | Default bounded queue capacity per stream. |
| `backpressure_policy` | string | `"block"` | Default policy. Values: `block`, `drop-oldest`, `drop-newest`, `error`. |
| `low_watermark_ratio` | float | `0.5` | Queue depth ratio at which backpressure deactivates. |

### `[runtime.pools]` — Buffer Pool Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `small_pool_size` | integer | `4096` | Number of 256-byte buffers pre-allocated. |
| `medium_pool_size` | integer | `1024` | Number of 4 KiB buffers pre-allocated. |
| `large_pool_size` | integer | `256` | Number of 64 KiB buffers pre-allocated. |
| `huge_pool_size` | integer | `32` | Maximum cached 1 MiB buffers (on-demand). |
| `exhaustion_policy` | string | `"fallback-alloc"` | Policy when pool is empty. Values: `fallback-alloc`, `error`. |

### `[observability]` — Observability Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `level` | string | `"production"` | Observability level. Values: `off`, `production`, `diagnostic`. |
| `tracing_enabled` | boolean | `true` | Enable trace collection. |
| `tracing_exporter` | string | `"stdout"` | Trace export target. Values: `otlp-grpc`, `otlp-http`, `stdout`, `file`, `none`. |
| `tracing_endpoint` | string | `""` | OTLP endpoint URL (for `otlp-grpc` and `otlp-http`). |
| `sample_rate` | float | `0.01` | Head-based trace sampling rate (0.0–1.0). |
| `error_promote` | boolean | `true` | Promote errored flows to full tracing. |
| `latency_promote_threshold_ms` | integer | `10` | Promote flows exceeding this latency. |
| `metrics_enabled` | boolean | `true` | Enable metrics collection. |
| `prometheus_enabled` | boolean | `true` | Serve `/metrics` on inspection API. |
| `otlp_metrics_enabled` | boolean | `false` | Push metrics via OTLP. |
| `otlp_metrics_interval_s` | integer | `15` | OTLP metrics push interval. |
| `ring_buffer_capacity` | integer | `64` | Per-flow span ring buffer size for retroactive sampling. |

### `[security]` — Security Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_capability_policy` | string | `"deny-all"` | Default policy for ungranted capabilities. |
| `audit_enabled` | boolean | `true` | Enable security audit logging. |
| `audit_target` | string | `"file"` | Audit log target. Values: `file`, `stdout`, `event-sink`. |

### `[security.grants.<COMPONENT>]` — Per-Component Capability Grants

```toml
[security.grants.my-transform]
capabilities = [
    "wasi:filesystem/read:/data/input",
    "wasi:clocks/wall-clock",
]
```

### `[registry]` — Registry Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default` | string | `""` | Default OCI registry URL for `torvyn publish`. |
