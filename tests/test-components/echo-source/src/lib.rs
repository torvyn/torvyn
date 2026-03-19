//! Echo source: produces numbered elements for testing.
//!
//! Configuration (JSON string via lifecycle.init):
//! ```json
//! { "count": 1000 }
//! ```

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "data-source",
    path: "../../../crates/torvyn-contracts/wit/torvyn-streaming",
});

struct EchoSource;

thread_local! {
    static STATE: RefCell<EchoSourceState> = RefCell::new(EchoSourceState {
        remaining: 0,
        sequence: 0,
    });
}

struct EchoSourceState {
    remaining: u64,
    sequence: u64,
}

impl Guest for EchoSource {
    fn init(config: String) -> Result<(), ProcessError> {
        let count: u64 = if config.is_empty() {
            1000 // default
        } else {
            config
                .trim()
                .strip_prefix("{\"count\":")
                .and_then(|s| s.strip_suffix('}'))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(1000)
        };

        STATE.with(|s| {
            let mut state = s.borrow_mut();
            state.remaining = count;
            state.sequence = 0;
        });

        Ok(())
    }

    fn teardown() {
        // Nothing to clean up
    }

    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if state.remaining == 0 {
                return Ok(None); // Stream complete
            }

            let seq = state.sequence;
            state.sequence += 1;
            state.remaining -= 1;

            // Produce a payload: the sequence number as 8 bytes (little-endian)
            let payload = seq.to_le_bytes().to_vec();

            Ok(Some(OutputElement {
                meta: ElementMeta {
                    sequence: seq,
                    timestamp_ns: 0,
                    content_type: "application/octet-stream".into(),
                },
                payload,
            }))
        })
    }
}

export!(EchoSource);
