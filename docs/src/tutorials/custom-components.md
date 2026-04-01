# Guide: Writing a Custom Torvyn Component

This guide explains how to write a Torvyn component from scratch — without templates. It covers WIT contract design, implementing the component in Rust, testing locally, and common patterns for error handling, configuration, and state management.

Use this guide when you need a component that does not fit any of the standard templates, or when you want to understand the full mechanics behind what the templates generate.

**Prerequisites:** Familiarity with Rust, basic understanding of WebAssembly concepts, and completion of at least the [Quickstart](../getting-started/quickstart.md).

**Time required:** 30–40 minutes (reading and building).

## Part 1: Designing the WIT Contract

Every Torvyn component begins with a WIT contract. The contract declares what the component exports (its role in the pipeline) and what it imports (the host capabilities it requires).

### Choosing a Component Role

Torvyn defines several standard interfaces. Choose the one that matches your component's role:

| Role | Interface | Behavior |
|------|-----------|----------|
| **Source** | `torvyn:streaming/source@0.1.0` | Produces elements. The runtime calls `pull()` repeatedly. |
| **Processor** | `torvyn:streaming/processor@0.1.0` | Transforms elements. Receives one input, emits one output or drops. |
| **Filter** | `torvyn:filtering/filter@0.1.0` | Accept/reject decisions. No buffer allocation needed. |
| **Sink** | `torvyn:streaming/sink@0.1.0` | Consumes elements. The runtime calls `push()` for each element. |
| **Router** | `torvyn:streaming/router@0.1.0` | Routes elements to named output ports. |
| **Aggregator** | Uses `torvyn:streaming/processor@0.1.0` | Accumulates state and emits periodically. Same interface as processor, different pattern. |

### Declaring Imports

Every component that reads data needs `torvyn:streaming/types@0.1.0`. Components that produce new buffers also need `torvyn:resources/buffer-ops@0.1.0` (which provides the `buffer-allocator` interface). Declare only what you need:

- **Sources and processors** typically import both `types` and `buffer-ops`.
- **Filters** typically import only `types` (they do not produce new buffers).
- **Sinks** typically import only `types`.

### Adding Lifecycle Hooks

If your component needs initialization (for example, to parse a configuration string or open a connection), you can also export `torvyn:streaming/lifecycle@0.1.0`:

```wit
package my-component:component;

world my-component {
    import torvyn:streaming/types@0.1.0;
    import torvyn:resources/buffer-ops@0.1.0;
    export torvyn:streaming/processor@0.1.0;
    export torvyn:streaming/lifecycle@0.1.0;
}
```

The `lifecycle` interface provides two functions:

- `init(config: string) -> result<_, process-error>` — called once after instantiation, before any stream processing begins. The `config` string is provided by the pipeline configuration and is typically JSON.
- `teardown()` — called once during shutdown. Release external resources here. The runtime calls this on a best-effort basis; it may not be called if the host shuts down forcefully or a timeout expires.

### Example: Rate Limiter Contract

Let us build a rate limiter component as our example. It tracks the rate of incoming elements and drops elements that exceed a configured threshold.

```wit
package rate-limiter:component;

world rate-limiter {
    import torvyn:streaming/types@0.1.0;
    export torvyn:filtering/filter@0.1.0;
    export torvyn:streaming/lifecycle@0.1.0;
}
```

This component is a filter (accept/reject, no buffer allocation) with lifecycle hooks for configuration.

## Part 2: Project Structure

Create the project directory:

```bash
mkdir -p rate-limiter/wit rate-limiter/src
```

### `rate-limiter/Cargo.toml`

```toml
[package]
name = "rate-limiter"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "rate-limiter:component"
```

### `rate-limiter/wit/world.wit`

```wit
package rate-limiter:component;

world rate-limiter {
    import torvyn:streaming/types@0.1.0;
    export torvyn:filtering/filter@0.1.0;
    export torvyn:streaming/lifecycle@0.1.0;
}
```

### `rate-limiter/Torvyn.toml`

```toml
[torvyn]
name = "rate-limiter"
version = "0.1.0"
contract_version = "0.1.0"

[[component]]
name = "rate-limiter"
path = "."
language = "rust"
```

## Part 3: Implementation

### `rate-limiter/src/lib.rs`

```rust
//! Rate limiter component.
//!
//! Tracks the rate of incoming elements using a sliding window
//! and drops elements that exceed the configured maximum rate.
//!
//! Configuration (JSON):
//! {
//!     "max_per_second": 100,
//!     "window_ms": 1000
//! }

wit_bindgen::generate!({
    world: "rate-limiter",
    path: "wit",
});

use exports::torvyn::filtering::filter::Guest as FilterGuest;
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::types::{StreamElement, ProcessError};

struct RateLimiter;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Configuration parsed from the init config string.
struct Config {
    max_per_second: u64,
    window_ms: u64,
}

static mut CONFIG: Option<Config> = None;
static mut WINDOW_COUNT: u64 = 0;
static mut WINDOW_START_NS: u64 = 0;
static mut TOTAL_PASSED: u64 = 0;
static mut TOTAL_DROPPED: u64 = 0;

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

impl LifecycleGuest for RateLimiter {
    fn init(config: String) -> Result<(), ProcessError> {
        // Parse the configuration string.
        // In a production component, use a proper JSON parser.
        let max_per_second = extract_u64(&config, "max_per_second").unwrap_or(100);
        let window_ms = extract_u64(&config, "window_ms").unwrap_or(1000);

        if max_per_second == 0 {
            return Err(ProcessError::InvalidInput(
                "max_per_second must be greater than 0".into(),
            ));
        }

        unsafe {
            CONFIG = Some(Config {
                max_per_second,
                window_ms,
            });
        }

        Ok(())
    }

    fn teardown() {
        unsafe {
            let passed = TOTAL_PASSED;
            let dropped = TOTAL_DROPPED;
            // In a production component, you might flush metrics here.
            // For now, we just reset state.
            CONFIG = None;
            WINDOW_COUNT = 0;
            WINDOW_START_NS = 0;
            let _ = (passed, dropped); // suppress unused warnings
        }
    }
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

impl FilterGuest for RateLimiter {
    fn evaluate(input: &StreamElement) -> Result<bool, ProcessError> {
        let config = unsafe {
            CONFIG.as_ref().ok_or_else(|| {
                ProcessError::Internal("rate limiter not initialized".into())
            })?
        };

        let now_ns = input.meta.timestamp_ns;
        let window_ns = config.window_ms * 1_000_000;

        unsafe {
            // Check if we are still within the current window.
            if now_ns.saturating_sub(WINDOW_START_NS) > window_ns {
                // Start a new window.
                WINDOW_START_NS = now_ns;
                WINDOW_COUNT = 0;
            }

            WINDOW_COUNT += 1;

            if WINDOW_COUNT <= config.max_per_second {
                TOTAL_PASSED += 1;
                Ok(true)
            } else {
                TOTAL_DROPPED += 1;
                Ok(false)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a u64 value from a simple JSON string.
/// This is a minimal parser for tutorial purposes.
/// Production components should use a proper JSON library.
fn extract_u64(json: &str, key: &str) -> Option<u64> {
    let pattern = format!(r#""{key}":"#);
    let start = json.find(&pattern)?;
    let value_start = start + pattern.len();
    let rest = &json[value_start..];

    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    rest[..end].parse().ok()
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

export!(RateLimiter);
```

### What This Demonstrates

**Configuration via lifecycle hooks.** The `init` function receives a JSON configuration string from the pipeline definition. This allows the same component binary to be configured differently in different pipelines — for example, one instance limiting to 100 elements/second and another to 1,000.

**Stateful filtering.** The rate limiter maintains a sliding window counter. State is stored in `static mut` variables because WebAssembly components are single-threaded (each instance runs in its own isolated sandbox). This is safe within the component's execution model.

**Error handling patterns.** The component uses the `ProcessError` variant types defined in the Torvyn contract:

- `ProcessError::InvalidInput` — the configuration string was malformed.
- `ProcessError::Internal` — an unexpected condition (component not initialized).

The runtime's error policy determines what happens when a component returns an error: it may retry, skip the element, or shut down the component, depending on the configured `ErrorPolicy`.

**Clean teardown.** The `teardown` function is called by the runtime during orderly shutdown. Components should release external resources (close connections, flush buffers) here. The runtime enforces a configurable timeout on teardown; if the component does not return in time, the runtime proceeds with forced termination.

## Part 4: Testing Locally

### Validate the Contract

```bash
cd rate-limiter
torvyn check
```

This validates the manifest and WIT contract without compiling.

### Build

```bash
cargo component build --target wasm32-wasip2
```

### Inspect the Component

After building, inspect the compiled component to verify its interface:

```bash
torvyn inspect target/wasm32-wasip2/debug/rate_limiter.wasm
```

This shows the component's exports, imports, and metadata — useful for verifying that the compiled binary matches your expected contract.

### Integration Test in a Pipeline

To test the rate limiter, wire it into a pipeline between a source and a sink. Add a flow configuration to `Torvyn.toml` that includes the rate limiter node with a configuration string:

```toml
[flow.test]
description = "Test rate limiter with 10 elements per second"

[flow.test.nodes.source]
component = "file://./path/to/source.wasm"
interface = "torvyn:streaming/source"

[flow.test.nodes.limiter]
component = "file://./target/wasm32-wasip2/debug/rate_limiter.wasm"
interface = "torvyn:filtering/filter"
config = '{"max_per_second": 10, "window_ms": 1000}'

[flow.test.nodes.sink]
component = "file://./path/to/sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.test.edges]]
from = { node = "source", port = "output" }
to = { node = "limiter", port = "input" }

[[flow.test.edges]]
from = { node = "limiter", port = "output" }
to = { node = "sink", port = "input" }
```

Run and observe the rate limiting:

```bash
torvyn run --flow test --limit 50
```

Trace to see which elements were dropped:

```bash
torvyn trace --flow test --limit 20
```

Benchmark to measure the overhead of the rate limiter:

```bash
torvyn bench --flow test --duration 5s
```

## Part 5: Common Patterns

### Pattern: Error Recovery

When a component encounters a non-fatal error, return the appropriate `ProcessError` variant. The runtime's error policy determines the response:

```rust
fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
    let data = input.payload.read_all();

    if data.is_empty() {
        // Non-fatal: input was empty. Signal invalid input.
        return Err(ProcessError::InvalidInput("empty payload".into()));
    }

    // ... process normally ...
}
```

The error categories have different runtime behaviors:

- `InvalidInput` — the element was malformed. The runtime may skip it or retry, depending on the configured error policy.
- `Unavailable` — a dependency is temporarily unreachable. The runtime may apply circuit-breaker logic.
- `Internal` — an unexpected error. The runtime may retry or skip.
- `DeadlineExceeded` — processing took too long. The runtime records a timeout.
- `Fatal` — the component cannot continue. The runtime tears down the component and will not send more elements.

### Pattern: Using Configuration

Pass structured configuration through the `lifecycle.init` function:

```rust
impl LifecycleGuest for MyComponent {
    fn init(config: String) -> Result<(), ProcessError> {
        if config.is_empty() {
            // Use defaults.
            return Ok(());
        }

        // Parse configuration (JSON recommended).
        // Store in static state for use during processing.
        Ok(())
    }
}
```

The configuration string is provided by the pipeline's `Torvyn.toml` via the `config` field on the node definition. JSON is the recommended format, but the string is opaque to the runtime — your component can parse it however it prefers.

### Pattern: Stateful Accumulation (Aggregator)

Aggregators use the processor interface but accumulate state across multiple elements:

```rust
static mut WINDOW: Vec<Vec<u8>> = Vec::new();
const WINDOW_SIZE: usize = 10;

fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
    let data = input.payload.read_all();

    unsafe {
        WINDOW.push(data);

        if WINDOW.len() >= WINDOW_SIZE {
            // Aggregate the window and emit a result.
            let aggregated = aggregate(&WINDOW);
            WINDOW.clear();

            // ... allocate buffer, write aggregated data, emit ...
        } else {
            // Accumulating — drop this element from output.
            Ok(ProcessResult::Drop)
        }
    }
}
```

Returning `ProcessResult::Drop` tells the runtime that the input was consumed without producing output. This is not an error — it means the element was absorbed into the component's internal state.

### Pattern: Minimal Buffer Reads

If your component only needs metadata to make a decision, avoid reading the buffer:

```rust
fn evaluate(input: &StreamElement) -> Result<bool, ProcessError> {
    // Decision based on metadata only — no buffer copy.
    Ok(input.meta.content_type == "application/json")
}
```

Each `read()` or `read_all()` call copies data from host memory into the component's linear memory. The resource manager records every copy. By checking metadata first and only reading the buffer when necessary, you minimize copies and improve throughput.

### Pattern: Content Type Routing

Use the `router` interface to send elements to different output ports based on content:

```rust
fn route(input: &StreamElement) -> String {
    match input.meta.content_type.as_str() {
        "application/json" => "json-sink".into(),
        "text/plain" => "text-sink".into(),
        _ => "default".into(),
    }
}
```

The returned string must match a port name defined in the pipeline topology.

## Summary

Writing a Torvyn component from scratch involves:

1. **Design the WIT contract** — choose the right role interface and declare only the imports you need.
2. **Create the project structure** — `Cargo.toml` with `crate-type = ["cdylib"]`, a `wit/` directory with the contract, and `src/lib.rs` with the implementation.
3. **Implement the interface** — use `wit_bindgen::generate!` to create bindings, then implement the required traits.
4. **Handle configuration** — use the `lifecycle` interface if you need initialization from a config string.
5. **Handle errors** — use the appropriate `ProcessError` variant for each failure mode.
6. **Test locally** — `torvyn check` for contract validation, `cargo component build` to compile, `torvyn run` and `torvyn trace` to verify behavior.

The key principle: **declare only what you need.** A filter that does not allocate buffers should not import `buffer-ops`. A sink that does not need initialization should not export `lifecycle`. This minimizes the component's capability surface and makes the contract self-documenting.
