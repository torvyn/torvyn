//! JSON sink -- prints each transformed JSON event.

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct JsonSink;
static mut INITIALIZED: bool = false;

impl LifecycleGuest for JsonSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { INITIALIZED = true; }
        Ok(())
    }
    fn teardown() {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { INITIALIZED = false; }
    }
}

impl SinkGuest for JsonSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let bytes = element.payload.read_all();
        let text = String::from_utf8(bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("{e}")))?;
        println!("[json-sink] seq={}: {}", element.meta.sequence, text);
        Ok(BackpressureSignal::Ready)
    }
    fn complete() -> Result<(), ProcessError> {
        println!("[json-sink] Stream complete.");
        Ok(())
    }
}

export!(JsonSink);
