# Token Streaming Pipeline

An AI-oriented pipeline: a simulated LLM token source emits tokens one at a time, a content policy filter rejects tokens matching a block list, a token aggregator collects tokens into complete sentences, and an output sink writes the assembled text.

## What It Demonstrates

- The `torvyn:filtering/filter` interface for accept/reject decisions
- Token-by-token streaming (granular elements)
- Aggregation with sentence-boundary detection
- AI pipeline composition pattern
- Filter components that inspect data without allocating buffers

## Pipeline Topology

```
token-source --> content-filter --> sentence-aggregator --> output-sink
```

The source emits 20 tokens simulating LLM output. The content filter drops tokens matching a block list (e.g., "blocked_word"). The sentence aggregator collects tokens until a sentence-ending punctuation mark is detected, then emits the complete sentence. The sink prints each sentence.

## Prerequisites

- Rust toolchain (1.75+)
- `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
- `cargo-component`: `cargo install cargo-component`
- Torvyn CLI: `cargo install torvyn-cli`

## Build

```bash
make build
```

## Run

```bash
make run
```

## Expected Output

```
[output-sink] sentence 0: The quick brown fox jumped over the lazy dog.
[output-sink] sentence 1: The was filtered.
[output-sink] sentence 2: Torvyn handles streaming tokens.
[output-sink] Stream complete.
```

Note that "blocked_word" was filtered out, so the second sentence reads "The was filtered." instead of "The blocked_word was filtered."

## Key Concepts

### Zero-Allocation Filtering

The content filter uses the `torvyn:filtering/filter` interface. Its `evaluate()` function returns a simple `bool` -- it does not allocate any output buffers. This makes it the most efficient component type in the pipeline:

1. Read the token from the borrowed buffer (one measured copy)
2. Compare against the block list
3. Return `true` (pass) or `false` (reject)

The runtime handles forwarding or dropping the element based on the result.

### Sentence Aggregation

The sentence aggregator uses the `torvyn:aggregation/aggregator` interface:

- `ingest()`: Appends each token to an internal buffer. When a sentence-ending punctuation mark is detected, emits the complete sentence.
- `flush()`: Emits any remaining partial sentence when the stream ends.

### Four-Stage Pipeline

This example demonstrates a realistic four-component pipeline combining different interface types (source, filter, aggregator, sink), each with distinct ownership semantics.

## Performance Characteristics (Design Targets)

| Metric | Target |
|--------|--------|
| Filter overhead per token | < 10 us (no buffer allocation in filter) |
| Token-to-sentence latency | < 200 us (accumulation + emit) |
| Memory per aggregator | Proportional to longest sentence |

## Learn More

- [Architecture Guide: Filter Pattern](../../docs/architecture.md#filter-pattern)
- [WIT Reference: `torvyn:filtering`](../../docs/wit-reference.md#torvyn-filtering)
- [Use Case Guide: AI Pipelines](../../docs/use-cases/ai-pipelines.md)
