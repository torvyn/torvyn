//! Echo sink: receives stream elements and reads each one's payload.
//! The `read_all()` call is what produces the per-element
//! `HostToComponent` copy event the runtime's invariant test counts —
//! a sink that ignored the payload would skip that copy and break the
//! "exactly 4 copies per element" assertion in the end-to-end test.

#[allow(warnings)]
mod bindings;

use bindings::exports::torvyn::streaming::{lifecycle, sink};
use bindings::torvyn::streaming::types::{BackpressureSignal, ProcessError, StreamElement};

struct EchoSink;

impl lifecycle::Guest for EchoSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        Ok(())
    }

    fn teardown() {}
}

impl sink::Guest for EchoSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        // Read the whole payload so the host records the
        // `HostToComponent` copy event. We don't care about the bytes —
        // the test asserts on the meta-level sequence and on the copy
        // ledger.
        let _ = element.payload.read_all();
        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        Ok(())
    }
}

bindings::export!(EchoSink with_types_in bindings);
