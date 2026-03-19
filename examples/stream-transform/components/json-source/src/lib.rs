//! JSON event source.
//!
//! Produces a sequence of JSON objects representing user events.
//! Each event has a "user" field and an "action" field.

wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct JsonSource {
    events: Vec<String>,
    index: usize,
}

static mut STATE: Option<JsonSource> = None;
fn state() -> &'static mut JsonSource {
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for JsonSource {
    fn init(_config: String) -> Result<(), ProcessError> {
        let events = vec![
            r#"{"user":"alice","action":"login"}"#.to_string(),
            r#"{"user":"bob","action":"purchase"}"#.to_string(),
            r#"{"user":"carol","action":"logout"}"#.to_string(),
        ];
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = Some(JsonSource { events, index: 0 }); }
        Ok(())
    }
    fn teardown() {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
}

impl SourceGuest for JsonSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();
        if s.index >= s.events.len() {
            return Ok(None);
        }
        let payload = s.events[s.index].as_bytes();
        let buf = buffer_allocator::allocate(payload.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(payload)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("application/json");
        let frozen = buf.freeze();
        s.index += 1;
        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: (s.index - 1) as u64,
                timestamp_ns: 0,
                content_type: "application/json".to_string(),
            },
            payload: frozen,
        }))
    }
    fn notify_backpressure(_signal: BackpressureSignal) {}
}

export!(JsonSource);
