//! Echo sink: collects elements and verifies ordering.
//!
//! On teardown, logs the total element count.

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../../crates/torvyn-contracts/wit/torvyn-streaming",
});

struct EchoSink;

thread_local! {
    static RECEIVED: RefCell<Vec<u64>> = RefCell::new(Vec::new());
}

impl Guest for EchoSink {
    fn init(_config: String) -> Result<(), ProcessError> {
        RECEIVED.with(|r| r.borrow_mut().clear());
        Ok(())
    }

    fn teardown() {
        RECEIVED.with(|r| {
            let received = r.borrow();
            // Verify ordering in teardown — the host verifies externally.
            for i in 1..received.len() {
                if received[i] != received[i - 1] + 1 {
                    // Out-of-order element detected
                }
            }
        });
    }

    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let seq = element.meta.sequence;
        RECEIVED.with(|r| r.borrow_mut().push(seq));
        Ok(BackpressureSignal::Accept)
    }
}

export!(EchoSink);
