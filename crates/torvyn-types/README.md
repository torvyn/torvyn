# torvyn-types

[![crates.io](https://img.shields.io/crates/v/torvyn-types.svg)](https://crates.io/crates/torvyn-types)
[![docs.rs](https://docs.rs/torvyn-types/badge.svg)](https://docs.rs/torvyn-types)
[![license](https://img.shields.io/crates/l/torvyn-types.svg)](https://github.com/torvyn/torvyn/blob/main/LICENSE)

Shared foundation types for the [Torvyn](https://github.com/torvyn/torvyn) reactive streaming runtime.

## Overview

`torvyn-types` is the universal leaf dependency in the Torvyn crate graph. Every other Torvyn crate depends on it, and it depends on nothing else within the workspace. It provides identity types, error types, domain enums, state machines, shared records, traits, and constants that form the common vocabulary across the entire runtime.

### Design Principles

- **Zero internal dependencies** — depends only on `std` and optionally `serde`.
- **Zero unsafe code** — `#![forbid(unsafe_code)]`.
- **Complete documentation** — `#![deny(missing_docs)]`.
- **Minimal compile time** — builds in under 2 seconds on a modern workstation.

## Position in the Architecture

`torvyn-types` sits at **Tier 1 (Foundation)** of the Torvyn crate hierarchy. It has no internal dependencies and is imported by every other crate in the workspace.

```
┌─────────────────────────────────────────────────┐
│  Tier 3: torvyn-reactor, torvyn-cli, torvyn     │
├─────────────────────────────────────────────────┤
│  Tier 2: config, contracts, engine, observability│
├─────────────────────────────────────────────────┤
│  Tier 1: torvyn-types  ◄── you are here         │
└─────────────────────────────────────────────────┘
```

## Modules

| Module | Contents |
|--------|----------|
| `identity` | Newtype wrappers: `ComponentTypeId`, `ComponentInstanceId`, `ComponentId`, `FlowId`, `StreamId`, `ResourceId`, `BufferHandle`, `TraceId`, `SpanId` |
| `error` | Structured error types: `TorvynError`, `ProcessError`, `ContractError`, `LinkError`, `ResourceError`, `ReactorError`, `ConfigError`, `SecurityError`, `PackagingError`, `EngineError` |
| `enums` | Domain enums: `ComponentRole`, `BackpressureSignal`, `BackpressurePolicy`, `ObservabilityLevel`, `Severity`, `CopyReason` |
| `state` | State machines with validated transitions: `FlowState`, `ResourceState`, `InvalidTransition` |
| `records` | Shared data records: `ElementMeta`, `TransferRecord`, `TraceContext` |
| `traits` | The `EventSink` trait (observability hot-path interface) and `NoopEventSink` |
| `constants` | Runtime-wide constants and limits |
| `timestamp` | `current_timestamp_ns()` for monotonic nanosecond timestamps |

## Usage

```rust
use torvyn_types::{
    FlowId, ComponentTypeId, ComponentRole,
    FlowState, BackpressureSignal,
    ProcessError, TorvynError,
    EventSink, NoopEventSink,
};

// Identity types are lightweight newtypes over strings or u64
let flow = FlowId::new("ingest-pipeline");
let component = ComponentTypeId::new("csv-parser");

// State machines enforce valid transitions at the type level
let state = FlowState::default();
assert_eq!(state, FlowState::Created);

// Error types carry structured context for diagnostics
let err = ProcessError::new(
    torvyn_types::ProcessErrorKind::InvalidInput,
    "malformed CSV row",
);
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `serde` | Yes | Enables `Serialize`/`Deserialize` derives on all public types |

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](https://github.com/torvyn/torvyn/blob/main/LICENSE) for details.

Part of the [Torvyn](https://github.com/torvyn/torvyn) project.
