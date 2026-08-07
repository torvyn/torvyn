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
/// Emit the complete `torvyn:streaming` contract under `prefix`.
///
/// Every scaffolded component receives the whole canonical package, not the
/// subset its own world happens to reference. Two reasons: `world.wit`
/// declares every world, so a component can switch role by changing one line
/// of its Cargo.toml; and the files are the contract crate's own, embedded at
/// compile time, so a scaffolded project cannot drift from the runtime it will
/// run against.
///
/// It was drift that made scaffolded projects unrunnable: the templates
/// carried a hand-maintained copy of the WIT whose `data-source` world had
/// lost `export lifecycle`, so the host refused every generated source with
/// "not a data-source".
fn canonical_wit(prefix: &str) -> Vec<TemplateFile> {
    [
        ("types.wit", TORVYN_STREAMING_TYPES_WIT),
        ("source.wit", TORVYN_STREAMING_SOURCE_WIT),
        ("sink.wit", TORVYN_STREAMING_SINK_WIT),
        ("processor.wit", TORVYN_STREAMING_PROCESSOR_WIT),
        ("lifecycle.wit", TORVYN_STREAMING_LIFECYCLE_WIT),
        (
            "buffer-allocator.wit",
            TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT,
        ),
        ("world.wit", TORVYN_STREAMING_WORLD_WIT),
    ]
    .into_iter()
    .map(|(name, content)| tf(&format!("{prefix}{name}"), content))
    .collect()
}

/// The example source that lets a scaffolded pipeline run.
///
/// Emits a thousand numbered greetings and then ends the stream, which is what
/// makes a scaffolded project finish on its own rather than run forever.
///
/// The same files back the `full-pipeline` template and the companion a
/// single-component template ships, so there is one implementation of the
/// contract's source world rather than two that can drift apart.
fn companion_source(dir: &str) -> Vec<TemplateFile> {
    let mut files = canonical_wit(&format!("{dir}/wit/torvyn-streaming/"));
    files.extend([
        tf(&format!("{dir}/Cargo.toml"), FP_SOURCE_CARGO_TOML),
        tf(&format!("{dir}/src/lib.rs"), FP_SOURCE_LIB_RS),
    ]);
    files
}

/// The example sink that lets a scaffolded pipeline run.
///
/// Prints what it receives, which is how a scaffolded project shows the user
/// its own output. It needs the `stdio:stdout` grant to do so; every manifest
/// that places this component grants exactly that and nothing else.
fn companion_sink(dir: &str) -> Vec<TemplateFile> {
    let mut files = canonical_wit(&format!("{dir}/wit/torvyn-streaming/"));
    files.extend([
        tf(&format!("{dir}/Cargo.toml"), FP_SINK_CARGO_TOML),
        tf(&format!("{dir}/src/lib.rs"), FP_SINK_LIB_RS),
    ]);
    files
}

/// Files for the stateless data transformer template.
pub fn transform_template() -> Template {
    let mut files = canonical_wit("wit/torvyn-streaming/");
    files.extend([
        tf("Torvyn.toml", TRANSFORM_TORVYN_TOML),
        tf("Cargo.toml", TRANSFORM_CARGO_TOML),
        tf("src/lib.rs", TRANSFORM_LIB_RS),
        tf(".gitignore", COMMON_GITIGNORE),
        tf("README.md", TRANSFORM_README),
    ]);
    // A transform has nothing to read from and nowhere to write to. Without
    // both ends the scaffolded project cannot run, and `torvyn init` prints
    // `torvyn run` as its third step.
    files.extend(companion_source("components/source"));
    files.extend(companion_sink("components/sink"));
    Template {
        description: "Stateless data transformer".into(),
        files,
    }
}

const TRANSFORM_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"

# The component you are building. Its source is at src/lib.rs.
[[component]]
name = "{{project_name}}"
path = "."
language = "rust"

# A transform reads from something and writes to something. These two example
# components supply both ends so the pipeline runs end to end; replace them
# with your own, or point the flow below at components you already have.
[[component]]
name = "source"
path = "components/source"
language = "rust"

[[component]]
name = "sink"
path = "components/sink"
language = "rust"

[flow.main]
description = "Feed {{project_name}} from an example source and print what it emits"

[flow.main.nodes.source]
component = "source"
interface = "torvyn:streaming/source"

[flow.main.nodes.transform]
component = "{{project_name}}"
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

# Components run fully sandboxed by default: no filesystem, no network, no
# stdio. The sink prints what it receives, so it is granted stdout — and
# nothing else. Grant keys are flow-node names.
[security.grants.sink]
capabilities = ["stdio:stdout"]
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
package = "torvyn:streaming"

# cargo-component reads the component's WIT from here. Without an explicit
# target it looks in `wit/` alone, misses `wit/torvyn-streaming/`, and reports
# that no package header was found.
[package.metadata.component.target]
world = "transform"
path = "wit/torvyn-streaming"

# An empty [workspace] table makes this component a standalone cargo package.
# Without it, creating a Torvyn project inside another cargo workspace makes
# cargo refuse to build the component until it is added to that workspace's
# members.
[workspace]
"#;

// ---------------------------------------------------------------------------
// Shared Torvyn streaming WIT definitions (bundled with templates)
// ---------------------------------------------------------------------------

const TORVYN_STREAMING_TYPES_WIT: &str =
    include_str!("../../../torvyn-contracts/wit/torvyn-streaming/types.wit");

const TORVYN_STREAMING_PROCESSOR_WIT: &str =
    include_str!("../../../torvyn-contracts/wit/torvyn-streaming/processor.wit");

const TORVYN_STREAMING_BUFFER_ALLOCATOR_WIT: &str =
    include_str!("../../../torvyn-contracts/wit/torvyn-streaming/buffer-allocator.wit");

const TORVYN_STREAMING_LIFECYCLE_WIT: &str =
    include_str!("../../../torvyn-contracts/wit/torvyn-streaming/lifecycle.wit");

const TORVYN_STREAMING_SOURCE_WIT: &str =
    include_str!("../../../torvyn-contracts/wit/torvyn-streaming/source.wit");

const TORVYN_STREAMING_SINK_WIT: &str =
    include_str!("../../../torvyn-contracts/wit/torvyn-streaming/sink.wit");

const TORVYN_STREAMING_WORLD_WIT: &str =
    include_str!("../../../torvyn-contracts/wit/torvyn-streaming/world.wit");

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
use torvyn::streaming::types::{StreamElement, OutputElement, ElementMeta, ProcessError};
use torvyn::streaming::buffer_allocator;

struct {{component_type}};

impl Guest for {{component_type}} {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        // TODO: Implement your transform logic here.
        //
        // `input` carries:
        //   - input.meta: element metadata (sequence, timestamp, content type)
        //   - input.payload: a *borrowed* handle to the incoming buffer
        //
        // The input buffer is borrowed for the duration of this call and the
        // host reclaims it afterwards, so an emitted element must own its
        // payload: allocate a buffer, write into it, and freeze it. That is
        // what keeps a component from handing out a reference to memory it
        // does not own.
        //
        // This body is a pass-through — it copies the payload through
        // unchanged. Replace the `data` below with whatever you produce.
        let data = input.payload.read_all();

        let out_buf = buffer_allocator::allocate(data.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        out_buf.append(&data)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;

        Ok(ProcessResult::Emit(OutputElement {
            meta: ElementMeta {
                sequence: input.meta.sequence,
                timestamp_ns: input.meta.timestamp_ns,
                content_type: input.meta.content_type,
            },
            payload: out_buf.freeze(),
        }))

        // Returning `Ok(ProcessResult::Drop)` instead discards the element.
    }
}

export!({{component_type}});
"#;

const TRANSFORM_README: &str = r#"# {{project_name}}

A Torvyn streaming transform component, with an example source and sink so the
pipeline runs end to end.

## Quick Start

```bash
torvyn check       # Validate contracts and manifest
torvyn build       # Compile every component to WebAssembly
torvyn run         # Run the pipeline
```

`torvyn run` prints the transformed messages. Edit `src/lib.rs` and run it
again to see your change.

## Project Structure

- `src/lib.rs` — **your component.** This is the file to edit.
- `Torvyn.toml` — project manifest, including the `main` flow
- `wit/torvyn-streaming/` — the Torvyn streaming WIT contracts
- `components/source/` — example source; replace it with your own
- `components/sink/` — example sink; prints what your transform emits

The example components exist because a transform has nothing to read from and
nowhere to write to. When you have real ones, point `[flow.main]` in
`Torvyn.toml` at them and delete these.

## Learn More

- [Torvyn Documentation](https://docs.torvyn.dev)
- [WIT Contract Guide](https://docs.torvyn.dev/guides/wit-primer)
"#;

// ---------------------------------------------------------------------------
// Source template
// ---------------------------------------------------------------------------

/// The `source` template: a data producer.
/// Files for the data producer (no input, one output) template.
pub fn source_template() -> Template {
    let mut files = canonical_wit("wit/torvyn-streaming/");
    files.extend([
        tf("Torvyn.toml", SOURCE_TORVYN_TOML),
        tf("Cargo.toml", SOURCE_CARGO_TOML),
        tf("src/lib.rs", SOURCE_LIB_RS),
        tf(".gitignore", COMMON_GITIGNORE),
        tf("README.md", SOURCE_README),
    ]);
    // A source needs somewhere for its elements to go before it can run.
    files.extend(companion_sink("components/sink"));
    Template {
        description: "Data producer (no input, one output)".into(),
        files,
    }
}

const SOURCE_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"

# The component you are building. Its source is at src/lib.rs.
[[component]]
name = "{{project_name}}"
path = "."
language = "rust"

# A source needs somewhere for its elements to go. This example sink prints
# them, which is how you see what your source produces; replace it with your
# own, or point the flow below at a component you already have.
[[component]]
name = "sink"
path = "components/sink"
language = "rust"

[flow.main]
description = "Print what {{project_name}} produces"

[flow.main.nodes.source]
component = "{{project_name}}"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "sink"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }

# Components run fully sandboxed by default: no filesystem, no network, no
# stdio. The sink prints what it receives, so it is granted stdout — and
# nothing else. Grant keys are flow-node names.
[security.grants.sink]
capabilities = ["stdio:stdout"]
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
package = "torvyn:streaming"

# cargo-component reads the component's WIT from here. Without an explicit
# target it looks in `wit/` alone, misses `wit/torvyn-streaming/`, and reports
# that no package header was found.
[package.metadata.component.target]
world = "data-source"
path = "wit/torvyn-streaming"

# An empty [workspace] table makes this component a standalone cargo package.
# Without it, creating a Torvyn project inside another cargo workspace makes
# cargo refuse to build the component until it is added to that workspace's
# members.
[workspace]
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
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::types::{OutputElement, ElementMeta, ProcessError, BackpressureSignal};
use torvyn::streaming::buffer_allocator;

struct {{component_type}};

static mut COUNTER: u64 = 0;

// The `data-source` and `data-sink` worlds export `lifecycle` as well as their
// role interface, and the host calls `lifecycle.init` on every component
// before the pipeline starts. A component that exports only its role
// interface does not satisfy its world, and the runtime declines to run it.
impl LifecycleGuest for {{component_type}} {
    fn init(_config: String) -> Result<(), ProcessError> {
        Ok(())
    }

    fn teardown() {}
}

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

A Torvyn streaming source component, with an example sink so the pipeline runs
end to end.

## Quick Start

```bash
torvyn check       # Validate contracts and manifest
torvyn build       # Compile every component to WebAssembly
torvyn run         # Run the pipeline
```

`torvyn run` prints what your source produces. Edit `src/lib.rs` and run it
again to see your change.

## Project Structure

- `src/lib.rs` — **your component.** This is the file to edit.
- `Torvyn.toml` — project manifest, including the `main` flow
- `wit/torvyn-streaming/` — the Torvyn streaming WIT contracts
- `components/sink/` — example sink; it prints each element your source emits

The example sink exists because a source needs somewhere for its elements to
go. When you have a real one, point `[flow.main]` in `Torvyn.toml` at it and
delete this.
"#;

// ---------------------------------------------------------------------------
// Sink template
// ---------------------------------------------------------------------------

/// The `sink` template: a data consumer.
/// Files for the data consumer (one input, no output) template.
pub fn sink_template() -> Template {
    let mut files = canonical_wit("wit/torvyn-streaming/");
    files.extend([
        tf("Torvyn.toml", SINK_TORVYN_TOML),
        tf("Cargo.toml", SINK_CARGO_TOML),
        tf("src/lib.rs", SINK_LIB_RS),
        tf(".gitignore", COMMON_GITIGNORE),
        tf("README.md", SINK_README),
    ]);
    // A sink needs something to feed it before it can run.
    files.extend(companion_source("components/source"));
    Template {
        description: "Data consumer (one input, no output)".into(),
        files,
    }
}

const SINK_TORVYN_TOML: &str = r#"[torvyn]
name = "{{project_name}}"
version = "0.1.0"
contract_version = "{{contract_version}}"

# The component you are building. Its source is at src/lib.rs.
[[component]]
name = "{{project_name}}"
path = "."
language = "rust"

# A sink needs something to feed it. This example source emits numbered
# greetings; replace it with your own, or point the flow below at a component
# you already have.
[[component]]
name = "source"
path = "components/source"
language = "rust"

[flow.main]
description = "Feed {{project_name}} from an example source"

[flow.main.nodes.source]
component = "source"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "{{project_name}}"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }

# Components run fully sandboxed by default: no filesystem, no network, no
# stdio. Your sink prints what it receives, so it is granted stdout — and
# nothing else. Grant keys are flow-node names, so this grants the `sink` node
# above, which is {{project_name}}.
[security.grants.sink]
capabilities = ["stdio:stdout"]
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
package = "torvyn:streaming"

# cargo-component reads the component's WIT from here. Without an explicit
# target it looks in `wit/` alone, misses `wit/torvyn-streaming/`, and reports
# that no package header was found.
[package.metadata.component.target]
world = "data-sink"
path = "wit/torvyn-streaming"

# An empty [workspace] table makes this component a standalone cargo package.
# Without it, creating a Torvyn project inside another cargo workspace makes
# cargo refuse to build the component until it is added to that workspace's
# members.
[workspace]
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
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::types::{StreamElement, ProcessError, BackpressureSignal};

struct {{component_type}};

// The `data-source` and `data-sink` worlds export `lifecycle` as well as their
// role interface, and the host calls `lifecycle.init` on every component
// before the pipeline starts. A component that exports only its role
// interface does not satisfy its world, and the runtime declines to run it.
impl LifecycleGuest for {{component_type}} {
    fn init(_config: String) -> Result<(), ProcessError> {
        Ok(())
    }

    fn teardown() {}
}

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

A Torvyn streaming sink component, with an example source so the pipeline runs
end to end.

## Quick Start

```bash
torvyn check       # Validate contracts and manifest
torvyn build       # Compile every component to WebAssembly
torvyn run         # Run the pipeline
```

`torvyn run` feeds your sink from the example source. Edit `src/lib.rs` and run
it again to see your change.

## Project Structure

- `src/lib.rs` — **your component.** This is the file to edit.
- `Torvyn.toml` — project manifest, including the `main` flow
- `wit/torvyn-streaming/` — the Torvyn streaming WIT contracts
- `components/source/` — example source; it emits numbered greetings

The example source exists because a sink needs something to feed it. When you
have a real one, point `[flow.main]` in `Torvyn.toml` at it and delete this.

Your sink prints what it receives, which needs the `stdio:stdout` capability.
`[security.grants.sink]` in `Torvyn.toml` grants exactly that; a component
that prints without the grant produces no output at all.
"#;

// ---------------------------------------------------------------------------
// Full-pipeline template
// ---------------------------------------------------------------------------

/// The `full-pipeline` template: complete multi-component pipeline.
/// Files for the complete pipeline: source, transform, and sink template.
pub fn full_pipeline_template() -> Template {
    let mut files = companion_source("components/source");
    files.extend(companion_sink("components/sink"));
    files.extend(canonical_wit("components/transform/wit/torvyn-streaming/"));
    files.extend([
        tf("Torvyn.toml", FULL_PIPELINE_TORVYN_TOML),
        tf("components/transform/Cargo.toml", FP_TRANSFORM_CARGO_TOML),
        tf("components/transform/src/lib.rs", FP_TRANSFORM_LIB_RS),
        tf(".gitignore", COMMON_GITIGNORE),
        tf("README.md", FP_README),
    ]);
    Template {
        description: "Complete pipeline with source + transform + sink".into(),
        files,
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

# Components run fully sandboxed by default: no filesystem, no network, no
# stdio. The sink prints what it receives, so it is granted stdout — and
# nothing else. Grant keys are flow-node names.
[security.grants.sink]
capabilities = ["stdio:stdout"]
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
package = "torvyn:streaming"

# cargo-component reads the component's WIT from here. Without an explicit
# target it looks in `wit/` alone, misses `wit/torvyn-streaming/`, and reports
# that no package header was found.
[package.metadata.component.target]
world = "data-source"
path = "wit/torvyn-streaming"

# An empty [workspace] table makes this component a standalone cargo package.
# Without it, creating a Torvyn project inside another cargo workspace makes
# cargo refuse to build the component until it is added to that workspace's
# members.
[workspace]
"#;

const FP_SOURCE_LIB_RS: &str = r#"// Source component for the {{project_name}} pipeline
// Generates numbered greeting messages.

wit_bindgen::generate!({
    world: "data-source",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::source::Guest;
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::types::{OutputElement, ElementMeta, ProcessError, BackpressureSignal};
use torvyn::streaming::buffer_allocator;

struct Source;

static mut COUNTER: u64 = 0;

// The `data-source` and `data-sink` worlds export `lifecycle` as well as their
// role interface, and the host calls `lifecycle.init` on every component
// before the pipeline starts. A component that exports only its role
// interface does not satisfy its world, and the runtime declines to run it.
impl LifecycleGuest for Source {
    fn init(_config: String) -> Result<(), ProcessError> {
        Ok(())
    }

    fn teardown() {}
}

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
package = "torvyn:streaming"

# cargo-component reads the component's WIT from here. Without an explicit
# target it looks in `wit/` alone, misses `wit/torvyn-streaming/`, and reports
# that no package header was found.
[package.metadata.component.target]
world = "transform"
path = "wit/torvyn-streaming"

# An empty [workspace] table makes this component a standalone cargo package.
# Without it, creating a Torvyn project inside another cargo workspace makes
# cargo refuse to build the component until it is added to that workspace's
# members.
[workspace]
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
package = "torvyn:streaming"

# cargo-component reads the component's WIT from here. Without an explicit
# target it looks in `wit/` alone, misses `wit/torvyn-streaming/`, and reports
# that no package header was found.
[package.metadata.component.target]
world = "data-sink"
path = "wit/torvyn-streaming"

# An empty [workspace] table makes this component a standalone cargo package.
# Without it, creating a Torvyn project inside another cargo workspace makes
# cargo refuse to build the component until it is added to that workspace's
# members.
[workspace]
"#;

const FP_SINK_LIB_RS: &str = r#"// Sink component for the {{project_name}} pipeline
// Prints received messages to stdout.

wit_bindgen::generate!({
    world: "data-sink",
    path: "wit/torvyn-streaming",
});

use exports::torvyn::streaming::sink::Guest;
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::types::{StreamElement, ProcessError, BackpressureSignal};

struct Sink;

// The `data-source` and `data-sink` worlds export `lifecycle` as well as their
// role interface, and the host calls `lifecycle.init` on every component
// before the pipeline starts. A component that exports only its role
// interface does not satisfy its world, and the runtime declines to run it.
impl LifecycleGuest for Sink {
    fn init(_config: String) -> Result<(), ProcessError> {
        Ok(())
    }

    fn teardown() {}
}

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
/// Files for the minimal skeleton template.
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
