# Tutorial: AI Token Streaming Pipeline

This tutorial builds a pipeline that simulates AI token streaming — a pattern common in AI inference systems where tokens are generated incrementally and must be filtered, assembled, and delivered in real time.

**What you will build:**

1. **Token Source** — simulates an AI model emitting tokens one at a time.
2. **Content Policy Filter** — evaluates each token against a blocklist and drops prohibited content.
3. **Token Sink** — collects tokens and displays the assembled text.

**What you will learn:** Streaming at the individual-element level, content filtering as a zero-allocation operation, backpressure between a fast producer and a slower consumer, and tracing to understand flow behavior.

**Prerequisites:** Complete [Your First Pipeline](../getting-started/your-first-pipeline.md).

**Time required:** 20–30 minutes.

## Project Setup

```bash
torvyn init token-pipeline --template full-pipeline
cd token-pipeline
```

We will replace the generated components with our own implementations.

## Step 1: Token Source

The source simulates an AI model generating tokens. Each token is a short string (a word or punctuation mark). The source emits one token per `pull` call.

### Contract: `components/source/wit/world.wit`

```wit
package source:component;

world source {
    import torvyn:streaming/types@0.1.0;
    import torvyn:resources/buffer-ops@0.1.0;
    export torvyn:streaming/source@0.1.0;
}
```

### Implementation: `components/source/src/lib.rs`

```rust
// Token source: simulates an AI model emitting tokens.

wit_bindgen::generate!({
    world: "source",
    path: "wit",
});

use exports::torvyn::streaming::source::Guest;
use torvyn::streaming::types::{OutputElement, ElementMeta, ProcessError, BackpressureSignal};
use torvyn::resources::buffer_ops;

struct TokenSource;

static mut INDEX: usize = 0;
static mut PAUSED: bool = false;

// Simulated token stream: a paragraph of generated text, split into tokens.
const TOKENS: &[&str] = &[
    "The", " quick", " brown", " fox", " jumped",
    " over", " the", " lazy", " dog", ".",
    " Meanwhile", ",", " the", " harmful_content",
    " was", " intercepted", " by", " the",
    " content", " policy", " filter", ".",
    " The", " system", " continued", " to",
    " operate", " normally", " after",
    " filtering", ".",
];

impl Guest for TokenSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        // Respect backpressure.
        if unsafe { PAUSED } {
            return Ok(None);
        }

        let idx = unsafe {
            let current = INDEX;
            INDEX += 1;
            current
        };

        if idx >= TOKENS.len() {
            return Ok(None); // End of stream.
        }

        let token = TOKENS[idx];
        let buf = buffer_ops::allocate(token.len() as u64)
            .map_err(|_| ProcessError::Internal("allocation failed".into()))?;
        buf.append(token.as_bytes())
            .map_err(|_| ProcessError::Internal("write failed".into()))?;
        buf.set_content_type("text/plain");

        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: 0,
                timestamp_ns: 0,
                content_type: "text/plain".into(),
            },
            payload: buf.freeze(),
        }))
    }

    fn notify_backpressure(signal: BackpressureSignal) {
        unsafe {
            PAUSED = matches!(signal, BackpressureSignal::Pause);
        }
    }
}

export!(TokenSource);
```

This source respects backpressure signals: when the downstream pipeline signals `Pause`, the source stops producing tokens until it receives `Ready`.

## Step 2: Content Policy Filter

The filter examines each token and rejects any that match a blocklist. Filters in Torvyn implement the `torvyn:filtering/filter` interface, which is optimized for this use case: the filter receives a borrowed reference to the element and returns a boolean. No buffer allocation is needed because the filter does not produce new data — it only decides whether to pass or drop the element.

### Contract: `components/transform/wit/world.wit`

Replace the transform's WIT contract with a filter contract:

```wit
package transform:component;

world transform {
    import torvyn:streaming/types@0.1.0;
    export torvyn:filtering/filter@0.1.0;
}
```

Notice: no `buffer-ops` import. Filters do not allocate buffers, which makes them very efficient.

### Implementation: `components/transform/src/lib.rs`

```rust
// Content policy filter: drops tokens that match a blocklist.

wit_bindgen::generate!({
    world: "transform",
    path: "wit",
});

use exports::torvyn::filtering::filter::Guest;
use torvyn::streaming::types::{StreamElement, ProcessError};

struct ContentFilter;

// Tokens that should be blocked by the content policy.
const BLOCKED_TOKENS: &[&str] = &[
    "harmful_content",
    "prohibited_term",
    "unsafe_output",
];

impl Guest for ContentFilter {
    fn evaluate(input: &StreamElement) -> Result<bool, ProcessError> {
        let data = input.payload.read_all();
        let token = String::from_utf8_lossy(&data);
        let trimmed = token.trim();

        // Check against blocklist.
        for blocked in BLOCKED_TOKENS {
            if trimmed.eq_ignore_ascii_case(blocked) {
                // Reject this token.
                return Ok(false);
            }
        }

        // Pass the token through.
        Ok(true)
    }
}

export!(ContentFilter);
```

The filter reads the token text and checks it against the blocklist. Returning `false` tells the runtime to drop the element — no copy, no allocation, no downstream delivery. This is one of the most efficient patterns in Torvyn: a filter that only reads metadata and a small payload can run with minimal overhead.

## Step 3: Token Collection Sink

The sink collects tokens and assembles them into complete text. It demonstrates how a sink can maintain internal state.

### Contract: `components/sink/wit/world.wit`

Create the sink directory and contract:

```bash
mkdir -p components/sink/wit
mkdir -p components/sink/src
```

```wit
package sink:component;

world sink {
    import torvyn:streaming/types@0.1.0;
    export torvyn:streaming/sink@0.1.0;
}
```

### Cargo.toml: `components/sink/Cargo.toml`

```toml
[package]
name = "sink"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

[package.metadata.component]
package = "sink:component"
```

### Implementation: `components/sink/src/lib.rs`

```rust
// Token collection sink: assembles tokens into text and displays them.

wit_bindgen::generate!({
    world: "sink",
    path: "wit",
});

use exports::torvyn::streaming::sink::Guest;
use torvyn::streaming::types::{StreamElement, BackpressureSignal, ProcessError};

struct TokenCollector;

static mut COLLECTED: Vec<u8> = Vec::new();
static mut TOKEN_COUNT: u64 = 0;

impl Guest for TokenCollector {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let data = element.payload.read_all();

        unsafe {
            COLLECTED.extend_from_slice(&data);
            TOKEN_COUNT += 1;

            // Print a running status every 10 tokens.
            if TOKEN_COUNT % 10 == 0 {
                let text = String::from_utf8_lossy(&COLLECTED);
                eprintln!("[{} tokens] {}", TOKEN_COUNT, text);
            }
        }

        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        unsafe {
            let text = String::from_utf8_lossy(&COLLECTED);
            println!("\n── Assembled Text ──");
            println!("{text}");
            println!("\n── Stats ──");
            println!("Total tokens received: {TOKEN_COUNT}");
        }
        Ok(())
    }
}

export!(TokenCollector);
```

The sink collects all tokens into an internal buffer and prints progress every 10 tokens. When the stream completes, it prints the fully assembled text. The token "harmful_content" will be absent — filtered out by the content policy stage.

## Step 4: Pipeline Configuration

Update `Torvyn.toml`:

```toml
[torvyn]
name = "token-pipeline"
version = "0.1.0"
description = "AI token streaming with content policy filtering"
contract_version = "0.1.0"

[[component]]
name = "source"
path = "components/source"
language = "rust"

[[component]]
name = "filter"
path = "components/transform"
language = "rust"

[[component]]
name = "sink"
path = "components/sink"
language = "rust"

[flow.main]
description = "Token stream → content filter → collector"

[flow.main.nodes.source]
component = "file://./components/source/target/wasm32-wasip2/debug/source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.filter]
component = "file://./components/transform/target/wasm32-wasip2/debug/transform.wasm"
interface = "torvyn:filtering/filter"

[flow.main.nodes.sink]
component = "file://./components/sink/target/wasm32-wasip2/debug/sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "filter", port = "input" }

[[flow.main.edges]]
from = { node = "filter", port = "output" }
to = { node = "sink", port = "input" }
```

## Step 5: Build and Run

```bash
cd components/source && cargo component build --target wasm32-wasip2 && cd ../..
cd components/transform && cargo component build --target wasm32-wasip2 && cd ../..
cd components/sink && cargo component build --target wasm32-wasip2 && cd ../..
```

```bash
torvyn check
torvyn link
torvyn run
```

Expected output:

```
▶ Running flow "main"

[10 tokens] The quick brown fox jumped over the lazy dog.
[20 tokens] The quick brown fox jumped over the lazy dog. Meanwhile, the

── Assembled Text ──
The quick brown fox jumped over the lazy dog. Meanwhile, the was intercepted by the content policy filter. The system continued to operate normally after filtering.

── Stats ──
Total tokens received: 30
```

Notice that the token "harmful_content" is missing from the output. The filter dropped it, and the remaining tokens assembled into coherent text (minus the blocked word).

## Step 6: Trace the Filter Behavior

```bash
torvyn trace --limit 15 --show-backpressure
```

In the trace output, look for elements where the filter stage shows "drop" instead of "pass." This confirms that the filter is working and that the dropped element never reaches the sink — no buffer was allocated for it, and no downstream processing occurred.

## Concepts Demonstrated

- **Streaming at token granularity** — each token is a separate stream element, enabling per-token processing.
- **Filtering without allocation** — the filter reads the element and returns a boolean. No output buffer is allocated for passed elements; the runtime forwards the original buffer.
- **Backpressure** — the source respects pause signals from the downstream pipeline.
- **End-of-stream signaling** — the source returns `None` to signal completion; the runtime calls `complete()` on the sink.
- **Observable filtering** — `torvyn trace` shows exactly which elements were dropped and why.

## Next Steps

- [Event Enrichment Pipeline](event-enrichment-pipeline.md) — multi-stage processing with resource ownership tracking.
- [Custom Component Guide](custom-component-guide.md) — build components from scratch without templates.
