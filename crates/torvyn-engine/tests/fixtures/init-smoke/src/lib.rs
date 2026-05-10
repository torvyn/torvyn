//! Minimal `data-sink` component used as a torvyn-engine integration-test
//! fixture. Implements `lifecycle.init` (the only export the bindgen smoke
//! test exercises in Session 2.1) and stubs out `sink.push` / `sink.complete`
//! / `lifecycle.teardown`.

#[allow(warnings)]
mod bindings;

use bindings::exports::torvyn::streaming::{lifecycle, sink};
use bindings::torvyn::streaming::types::{BackpressureSignal, ProcessError, StreamElement};

struct InitSmoke;

impl lifecycle::Guest for InitSmoke {
    fn init(_config: String) -> Result<(), ProcessError> {
        Ok(())
    }

    fn teardown() {}
}

impl sink::Guest for InitSmoke {
    fn push(_element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        Ok(())
    }
}

bindings::export!(InitSmoke with_types_in bindings);
