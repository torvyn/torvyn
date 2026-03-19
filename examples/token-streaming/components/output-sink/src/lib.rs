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
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for OutputSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = Some(OutputSink { received: 0 }); }
        Ok(())
    }
    fn teardown() {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
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
