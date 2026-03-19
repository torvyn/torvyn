//! Content policy filter.
//!
//! Uses the torvyn:filtering/filter interface to accept or reject tokens.
//! This component type is extremely efficient: it does not allocate output
//! buffers. It reads the token to inspect it, then returns a boolean.
//! The runtime forwards or drops the element based on the result.

#[allow(warnings)]
mod bindings;

use bindings::exports::torvyn::filtering::filter::Guest as FilterGuest;
use bindings::exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use bindings::torvyn::streaming::types::*;

struct ContentFilter {
    blocked_words: Vec<String>,
    filtered_count: u64,
}

static mut STATE: Option<ContentFilter> = None;
fn state() -> &'static mut ContentFilter {
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for ContentFilter {
    fn init(config: String) -> Result<(), ProcessError> {
        // Config: comma-separated list of blocked words.
        let blocked: Vec<String> = if config.is_empty() {
            vec!["blocked_word".to_string()]
        } else {
            config.split(',').map(|s| s.trim().to_string()).collect()
        };
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe {
            STATE = Some(ContentFilter {
                blocked_words: blocked,
                filtered_count: 0,
            });
        }
        Ok(())
    }
    fn teardown() {
        let s = state();
        if s.filtered_count > 0 {
            println!(
                "[content-filter] Filtered {} token(s) during this flow.",
                s.filtered_count
            );
        }
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
}

impl FilterGuest for ContentFilter {
    /// Evaluate whether a token passes the content policy.
    ///
    /// - true: token passes through to downstream.
    /// - false: token is dropped by the runtime (no output buffer allocated).
    ///
    /// The filter reads the borrowed buffer to inspect the token contents.
    /// This is a single measured copy. No output buffer is allocated.
    fn evaluate(element: StreamElement) -> Result<bool, ProcessError> {
        let s = state();
        let bytes = element.payload.read_all();
        let token = String::from_utf8_lossy(&bytes);
        let trimmed = token.trim();

        for blocked in &s.blocked_words {
            if trimmed.eq_ignore_ascii_case(blocked) {
                s.filtered_count += 1;
                return Ok(false);
            }
        }
        Ok(true)
    }
}

bindings::export!(ContentFilter with_types_in bindings);
