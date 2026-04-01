# Testing Guide

This guide covers how to write and run tests for the Torvyn project. Torvyn uses a multi-layered testing strategy: unit tests for individual functions and types, integration tests for cross-module behavior, benchmark tests for performance verification, property tests for invariant validation, and fuzz tests for input boundary exploration.

## Test Infrastructure Overview

Tests live in two places:

- **Inline unit tests** in `#[cfg(test)]` modules at the bottom of source files. Use these for testing private functions and implementation details.
- **Integration tests** in `crates/<crate-name>/tests/`. Use these for testing the crate's public API as an external consumer would use it.

Some integration tests depend on pre-compiled Wasm test components (fixtures) located in `examples/test-components/`. Build these before running the full test suite:

```bash
cd examples/test-components
cargo component build --release
cd ../..
```

## How to Write Unit Tests

Unit tests for Torvyn crates follow the Arrange-Act-Assert pattern and the naming convention described in the Coding Standards.

### Testing Identity Types

Identity types in `torvyn-types` should be tested for: construction, equality, hashing, `Copy` semantics, display formatting, and debug formatting.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_id_equality() {
        let a = FlowId::new(42);
        let b = FlowId::new(42);
        let c = FlowId::new(43);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_flow_id_display() {
        let id = FlowId::new(7);
        assert_eq!(format!("{}", id), "flow-7");
    }
}
```

### Testing State Machines

The `FlowState` and `ResourceState` machines must be tested for every valid transition and every invalid transition. The existing tests in `torvyn-types/tests/state_machine_tests.rs` demonstrate the pattern:

```rust
#[test]
fn test_flow_state_full_happy_path() {
    let state = FlowState::Created;
    let state = state.transition_to(FlowState::Validated).unwrap();
    let state = state.transition_to(FlowState::Instantiated).unwrap();
    let state = state.transition_to(FlowState::Running).unwrap();
    let state = state.transition_to(FlowState::Draining).unwrap();
    let state = state.transition_to(FlowState::Completed).unwrap();
    assert!(state.is_terminal());
}

#[test]
fn test_flow_state_invalid_transition_returns_error() {
    let state = FlowState::Created;
    let result = state.transition_to(FlowState::Running);
    assert!(result.is_err());
}
```

### Testing Error Types

Every error variant should have a test verifying: construction, display output, and conversion to `TorvynError`.

```rust
#[test]
fn test_process_error_converts_to_torvyn_error() {
    let process_err = ProcessError::Fatal("disk full".into());
    let torvyn_err: TorvynError = process_err.into();
    let display = format!("{torvyn_err}");
    assert!(display.contains("FATAL"));
    assert!(display.contains("disk full"));
}
```

## How to Write Integration Tests

Integration tests verify cross-module behavior by using the crate's public API as an external consumer would. Place these in `crates/<crate-name>/tests/`.

For crates that depend on the Wasm engine (e.g., `torvyn-engine`, `torvyn-host`), integration tests may need compiled Wasm test components. These test fixtures live in `examples/test-components/` and are compiled as part of the test setup.

```rust
// crates/torvyn-engine/tests/compilation_test.rs
use torvyn_engine::{WasmtimeEngine, WasmtimeEngineConfig, WasmEngine};

#[test]
fn test_compile_valid_component_succeeds() {
    let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default()).unwrap();
    let wasm_bytes = std::fs::read("../../examples/test-components/target/wasm32-wasip2/release/passthrough.wasm").unwrap();
    let compiled = engine.compile(&wasm_bytes);
    assert!(compiled.is_ok());
}
```

## How to Write Benchmark Tests

Torvyn uses Criterion for benchmark tests. Benchmarks live in `benches/` directories within each crate.

```rust
// crates/torvyn-resources/benches/pool_benchmark.rs
use criterion::{criterion_group, criterion_main, Criterion};
use torvyn_resources::{BufferPoolSet, TierConfig};

fn bench_allocate_and_release(c: &mut Criterion) {
    let pool = BufferPoolSet::new(TierConfig::default());
    c.bench_function("allocate_small_buffer", |b| {
        b.iter(|| {
            let handle = pool.allocate(256).unwrap();
            pool.release(handle);
        })
    });
}

criterion_group!(benches, bench_allocate_and_release);
criterion_main!(benches);
```

Run benchmarks:

```bash
cargo bench -p torvyn-resources
```

When submitting a PR that modifies hot-path code, include benchmark results showing the before and after. Use `cargo bench -- --save-baseline before` and `cargo bench -- --baseline before` to generate comparison reports.

## How to Use Test Components

The `examples/test-components/` directory contains minimal Wasm components designed for testing. These implement Torvyn's WIT interfaces with simple, deterministic behavior:

- **passthrough** — A processor that copies input to output without modification. Useful for measuring baseline overhead.
- **counter-source** — A source that produces elements containing a monotonically increasing counter. Useful for testing flow lifecycle and ordering.
- **slow-sink** — A sink that introduces configurable artificial delay. Useful for testing backpressure behavior.
- **failing-processor** — A processor that returns `ProcessError::Fatal` after a configurable number of elements. Useful for testing error handling.

Build all test components:

```bash
cd examples/test-components
cargo component build --release
```

## Running Specific Test Subsets

```bash
# All tests in a specific crate
cargo test -p torvyn-reactor

# Tests matching a name pattern
cargo test -p torvyn-types flow_state

# Only integration tests (not unit tests)
cargo test -p torvyn-engine --test '*'

# Only unit tests (not integration tests)
cargo test -p torvyn-types --lib

# Only doc tests
cargo test -p torvyn-types --doc
```

## Measuring Code Coverage

Use `cargo-llvm-cov` for coverage measurement:

```bash
cargo install cargo-llvm-cov

# Generate coverage for the workspace
cargo llvm-cov --workspace --html

# Open the report
open target/llvm-cov/html/index.html

# Generate coverage for a single crate
cargo llvm-cov -p torvyn-types --html
```

There is no hard coverage target. Focus coverage on: all public API functions, all state machine transitions, all error paths, and all hot-path code.

## Property Testing with proptest

Torvyn uses `proptest` for property-based testing, particularly for identity types, state machines, and serialization round-trips.

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_resource_id_roundtrip(index in 0u32..u32::MAX, gen in 0u32..u32::MAX) {
        let id = ResourceId::new(index, gen);
        assert_eq!(id.index(), index);
        assert_eq!(id.generation(), gen);
    }

    #[test]
    fn test_flow_state_terminal_states_are_idempotent(
        terminal in prop_oneof![
            Just(FlowState::Completed),
            Just(FlowState::Failed),
        ]
    ) {
        // No transition from a terminal state should succeed
        for target in FlowState::all_variants() {
            if target != terminal {
                assert!(terminal.transition_to(target).is_err());
            }
        }
    }
}
```

Add `proptest` as a dev dependency in the crate's `Cargo.toml`:

```toml
[dev-dependencies]
proptest = "1"
```

## Fuzz Testing Strategy

Fuzz testing is planned for security-sensitive input parsing boundaries: WIT file parsing (`torvyn-contracts`), TOML configuration parsing (`torvyn-config`), and OCI artifact deserialization (`torvyn-packaging`).

Fuzz targets use `cargo-fuzz` with `libFuzzer`:

```bash
cargo install cargo-fuzz

# List available fuzz targets
cargo fuzz list

# Run a specific fuzz target for 60 seconds
cargo fuzz run wit_parser_fuzz -- -max_total_time=60
```

Fuzz targets live in `fuzz/` directories within the relevant crates. When writing a new fuzz target, focus on functions that accept arbitrary byte slices or string input from untrusted sources.
