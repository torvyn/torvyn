# Token Streaming

## What It Demonstrates

An AI-oriented pipeline: a simulated LLM token source emits tokens one at a time, a content policy filter rejects tokens matching a block list, a token aggregator collects tokens into complete sentences, and an output sink writes the assembled text. Demonstrates Torvyn's fit for AI inference pipelines.

## Concepts Covered

- The `torvyn:filtering/filter` interface for accept/reject decisions
- Token-by-token streaming (granular elements)
- Aggregation with sentence-boundary detection
- AI pipeline composition pattern
- Filter components that inspect data without allocating buffers

## Key Components

**`components/token-source/src/lib.rs`**

```rust
//! Simulated LLM token source.
//!
//! Emits tokens from a pre-defined sequence simulating model output.
//! Each token is a separate stream element, mimicking the granularity
//! of real language model decoding.

wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

const TOKENS: &[&str] = &[
    "The", " quick", " brown", " fox", " jumped", " over",
    " the", " lazy", " dog", ".",
    " The", " blocked_word", " was", " filtered", ".",
    " Torvyn", " handles", " streaming", " tokens", ".",
];

struct TokenSource {
    index: usize,
}

static mut STATE: Option<TokenSource> = None;
fn state() -> &'static mut TokenSource {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for TokenSource {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { STATE = Some(TokenSource { index: 0 }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl SourceGuest for TokenSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();
        if s.index >= TOKENS.len() {
            return Ok(None);
        }
        let token = TOKENS[s.index];
        let buf = buffer_allocator::allocate(token.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(token.as_bytes())
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("text/plain; charset=utf-8");
        let frozen = buf.freeze();
        s.index += 1;
        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: (s.index - 1) as u64,
                timestamp_ns: 0,
                content_type: "text/plain; charset=utf-8".to_string(),
            },
            payload: frozen,
        }))
    }
    fn notify_backpressure(_signal: BackpressureSignal) {}
}

export!(TokenSource);
```

**`components/content-filter/src/lib.rs`**

```rust
//! Content policy filter.
//!
//! Uses the torvyn:filtering/filter interface to accept or reject tokens.
//! This component type is extremely efficient: it does not allocate output
//! buffers. It reads the token to inspect it, then returns a boolean.
//! The runtime forwards or drops the element based on the result.

wit_bindgen::generate!({
    world: "content-filter",
    path: "../../wit",
});

use exports::torvyn::filtering::filter::Guest as FilterGuest;
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::types::*;

struct ContentFilter {
    blocked_words: Vec<String>,
    filtered_count: u64,
}

static mut STATE: Option<ContentFilter> = None;
fn state() -> &'static mut ContentFilter {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for ContentFilter {
    fn init(config: String) -> Result<(), ProcessError> {
        // Config: comma-separated list of blocked words.
        let blocked: Vec<String> = if config.is_empty() {
            vec!["blocked_word".to_string()]
        } else {
            config.split(',').map(|s| s.trim().to_string()).collect()
        };
        unsafe {
            STATE = Some(ContentFilter {
                blocked_words: blocked,
                filtered_count: 0,
            });
        }
        Ok(())
    }
    fn teardown() {
        let s = state();
        if s.filtered_count > 0 {
            println!(
                "[content-filter] Filtered {} token(s) during this flow.",
                s.filtered_count
            );
        }
        unsafe { STATE = None; }
    }
}

impl FilterGuest for ContentFilter {
    /// Evaluate whether a token passes the content policy.
    ///
    /// - true: token passes through to downstream.
    /// - false: token is dropped by the runtime (no output buffer allocated).
    ///
    /// The filter reads the borrowed buffer to inspect the token contents.
    /// This is a single measured copy. No output buffer is allocated.
    fn evaluate(element: StreamElement) -> Result<bool, ProcessError> {
        let s = state();
        let bytes = element.payload.read_all();
        let token = String::from_utf8_lossy(&bytes);
        let trimmed = token.trim();

        for blocked in &s.blocked_words {
            if trimmed.eq_ignore_ascii_case(blocked) {
                s.filtered_count += 1;
                return Ok(false);
            }
        }
        Ok(true)
    }
}

export!(ContentFilter);
```

**`components/sentence-aggregator/src/lib.rs`**

```rust
//! Token-to-sentence aggregator.
//!
//! Collects streaming tokens into complete sentences. A sentence boundary
//! is detected when a token ends with '.', '!', or '?'. When a boundary
//! is reached, the accumulated text is emitted as a single output element.
//! Any remaining text is emitted on flush().

wit_bindgen::generate!({
    world: "stream-aggregator",
    path: "../../wit",
});

use exports::torvyn::aggregation::aggregator::Guest as AggregatorGuest;
use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct SentenceAggregator {
    buffer: String,
    sentence_count: u64,
}

static mut STATE: Option<SentenceAggregator> = None;
fn state() -> &'static mut SentenceAggregator {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for SentenceAggregator {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe {
            STATE = Some(SentenceAggregator {
                buffer: String::new(),
                sentence_count: 0,
            });
        }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

/// Helper: allocate a buffer, write sentence text, freeze, return as OutputElement.
fn emit_sentence(sentence: &str, seq: u64) -> Result<OutputElement, ProcessError> {
    let bytes = sentence.as_bytes();
    let buf = buffer_allocator::allocate(bytes.len() as u64)
        .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
    buf.append(bytes)
        .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
    buf.set_content_type("text/plain");
    let frozen = buf.freeze();
    Ok(OutputElement {
        meta: ElementMeta {
            sequence: seq,
            timestamp_ns: 0,
            content_type: "text/plain".to_string(),
        },
        payload: frozen,
    })
}

impl AggregatorGuest for SentenceAggregator {
    fn ingest(element: StreamElement) -> Result<Option<OutputElement>, ProcessError> {
        let s = state();
        let bytes = element.payload.read_all();
        let token = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;

        s.buffer.push_str(&token);

        // Detect sentence boundary.
        let trimmed = token.trim_end();
        if trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?') {
            let sentence = s.buffer.trim().to_string();
            s.buffer.clear();
            let seq = s.sentence_count;
            s.sentence_count += 1;
            return Ok(Some(emit_sentence(&sentence, seq)?));
        }

        Ok(None) // Absorb — sentence not yet complete.
    }

    fn flush() -> Result<Vec<OutputElement>, ProcessError> {
        let s = state();
        if s.buffer.trim().is_empty() {
            return Ok(vec![]);
        }
        // Emit any remaining partial sentence.
        let sentence = s.buffer.trim().to_string();
        s.buffer.clear();
        let seq = s.sentence_count;
        Ok(vec![emit_sentence(&sentence, seq)?])
    }
}

export!(SentenceAggregator);
```

**`components/output-sink/src/lib.rs`**

```rust
//! Output sink for the token-streaming pipeline.
//! Prints each assembled sentence.

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct OutputSink {
    received: u64,
}

static mut STATE: Option<OutputSink> = None;
fn state() -> &'static mut OutputSink {
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for OutputSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        unsafe { STATE = Some(OutputSink { received: 0 }); }
        Ok(())
    }
    fn teardown() { unsafe { STATE = None; } }
}

impl SinkGuest for OutputSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let s = state();
        let bytes = element.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;
        println!("[output-sink] sentence {}: {}", s.received, text);
        s.received += 1;
        Ok(BackpressureSignal::Ready)
    }
    fn complete() -> Result<(), ProcessError> {
        println!("[output-sink] Stream complete.");
        Ok(())
    }
}

export!(OutputSink);
```

## Pipeline Configuration

**`Torvyn.toml`**

```toml
[torvyn]
name = "token-streaming"
version = "0.1.0"
contract_version = "0.1.0"
description = "AI token streaming pipeline with content filtering"

[[component]]
name = "token-source"
path = "components/token-source"

[[component]]
name = "content-filter"
path = "components/content-filter"

[[component]]
name = "sentence-aggregator"
path = "components/sentence-aggregator"

[[component]]
name = "output-sink"
path = "components/output-sink"

[flow.main]
description = "Tokens → Filter → Aggregate → Output"

[flow.main.nodes.source]
component = "token-source"
interface = "torvyn:streaming/source"

[flow.main.nodes.filter]
component = "content-filter"
interface = "torvyn:filtering/filter"
config = "blocked_word"

[flow.main.nodes.aggregator]
component = "sentence-aggregator"
interface = "torvyn:aggregation/aggregator"

[flow.main.nodes.sink]
component = "output-sink"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "filter", port = "input" }

[[flow.main.edges]]
from = { node = "filter", port = "output" }
to = { node = "aggregator", port = "input" }

[[flow.main.edges]]
from = { node = "aggregator", port = "output" }
to = { node = "sink", port = "input" }
```

## Expected Output

```
$ torvyn run flow.main
[torvyn] Running flow 'main'

[output-sink] sentence 0: The quick brown fox jumped over the lazy dog.
[output-sink] sentence 1: The was filtered.
[output-sink] sentence 2: Torvyn handles streaming tokens.
[output-sink] Stream complete.

[torvyn] Flow 'main' completed.
[torvyn] 20 tokens produced | 1 filtered | 3 sentences emitted
```

Note that "blocked_word" was filtered out, so the second sentence reads "The was filtered." instead of "The blocked_word was filtered."

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| Filter overhead per token | < 10 us (no buffer allocation in filter) |
| Token-to-sentence latency | < 200 us (accumulation + emit) |
| Memory per aggregator | Proportional to longest sentence |

## Learn More

- [Architecture Guide: Filter Pattern](docs/architecture.md#filter-pattern) — Zero-allocation filtering
- [WIT Reference: `torvyn:filtering`](docs/wit-reference.md#torvyn-filtering)
- [Use Case Guide: AI Pipelines](docs/use-cases/ai-pipelines.md)
