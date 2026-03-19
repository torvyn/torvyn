//! Hello World source component.
//!
//! Produces a configurable number of "Hello, World!" messages,
//! then signals stream exhaustion.

// Generate bindings from the WIT contract.
// This creates Rust types and traits matching the WIT interfaces.
wit_bindgen::generate!({
    world: "data-source",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::source::Guest as SourceGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::{BackpressureSignal, OutputElement, ElementMeta, ProcessError};

/// Component state. Holds configuration and tracks how many
/// messages have been produced.
struct HelloSource {
    /// Total messages to produce before signaling exhaustion.
    total_messages: u64,
    /// Messages produced so far.
    produced: u64,
}

// Global mutable state. In a Wasm component, there is exactly one
// instance of this state per component instantiation. The host
// guarantees no concurrent access (no reentrancy).
static mut STATE: Option<HelloSource> = None;

fn state() -> &'static mut HelloSource {
    // SAFETY: Wasm components are single-threaded and the host guarantees
    // no reentrancy, so mutable static access is safe here.
    unsafe { STATE.as_mut().expect("component not initialized") }
}

impl LifecycleGuest for HelloSource {
    /// Initialize the source. Accepts an optional JSON config string
    /// specifying `{"count": N}`. Defaults to 5 messages.
    fn init(config: String) -> Result<(), ProcessError> {
        let total = if config.is_empty() {
            5
        } else {
            // Simple manual parsing to avoid pulling in serde for a demo.
            // In production, use serde_json.
            config
                .trim()
                .strip_prefix("{\"count\":")
                .and_then(|s| s.strip_suffix('}'))
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(5)
        };

        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe {
            STATE = Some(HelloSource {
                total_messages: total,
                produced: 0,
            });
        }
        Ok(())
    }

    fn teardown() {
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
}

impl SourceGuest for HelloSource {
    /// Pull the next element. Returns None when all messages are produced.
    fn pull() -> Result<Option<OutputElement>, ProcessError> {
        let s = state();

        if s.produced >= s.total_messages {
            // Stream exhausted. The runtime will call complete() on
            // downstream sinks and transition the flow to Draining.
            return Ok(None);
        }

        // Format the message payload.
        let message = format!("Hello, World! (message {})", s.produced + 1);
        let payload_bytes = message.as_bytes();

        // Allocate a buffer from the host's buffer pool.
        // The host manages the memory — the component never directly
        // allocates host-side buffers.
        let mut_buf = buffer_allocator::allocate(payload_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("buffer allocation failed: {e:?}")))?;

        // Write the payload into the mutable buffer.
        mut_buf.append(payload_bytes)
            .map_err(|e| ProcessError::Internal(format!("buffer write failed: {e:?}")))?;

        // Set content type for downstream consumers.
        mut_buf.set_content_type("text/plain");

        // Freeze the mutable buffer into an immutable buffer.
        // Ownership of the buffer transfers to the runtime when
        // we return it inside the OutputElement.
        let frozen = mut_buf.freeze();

        s.produced += 1;

        Ok(Some(OutputElement {
            meta: ElementMeta {
                // The runtime overwrites sequence and timestamp-ns (per C01-4).
                // These values are advisory.
                sequence: s.produced - 1,
                timestamp_ns: 0,
                content_type: "text/plain".to_string(),
            },
            payload: frozen,
        }))
    }

    fn notify_backpressure(_signal: BackpressureSignal) {
        // This simple source ignores backpressure signals.
        // A production source would pause or slow its data generation.
    }
}

// Register the component with the Wasm component model.
export!(HelloSource);
