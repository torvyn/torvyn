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
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for TokenSource {
    fn init(_config: String) -> Result<(), ProcessError> {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = Some(TokenSource { index: 0 }); }
        Ok(())
    }
    fn teardown() {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
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
