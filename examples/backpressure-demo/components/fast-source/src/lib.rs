//! Fast source -- produces elements as quickly as possible.
//! Emits 1,000 numbered elements, respecting backpressure signals.

wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct FastSource {
    total: u64,
    produced: u64,
    paused: bool,
}

static mut STATE: Option<FastSource> = None;
fn state() -> &'static mut FastSource {
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for FastSource {
    fn init(config: String) -> Result<(), ProcessError> {
        let total = config.trim().parse::<u64>().unwrap_or(1000);
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = Some(FastSource { total, produced: 0, paused: false }); }
        Ok(())
    }
    fn teardown() {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
}

impl SourceGuest for FastSource {
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();

        // Respect backpressure: if paused, return no element.
        // The runtime will poll again after backpressure clears.
        if s.paused {
            return Ok(None);
        }

        if s.produced >= s.total {
            return Ok(None);
        }

        let msg = format!("element-{}", s.produced);
        let bytes = msg.as_bytes();
        let buf = buffer_allocator::allocate(bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.append(bytes).map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        buf.set_content_type("text/plain");
        let frozen = buf.freeze();
        s.produced += 1;

        Ok(Some(OutputElement {
            meta: ElementMeta {
                sequence: s.produced - 1,
                timestamp_ns: 0,
                content_type: "text/plain".to_string(),
            },
            payload: frozen,
        }))
    }

    fn notify_backpressure(signal: BackpressureSignal) {
        let s = state();
        match signal {
            BackpressureSignal::Pause => {
                s.paused = true;
                // In a real source, you would also stop reading from
                // the external data source (network, file, etc.).
            }
            BackpressureSignal::Ready => {
                s.paused = false;
            }
        }
    }
}

export!(FastSource);
