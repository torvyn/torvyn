//! Embedded template file contents.
//!
//! Each template function returns a [`Template`] with all files
//! needed for that project pattern.

use super::{Template, TemplateFile};
use std::path::PathBuf;

fn tf(path: &str, content: &str) -> TemplateFile {
    TemplateFile {
        relative_path: PathBuf::from(path),
        content: content.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Transform template (default)
// ---------------------------------------------------------------------------

/// The `transform` template: a stateless data transformer.
pub fn transform_template() -> Template {
    Template {
        description: "Stateless data transformer".into(),
        files: vec![
            tf("Torvyn.toml", TRANSFORM_TORVYN_TOML),
            tf("Cargo.toml", TRANSFORM_CARGO_TOML),
            tf("wit/torvyn-streaming/types.wit", TORVYN_STREAMING_TYPES_WIT),
            tf(
                "wit/torvyn-streaming/processor.wit",
                TORVYN_STREAMING_PROCESSOR_WIT,
            ),
            tf(
                "wit/torvyn-streaming/buffer-allocator.wit",
                TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT,
            ),
            tf(
                "wit/torvyn-streaming/lifecycle.wit",
                TORVYN_STREAMING_LIFECYCLE_WIT,
            ),
            tf(
                "wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_TRANSFORM_WORLD_WIT,
            ),
            tf("src/lib.rs", TRANSFORM_LIB_RS),
            tf(".gitignore", COMMON_GITIGNORE),
            tf("README.md", TRANSFORM_README),
        ],
    }
}

const TRANSFORM_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"

[[component]]
name = "{{project_name}}"
path = "."
language = "rust"
"#;

const TRANSFORM_CARGO_TOML: &str = r#"[package]
name = "{{project_name}}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "{{project_name}}:component"
"#;

// ---------------------------------------------------------------------------
// Shared Torvyn streaming WIT definitions (bundled with templates)
// ---------------------------------------------------------------------------

const TORVYN_STREAMING_TYPES_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface types {
    resource buffer {
        size: func() -> u64;
        content-type: func() -> string;
        read: func(offset: u64, len: u64) -> list<u8>;
        read-all: func() -> list<u8>;
    }

    resource mutable-buffer {
        write: func(offset: u64, bytes: list<u8>) -> result<_, buffer-error>;
        append: func(bytes: list<u8>) -> result<_, buffer-error>;
        size: func() -> u64;
        capacity: func() -> u64;
        set-content-type: func(content-type: string);
        freeze: func() -> buffer;
    }

    variant buffer-error {
        capacity-exceeded,
        out-of-bounds,
        allocation-failed(string),
    }

    resource flow-context {
        trace-id: func() -> string;
        span-id: func() -> string;
        deadline-ns: func() -> u64;
        flow-id: func() -> string;
    }

    record element-meta {
        sequence: u64,
        timestamp-ns: u64,
        content-type: string,
    }

    record stream-element {
        meta: element-meta,
        payload: borrow<buffer>,
        context: borrow<flow-context>,
    }

    record output-element {
        meta: element-meta,
        payload: buffer,
    }

    variant process-result {
        emit(output-element),
        drop,
    }

    variant process-error {
        invalid-input(string),
        unavailable(string),
        internal(string),
        deadline-exceeded,
        fatal(string),
    }

    enum backpressure-signal {
        ready,
        pause,
    }
}
"#;

const TORVYN_STREAMING_PROCESSOR_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface processor {
    use types.{stream-element, process-result, process-error};

    process: func(input: stream-element) -> result<process-result, process-error>;
}
"#;

const TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface buffer-allocator {
    use types.{mutable-buffer, buffer-error, buffer};

    allocate: func(capacity-hint: u64) -> result<mutable-buffer, buffer-error>;
    clone-into-mutable: func(source: borrow<buffer>) -> result<mutable-buffer, buffer-error>;
}
"#;

const TORVYN_STREAMING_LIFECYCLE_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface lifecycle {
    use types.{process-error};

    init: func(config: string) -> result<_, process-error>;
    teardown: func();
}
"#;

const TORVYN_STREAMING_SOURCE_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface source {
    use types.{output-element, process-error, backpressure-signal};

    pull: func() -> result<option<output-element>, process-error>;
    notify-backpressure: func(signal: backpressure-signal);
}
"#;

const TORVYN_STREAMING_SINK_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface sink {
    use types.{stream-element, process-error, backpressure-signal};

    push: func(element: stream-element) -> result<backpressure-signal, process-error>;
    complete: func() -> result<_, process-error>;
}
"#;

const TORVYN_STREAMING_TRANSFORM_WORLD_WIT: &str = r#"package torvyn:streaming@0.1.0;

world transform {
    import types;
    import buffer-allocator;

    export processor;
}
"#;

const TRANSFORM_LIB_RS: &str = r#"// Generated by `torvyn init --template transform` on {{date}}
// Torvyn CLI v{{torvyn_version}}
//
// This component implements the torvyn:streaming/processor interface.
// It receives stream elements, transforms them, and produces output elements.

wit_bindgen::generate!({
    world: "transform",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::processor::{Guest, ProcessResult};
use torvyn::streaming::types::{StreamElement, ProcessError};

struct {{component_type}};

impl Guest for {{component_type}} {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        // TODO: Implement your transform logic here.
        //
        // `input` contains:
        //   - input.meta: element metadata (trace ID, content type, timestamp)
        //   - input.buffer: a handle to the payload buffer
        //
        // Pass-through: emit the input unchanged
        Ok(ProcessResult::Emit(input))
    }
}

export!({{component_type}});
"#;

const TRANSFORM_README: &str = r#"# {{project_name}}

A Torvyn streaming transform component.

## Quick Start

```bash
torvyn check       # Validate contracts and manifest
torvyn build       # Compile to WebAssembly component
torvyn run         # Execute the pipeline locally
```

## Project Structure

- `Torvyn.toml` — Project manifest
- `wit/torvyn-streaming/` — Torvyn streaming WIT contracts
- `src/lib.rs` — Component implementation

## Learn More

- [Torvyn Documentation](https://docs.torvyn.dev)
- [WIT Contract Guide](https://docs.torvyn.dev/guides/wit-primer)
"#;

// ---------------------------------------------------------------------------
// Source template
// ---------------------------------------------------------------------------

/// The `source` template: a data producer.
pub fn source_template() -> Template {
    Template {
        description: "Data producer (no input, one output)".into(),
        files: vec![
            tf("Torvyn.toml", SOURCE_TORVYN_TOML),
            tf("Cargo.toml", SOURCE_CARGO_TOML),
            tf("wit/torvyn-streaming/types.wit", TORVYN_STREAMING_TYPES_WIT),
            tf(
                "wit/torvyn-streaming/source.wit",
                TORVYN_STREAMING_SOURCE_WIT,
            ),
            tf(
                "wit/torvyn-streaming/buffer-allocator.wit",
                TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT,
            ),
            tf(
                "wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_SOURCE_WORLD_WIT,
            ),
            tf("src/lib.rs", SOURCE_LIB_RS),
            tf(".gitignore", COMMON_GITIGNORE),
            tf("README.md", SOURCE_README),
        ],
    }
}

const SOURCE_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"

[[component]]
name = "{{project_name}}"
path = "."
language = "rust"
"#;

const SOURCE_CARGO_TOML: &str = r#"[package]
name = "{{project_name}}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "{{project_name}}:component"
"#;

const TORVYN_STREAMING_SOURCE_WORLD_WIT: &str = r#"package torvyn:streaming@0.1.0;

world data-source {
    import types;
    import buffer-allocator;

    export source;
}
"#;

const SOURCE_LIB_RS: &str = r#"// Generated by `torvyn init --template source` on {{date}}
// Torvyn CLI v{{torvyn_version}}
//
// This component implements the torvyn:streaming/source interface.
// It generates stream elements for downstream processing.

wit_bindgen::generate!({
    world: "data-source",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::source::Guest;
use torvyn::streaming::types::{OutputElement, ElementMeta, ProcessError, BackpressureSignal};
use torvyn::streaming::buffer_allocator;

struct {{component_type}};

static mut COUNTER: u64 = 0;

impl Guest for {{component_type}} {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        // TODO: Replace with your data generation logic.
        //
        // Return `Ok(None)` to signal end of stream.
        // Return `Ok(Some(element))` to produce an element.

        let count = unsafe {
            COUNTER += 1;
            COUNTER
        };

        if count > 1000 {
            return Ok(None); // End of stream after 1000 elements
        }

        let message = format!("Hello, Torvyn! ({count})");
        let buf = buffer_allocator::allocate(message.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(message.as_bytes())
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;

        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: count,
                timestamp_ns: 0,
                content_type: "text/plain".to_string(),
            },
            payload: buf.freeze(),
        }))
    }

    fn notify_backpressure(_signal: BackpressureSignal) {
        // TODO: Handle backpressure signals from downstream.
    }
}

export!({{component_type}});
"#;

const SOURCE_README: &str = r#"# {{project_name}}

A Torvyn streaming source component.

## Quick Start

```bash
torvyn check       # Validate contracts
torvyn build       # Compile to WebAssembly
```
"#;

// ---------------------------------------------------------------------------
// Sink template
// ---------------------------------------------------------------------------

/// The `sink` template: a data consumer.
pub fn sink_template() -> Template {
    Template {
        description: "Data consumer (one input, no output)".into(),
        files: vec![
            tf("Torvyn.toml", SINK_TORVYN_TOML),
            tf("Cargo.toml", SINK_CARGO_TOML),
            tf("wit/torvyn-streaming/types.wit", TORVYN_STREAMING_TYPES_WIT),
            tf("wit/torvyn-streaming/sink.wit", TORVYN_STREAMING_SINK_WIT),
            tf(
                "wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_SINK_WORLD_WIT,
            ),
            tf("src/lib.rs", SINK_LIB_RS),
            tf(".gitignore", COMMON_GITIGNORE),
            tf("README.md", SINK_README),
        ],
    }
}

const SINK_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"

[[component]]
name = "{{project_name}}"
path = "."
language = "rust"
"#;

const SINK_CARGO_TOML: &str = r#"[package]
name = "{{project_name}}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "{{project_name}}:component"
"#;

const TORVYN_STREAMING_SINK_WORLD_WIT: &str = r#"package torvyn:streaming@0.1.0;

world data-sink {
    import types;

    export sink;
}
"#;

const SINK_LIB_RS: &str = r#"// Generated by `torvyn init --template sink` on {{date}}
// Torvyn CLI v{{torvyn_version}}
//
// This component implements the torvyn:streaming/sink interface.
// It receives stream elements and consumes them (e.g., writes to stdout).

wit_bindgen::generate!({
    world: "data-sink",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::sink::Guest;
use torvyn::streaming::types::{StreamElement, ProcessError, BackpressureSignal};

struct {{component_type}};

impl Guest for {{component_type}} {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        // TODO: Implement your sink logic here.
        let data = element.payload.read_all();
        let text = String::from_utf8_lossy(&data);
        println!("{text}");
        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        // Called when the stream ends.
        Ok(())
    }
}

export!({{component_type}});
"#;

const SINK_README: &str = r#"# {{project_name}}

A Torvyn streaming sink component.

## Quick Start

```bash
torvyn check
torvyn build
```
"#;

// ---------------------------------------------------------------------------
// Filter template
// ---------------------------------------------------------------------------

/// The `filter` template: a content filter/guard.
pub fn filter_template() -> Template {
    Template {
        description: "Content filter/guard".into(),
        files: vec![
            tf("Torvyn.toml", FILTER_TORVYN_TOML),
            tf("Cargo.toml", FILTER_CARGO_TOML),
            tf("wit/torvyn-streaming/types.wit", TORVYN_STREAMING_TYPES_WIT),
            tf(
                "wit/torvyn-streaming/filter.wit",
                TORVYN_STREAMING_FILTER_WIT,
            ),
            tf(
                "wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_FILTER_WORLD_WIT,
            ),
            tf("src/lib.rs", FILTER_LIB_RS),
            tf(".gitignore", COMMON_GITIGNORE),
            tf("README.md", FILTER_README),
        ],
    }
}

const FILTER_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"

[[component]]
name = "{{project_name}}"
path = "."
language = "rust"
"#;

const FILTER_CARGO_TOML: &str = r#"[package]
name = "{{project_name}}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "{{project_name}}:component"
"#;

const TORVYN_STREAMING_FILTER_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface filter {
    use types.{stream-element, process-error};

    /// Evaluate whether a stream element should pass through.
    ///
    /// - ok(true): Element passes. Runtime forwards it.
    /// - ok(false): Element rejected. Runtime drops it.
    /// - err(error): Filter encountered an error.
    evaluate: func(element: stream-element) -> result<bool, process-error>;
}
"#;

const TORVYN_STREAMING_FILTER_WORLD_WIT: &str = r#"package torvyn:streaming@0.1.0;

world content-filter {
    import types;

    export filter;
}
"#;

const FILTER_LIB_RS: &str = r#"// Generated by `torvyn init --template filter` on {{date}}
// Torvyn CLI v{{torvyn_version}}
//
// This component implements the torvyn:streaming/filter interface.
// It evaluates each element and decides whether to pass or drop it.

wit_bindgen::generate!({
    world: "content-filter",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::filter::Guest;
use torvyn::streaming::types::{StreamElement, ProcessError};

struct {{component_type}};

impl Guest for {{component_type}} {
    fn evaluate(element: StreamElement) -> Result<bool, ProcessError> {
        // TODO: Implement your filter logic here.
        //
        // Return `Ok(true)` to pass the element through.
        // Return `Ok(false)` to drop it.
        //
        // Access payload bytes: element.payload.read_all()
        // Access metadata: element.meta.content_type

        // Default: pass everything
        Ok(true)
    }
}

export!({{component_type}});
"#;

const FILTER_README: &str = r#"# {{project_name}}

A Torvyn streaming filter component.

## Quick Start

```bash
torvyn check       # Validate contracts
torvyn build       # Compile to WebAssembly
```

## Project Structure

- `Torvyn.toml` — Project manifest
- `wit/torvyn-streaming/` — Torvyn streaming WIT contracts
- `src/lib.rs` — Component implementation
"#;

// ---------------------------------------------------------------------------
// Router template
// ---------------------------------------------------------------------------

/// The `router` template: multi-output router.
pub fn router_template() -> Template {
    Template {
        description: "Multi-output router".into(),
        files: vec![
            tf("Torvyn.toml", TRANSFORM_TORVYN_TOML),
            tf("Cargo.toml", TRANSFORM_CARGO_TOML),
            tf("wit/torvyn-streaming/types.wit", TORVYN_STREAMING_TYPES_WIT),
            tf(
                "wit/torvyn-streaming/router.wit",
                TORVYN_STREAMING_ROUTER_WIT,
            ),
            tf(
                "wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_ROUTER_WORLD_WIT,
            ),
            tf("src/lib.rs", ROUTER_LIB_RS),
            tf(".gitignore", COMMON_GITIGNORE),
            tf("README.md", ROUTER_README),
        ],
    }
}

const TORVYN_STREAMING_ROUTER_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface router {
    use types.{stream-element, process-error};

    /// Determine which output port(s) should receive this element.
    ///
    /// Returns a list of port names. Empty list means drop.
    /// Multiple names means fan-out.
    route: func(element: stream-element) -> result<list<string>, process-error>;
}
"#;

const TORVYN_STREAMING_ROUTER_WORLD_WIT: &str = r#"package torvyn:streaming@0.1.0;

world content-router {
    import types;

    export router;
}
"#;

const ROUTER_LIB_RS: &str = r#"// Generated by `torvyn init --template router` on {{date}}
// Torvyn CLI v{{torvyn_version}}
//
// This component implements the torvyn:streaming/router interface.
// It routes each element to one of multiple output ports.

wit_bindgen::generate!({
    world: "content-router",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::router::Guest;
use torvyn::streaming::types::{StreamElement, ProcessError};

struct {{component_type}};

impl Guest for {{component_type}} {
    fn route(element: StreamElement) -> Result<Vec<String>, ProcessError> {
        // TODO: Return the port names to route the element to.
        //
        // Return an empty list to drop the element.
        // Return multiple names for fan-out (runtime borrows the
        // same buffer to each downstream).

        // Default: route everything to "default"
        Ok(vec!["default".to_string()])
    }
}

export!({{component_type}});
"#;

const ROUTER_README: &str = r#"# {{project_name}}

A Torvyn streaming router component.

## Quick Start

```bash
torvyn check       # Validate contracts
torvyn build       # Compile to WebAssembly
```

## Project Structure

- `Torvyn.toml` — Project manifest
- `wit/torvyn-streaming/` — Torvyn streaming WIT contracts
- `src/lib.rs` — Component implementation
"#;

// ---------------------------------------------------------------------------
// Aggregator template
// ---------------------------------------------------------------------------

/// The `aggregator` template: stateful windowed aggregator.
pub fn aggregator_template() -> Template {
    Template {
        description: "Stateful windowed aggregator".into(),
        files: vec![
            tf("Torvyn.toml", TRANSFORM_TORVYN_TOML),
            tf("Cargo.toml", TRANSFORM_CARGO_TOML),
            tf("wit/torvyn-streaming/types.wit", TORVYN_STREAMING_TYPES_WIT),
            tf(
                "wit/torvyn-streaming/buffer-allocator.wit",
                TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT,
            ),
            tf(
                "wit/torvyn-streaming/aggregator.wit",
                TORVYN_STREAMING_AGGREGATOR_WIT,
            ),
            tf(
                "wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_AGGREGATOR_WORLD_WIT,
            ),
            tf("src/lib.rs", AGGREGATOR_LIB_RS),
            tf(".gitignore", COMMON_GITIGNORE),
            tf("README.md", AGGREGATOR_README),
        ],
    }
}

const TORVYN_STREAMING_AGGREGATOR_WIT: &str = r#"package torvyn:streaming@0.1.0;

interface aggregator {
    use types.{stream-element, output-element, process-error};

    /// Ingest a stream element into internal state.
    ///
    /// - ok(none): Absorbed, no output yet.
    /// - ok(some(element)): Absorbed AND aggregated result ready.
    /// - err(error): Ingestion failed.
    ingest: func(element: stream-element) -> result<option<output-element>, process-error>;

    /// Signal no more elements. Emit remaining buffered results.
    flush: func() -> result<list<output-element>, process-error>;
}
"#;

const TORVYN_STREAMING_AGGREGATOR_WORLD_WIT: &str = r#"package torvyn:streaming@0.1.0;

world stream-aggregator {
    import types;
    import buffer-allocator;

    export aggregator;
}
"#;

const AGGREGATOR_LIB_RS: &str = r#"// Generated by `torvyn init --template aggregator` on {{date}}
// Torvyn CLI v{{torvyn_version}}
//
// This component implements the torvyn:streaming/aggregator interface.
// It accumulates elements over a window and emits aggregated results.

wit_bindgen::generate!({
    world: "stream-aggregator",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::aggregator::Guest;
use torvyn::streaming::types::{StreamElement, OutputElement, ElementMeta, ProcessError};
use torvyn::streaming::buffer_allocator;

struct {{component_type}};

// Window state
static mut WINDOW_COUNT: u64 = 0;
const WINDOW_SIZE: u64 = 10;

impl Guest for {{component_type}} {
    fn ingest(element: StreamElement) -> Result<Option<OutputElement>, ProcessError> {
        // TODO: Implement your aggregation logic.
        //
        // Return Ok(None) to absorb without producing output.
        // Return Ok(Some(output)) when a window completes.

        unsafe {
            WINDOW_COUNT += 1;
            if WINDOW_COUNT >= WINDOW_SIZE {
                WINDOW_COUNT = 0;

                // Clone input buffer and emit aggregated result
                let data = element.payload.read_all();
                let out = buffer_allocator::allocate(data.len() as u64)
                    .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
                out.append(&data)
                    .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;

                Ok(Some(OutputElement {
                    meta: ElementMeta {
                        sequence: element.meta.sequence,
                        timestamp_ns: element.meta.timestamp_ns,
                        content_type: element.meta.content_type,
                    },
                    payload: out.freeze(),
                }))
            } else {
                Ok(None)
            }
        }
    }

    fn flush() -> Result<Vec<OutputElement>, ProcessError> {
        // Called when the stream ends. Return any remaining buffered results.
        Ok(vec![])
    }
}

export!({{component_type}});
"#;

const AGGREGATOR_README: &str = r#"# {{project_name}}

A Torvyn stateful windowed aggregator component.

## Quick Start

```bash
torvyn check       # Validate contracts
torvyn build       # Compile to WebAssembly
```

## Project Structure

- `Torvyn.toml` — Project manifest
- `wit/torvyn-streaming/` — Torvyn streaming WIT contracts
- `src/lib.rs` — Component implementation
"#;

// ---------------------------------------------------------------------------
// Full-pipeline template
// ---------------------------------------------------------------------------

/// The `full-pipeline` template: complete multi-component pipeline.
pub fn full_pipeline_template() -> Template {
    Template {
        description: "Complete pipeline with source + transform + sink".into(),
        files: vec![
            tf("Torvyn.toml", FULL_PIPELINE_TORVYN_TOML),
            // Source component
            tf("components/source/Cargo.toml", FP_SOURCE_CARGO_TOML),
            tf(
                "components/source/wit/torvyn-streaming/types.wit",
                TORVYN_STREAMING_TYPES_WIT,
            ),
            tf(
                "components/source/wit/torvyn-streaming/source.wit",
                TORVYN_STREAMING_SOURCE_WIT,
            ),
            tf(
                "components/source/wit/torvyn-streaming/buffer-allocator.wit",
                TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT,
            ),
            tf(
                "components/source/wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_SOURCE_WORLD_WIT,
            ),
            tf("components/source/src/lib.rs", FP_SOURCE_LIB_RS),
            // Transform component
            tf("components/transform/Cargo.toml", FP_TRANSFORM_CARGO_TOML),
            tf(
                "components/transform/wit/torvyn-streaming/types.wit",
                TORVYN_STREAMING_TYPES_WIT,
            ),
            tf(
                "components/transform/wit/torvyn-streaming/processor.wit",
                TORVYN_STREAMING_PROCESSOR_WIT,
            ),
            tf(
                "components/transform/wit/torvyn-streaming/buffer-allocator.wit",
                TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT,
            ),
            tf(
                "components/transform/wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_TRANSFORM_WORLD_WIT,
            ),
            tf("components/transform/src/lib.rs", FP_TRANSFORM_LIB_RS),
            // Sink component
            tf("components/sink/Cargo.toml", FP_SINK_CARGO_TOML),
            tf(
                "components/sink/wit/torvyn-streaming/types.wit",
                TORVYN_STREAMING_TYPES_WIT,
            ),
            tf(
                "components/sink/wit/torvyn-streaming/sink.wit",
                TORVYN_STREAMING_SINK_WIT,
            ),
            tf(
                "components/sink/wit/torvyn-streaming/world.wit",
                TORVYN_STREAMING_SINK_WORLD_WIT,
            ),
            tf("components/sink/src/lib.rs", FP_SINK_LIB_RS),
            tf(".gitignore", COMMON_GITIGNORE),
            tf("README.md", FP_README),
        ],
    }
}

const FULL_PIPELINE_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
description = "A complete streaming pipeline with source, transform, and sink"
contract_version = "{{contract_version}}"

[[component]]
name = "source"
path = "components/source"
language = "rust"

[[component]]
name = "transform"
path = "components/transform"
language = "rust"

[[component]]
name = "sink"
path = "components/sink"
language = "rust"

[flow.main]
description = "Generate messages, transform them, and print to stdout"

[flow.main.nodes.source]
component = "source"
interface = "torvyn:streaming/source"

[flow.main.nodes.transform]
component = "transform"
interface = "torvyn:streaming/processor"

[flow.main.nodes.sink]
component = "sink"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "transform", port = "input" }

[[flow.main.edges]]
from = { node = "transform", port = "output" }
to = { node = "sink", port = "input" }
"#;

const FP_SOURCE_CARGO_TOML: &str = r#"[package]
name = "source"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "source:component"
"#;

const FP_SOURCE_LIB_RS: &str = r#"// Source component for the {{project_name}} pipeline
// Generates numbered greeting messages.

wit_bindgen::generate!({
    world: "data-source",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::source::Guest;
use torvyn::streaming::types::{OutputElement, ElementMeta, ProcessError, BackpressureSignal};
use torvyn::streaming::buffer_allocator;

struct Source;

static mut COUNTER: u64 = 0;

impl Guest for Source {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let count = unsafe {
            COUNTER += 1;
            COUNTER
        };

        if count > 1000 {
            return Ok(None);
        }

        let message = format!("Hello, Torvyn! ({count})");
        let buf = buffer_allocator::allocate(message.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(message.as_bytes())
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;

        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: count,
                timestamp_ns: 0,
                content_type: "text/plain".to_string(),
            },
            payload: buf.freeze(),
        }))
    }

    fn notify_backpressure(_signal: BackpressureSignal) {}
}

export!(Source);
"#;

const FP_TRANSFORM_CARGO_TOML: &str = r#"[package]
name = "transform"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "transform:component"
"#;

const FP_TRANSFORM_LIB_RS: &str = r#"// Transform component for the {{project_name}} pipeline
// Converts input text to uppercase.

wit_bindgen::generate!({
    world: "transform",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::processor::{Guest, ProcessResult};
use torvyn::streaming::types::{StreamElement, OutputElement, ElementMeta, ProcessError};
use torvyn::streaming::buffer_allocator;

struct Transform;

impl Guest for Transform {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        let data = input.payload.read_all();
        let text = String::from_utf8_lossy(&data);
        let upper = text.to_uppercase();

        let out_buf = buffer_allocator::allocate(upper.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        out_buf.append(upper.as_bytes())
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;

        Ok(ProcessResult::Emit(OutputElement {
            meta: ElementMeta {
                sequence: input.meta.sequence,
                timestamp_ns: input.meta.timestamp_ns,
                content_type: input.meta.content_type,
            },
            payload: out_buf.freeze(),
        }))
    }
}

export!(Transform);
"#;

const FP_SINK_CARGO_TOML: &str = r#"[package]
name = "sink"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "sink:component"
"#;

const FP_SINK_LIB_RS: &str = r#"// Sink component for the {{project_name}} pipeline
// Prints received messages to stdout.

wit_bindgen::generate!({
    world: "data-sink",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::sink::Guest;
use torvyn::streaming::types::{StreamElement, ProcessError, BackpressureSignal};

struct Sink;

impl Guest for Sink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let data = element.payload.read_all();
        let text = String::from_utf8_lossy(&data);
        println!("{text}");
        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        Ok(())
    }
}

export!(Sink);
"#;

const FP_README: &str = r#"# {{project_name}}

A complete Torvyn streaming pipeline with three components:

- **source** — generates numbered greeting messages
- **transform** — converts text to uppercase
- **sink** — prints messages to stdout

## Quick Start

```bash
torvyn check              # Validate contracts and manifest
torvyn build              # Compile all components to WebAssembly
torvyn run                # Run the pipeline
torvyn run --limit 10     # Run with element limit
```

## Project Structure

- `Torvyn.toml` — Project manifest with flow definition
- `components/source/` — Source component (data producer)
- `components/transform/` — Transform component (data processor)
- `components/sink/` — Sink component (data consumer)
"#;

// ---------------------------------------------------------------------------
// Empty template
// ---------------------------------------------------------------------------

/// The `empty` template: minimal skeleton.
pub fn empty_template() -> Template {
    Template {
        description: "Minimal skeleton for experienced users".into(),
        files: vec![
            tf("Torvyn.toml", EMPTY_TORVYN_TOML),
            tf(".gitignore", COMMON_GITIGNORE),
        ],
    }
}

const EMPTY_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"
"#;

// ---------------------------------------------------------------------------
// Common files
// ---------------------------------------------------------------------------

const COMMON_GITIGNORE: &str = r#"target/
.torvyn/
*.wasm
"#;
