//! Echo source: produces N numbered elements through the real
//! buffer-allocator path so the host's `DefaultResourceManager` sees one
//! `ComponentToHost` copy event per element (8 bytes — the sequence
//! number encoded little-endian).
//!
//! Configuration (JSON string via `lifecycle.init`):
//! ```json
//! { "count": 100 }
//! ```
//! Empty config falls back to 1000.

#[allow(warnings)]
mod bindings;

use std::cell::RefCell;

use bindings::exports::torvyn::streaming::{lifecycle, source};
use bindings::torvyn::streaming::buffer_allocator::allocate;
use bindings::torvyn::streaming::types::{
    BackpressureSignal, ElementMeta, OutputElement, ProcessError,
};

struct EchoSource;

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { remaining: 0, sequence: 0 }) };
}

struct State {
    remaining: u64,
    sequence: u64,
}

impl lifecycle::Guest for EchoSource {
    fn init(config: String) -> Result<(), ProcessError> {
        let count: u64 = if config.is_empty() {
            1000
        } else {
            // Minimal JSON parser sufficient for `{"count":N}` and `{}`.
            config
                .trim()
                .strip_prefix("{\"count\":")
                .and_then(|s| s.strip_suffix('}'))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(1000)
        };

        STATE.with(|s| {
            let mut state = s.borrow_mut();
            state.remaining = count;
            state.sequence = 0;
        });

        Ok(())
    }

    fn teardown() {}
}

impl source::Guest for EchoSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let (seq, has_more) = STATE.with(|s| {
            let mut state = s.borrow_mut();
            if state.remaining == 0 {
                return (0, false);
            }
            let seq = state.sequence;
            state.sequence += 1;
            state.remaining -= 1;
            (seq, true)
        });
        if !has_more {
            return Ok(None);
        }

        let mb = allocate(8).map_err(|_| ProcessError::Internal("allocate failed".into()))?;
        mb.write(0, &seq.to_le_bytes())
            .map_err(|_| ProcessError::Internal("write failed".into()))?;
        let buf = mb.freeze();

        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: seq,
                timestamp_ns: 0,
                content_type: "application/octet-stream".into(),
            },
            payload: buf,
        }))
    }

    fn notify_backpressure(_signal: BackpressureSignal) {}
}

bindings::export!(EchoSource with_types_in bindings);
