//! JSON output rendering.
//!
//! Emits structured JSON to stdout. Used when `--format json` is specified.

use serde::Serialize;

/// Print a serializable value as JSON to stdout.
///
/// COLD PATH — called once per command.
pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json_str) => {
            println!("{json_str}");
        }
        Err(e) => {
            eprintln!("error: Failed to serialize output to JSON: {e}");
            println!(r#"{{"error": "serialization_failed", "detail": "{}"}}"#, e);
        }
    }
}

/// Print a value as NDJSON (newline-delimited JSON).
///
/// COLD PATH — called per progress event during long operations.
#[allow(dead_code)]
pub fn print_ndjson<T: Serialize>(value: &T) {
    match serde_json::to_string(value) {
        Ok(json_str) => {
            println!("{json_str}");
        }
        Err(e) => {
            eprintln!("error: Failed to serialize event to JSON: {e}");
        }
    }
}
