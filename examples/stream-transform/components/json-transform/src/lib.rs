//! JSON transform processor.
//!
//! Reads each JSON event, adds a "processed_at" timestamp field,
//! and renames "user" to "username". Demonstrates the processor
//! interface's ownership model: input is borrowed, output is owned.

wit_bindgen::generate!({
    world: "managed-transform",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::processor::Guest as ProcessorGuest;
use torvyn::streaming::buffer_allocator;
use torvyn::streaming::types::*;

struct JsonTransform;

static mut INITIALIZED: bool = false;

impl LifecycleGuest for JsonTransform {
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

impl ProcessorGuest for JsonTransform {
    fn process(input: StreamElement) -> Result<ProcessResult, ProcessError> {
        // Step 1: Read the input payload (borrowed buffer -> copy into
        // component linear memory). This is a measured copy.
        let input_bytes = input.payload.read_all();

        // Step 2: Parse and transform the JSON.
        // Using simple string manipulation to avoid pulling in serde.
        // In production code, use serde_json.
        let input_str = String::from_utf8(input_bytes)
            .map_err(|e| ProcessError::InvalidInput(format!("not UTF-8: {e}")))?;

        // Rename "user" -> "username" and add "processed_at".
        let transformed = input_str
            .replace("\"user\":", "\"username\":")
            .trim_end_matches('}')
            .to_string()
            + ",\"processed_at\":\"2025-01-15T10:30:00Z\"}";

        let out_bytes = transformed.as_bytes();

        // Step 3: Allocate a new output buffer from the host pool.
        let out_buf = buffer_allocator::allocate(out_bytes.len() as u64)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        out_buf.append(out_bytes)
            .map_err(|e| ProcessError::Internal(format!("{e:?}")))?;
        out_buf.set_content_type("application/json");

        // Step 4: Freeze and return. Ownership of the output buffer
        // transfers to the runtime via the OutputElement.
        let frozen = out_buf.freeze();

        Ok(ProcessResult::Emit(OutputElement {
            meta: ElementMeta {
                sequence: input.meta.sequence,
                timestamp_ns: 0,
                content_type: "application/json".to_string(),
            },
            payload: frozen,
        }))
    }
}

export!(JsonTransform);
