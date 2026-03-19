//! Hello World sink component.
//!
//! Receives stream elements and prints their payload contents
//! to the component's standard output (captured by the host).

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::{BackpressureSignal, ProcessError, StreamElement};

struct HelloSink {
    received: u64,
}

static mut STATE: Option<HelloSink> = None;

fn state() -> &'static mut HelloSink {
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("component not initialized") }
}

impl LifecycleGuest for HelloSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe {
            STATE = Some(HelloSink { received: 0 });
        }
        Ok(())
    }

    fn teardown() {
        let s = state();
        // Print summary on teardown.
        println!("[hello-sink] Received {} messages total.", s.received);
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
}

impl SinkGuest for HelloSink {
    /// Receive a stream element and print its payload.
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let s = state();

        // Read the payload bytes from the borrowed buffer handle.
        // This copies data from host memory into component linear memory.
        // The resource manager records this copy for observability.
        let payload_bytes = element.payload.read_all();

        // Convert bytes to a UTF-8 string.
        let text = String::from_utf8(payload_bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("payload is not UTF-8: {e}")))?;

        println!("[hello-sink] seq={}: {}", element.meta.sequence, text);

        s.received += 1;

        // Signal that we are ready for the next element.
        // Returning BackpressureSignal::Pause would tell the runtime
        // to stop delivering elements until we are ready.
        Ok(BackpressureSignal::Ready)
    }

    /// Called when the upstream source is exhausted.
    fn complete() -> Result<(), ProcessError> {
        println!("[hello-sink] Stream complete.");
        Ok(())
    }
}

export!(HelloSink);
