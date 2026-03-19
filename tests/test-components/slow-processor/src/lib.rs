//! Slow processor: adds configurable delay (via fuel consumption spin loop).
//!
//! Config: `{ "spin_iterations": 10000 }`

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../../crates/torvyn-contracts/wit/torvyn-streaming",
});

struct SlowProcessor;

thread_local! {
    static SPIN: RefCell<u64> = RefCell::new(10_000);
}

impl Guest for SlowProcessor {
    fn init(config: String) -> Result<(), ProcessError> {
        let iterations = config
            .trim()
            .strip_prefix("{\"spin_iterations\":")
            .and_then(|s| s.strip_suffix('}'))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(10_000u64);
        SPIN.with(|s| *s.borrow_mut() = iterations);
        Ok(())
    }

    fn teardown() {}

    fn process(element: StreamElement) -> Result<ProcessResult, ProcessError> {
        // Spin to consume fuel and simulate slow processing
        let iterations = SPIN.with(|s| *s.borrow());
        let mut x: u64 = 0;
        for i in 0..iterations {
            x = x.wrapping_add(i);
        }
        // Use x to prevent optimization
        let _ = std::hint::black_box(x);

        Ok(ProcessResult::Emit(OutputElement {
            meta: element.meta,
            payload: element.payload,
        }))
    }
}

export!(SlowProcessor);
