//! Failing processor: fails after N elements.
//!
//! Config: `{ "fail_after": 50 }`

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../../crates/torvyn-contracts/wit/torvyn-streaming",
});

struct FailingProcessor;

thread_local! {
    static COUNT: RefCell<u64> = RefCell::new(0);
    static LIMIT: RefCell<u64> = RefCell::new(50);
}

impl Guest for FailingProcessor {
    fn init(config: String) -> Result<(), ProcessError> {
        let limit = config
            .trim()
            .strip_prefix("{\"fail_after\":")
            .and_then(|s| s.strip_suffix('}'))
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(50u64);
        LIMIT.with(|l| *l.borrow_mut() = limit);
        COUNT.with(|c| *c.borrow_mut() = 0);
        Ok(())
    }

    fn teardown() {}

    fn process(element: StreamElement) -> Result<ProcessResult, ProcessError> {
        let current = COUNT.with(|c| {
            let mut count = c.borrow_mut();
            *count += 1;
            *count
        });

        let limit = LIMIT.with(|l| *l.borrow());

        if current > limit {
            return Err(ProcessError::Fatal(format!(
                "intentional failure after {limit} elements"
            )));
        }

        Ok(ProcessResult::Emit(OutputElement {
            meta: element.meta,
            payload: element.payload,
        }))
    }
}

export!(FailingProcessor);
