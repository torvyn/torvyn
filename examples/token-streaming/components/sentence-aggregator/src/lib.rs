//! Token-to-sentence aggregator.
//!
//! Collects streaming tokens into complete sentences. A sentence boundary
//! is detected when a token ends with '.', '!', or '?'. When a boundary
//! is reached, the accumulated text is emitted as a single output element.
//! Any remaining text is emitted on flush().

#[allow(warnings)]
mod bindings;

use bindings::exports::torvyn::aggregation::aggregator::Guest as AggregatorGuest;
use bindings::exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use bindings::torvyn::streaming::buffer_allocator;
use bindings::torvyn::streaming::types::*;

struct SentenceAggregator {
    buffer: String,
    sentence_count: u64,
}

static mut STATE: Option<SentenceAggregator> = None;
fn state() -> &'static mut SentenceAggregator {
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for SentenceAggregator {
    fn init(_config: String) -> Result<(), ProcessError> {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe {
            STATE = Some(SentenceAggregator {
                buffer: String::new(),
                sentence_count: 0,
            });
        }
        Ok(())
    }
    fn teardown() {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
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

        Ok(None) // Absorb -- sentence not yet complete.
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

bindings::export!(SentenceAggregator with_types_in bindings);
