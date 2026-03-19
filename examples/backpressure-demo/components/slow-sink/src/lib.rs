//! Slow sink -- simulates a slow consumer by introducing a delay
//! per element via busy-waiting (since WASI sleep is not available
//! in all environments).

wit_bindgen::generate!({
    world: "data-sink",
    path: "../../wit/torvyn-streaming",
});

use exports::torvyn::streaming::lifecycle::Guest as LifecycleGuest;
use exports::torvyn::streaming::sink::Guest as SinkGuest;
use torvyn::streaming::types::*;

struct SlowSink {
    received: u64,
    delay_iterations: u64,
}

static mut STATE: Option<SlowSink> = None;
fn state() -> &'static mut SlowSink {
    // SAFETY: Single-threaded Wasm component, no reentrancy.
    unsafe { STATE.as_mut().expect("not initialized") }
}

impl LifecycleGuest for SlowSink {
    fn init(config: String) -> Result<(), ProcessError> {
        // delay_iterations controls how slow the sink is.
        // Higher = slower. Calibrate for your hardware.
        let delay = config.trim().parse::<u64>().unwrap_or(100_000);
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = Some(SlowSink { received: 0, delay_iterations: delay }); }
        Ok(())
    }
    fn teardown() {
        let s = state();
        println!("[slow-sink] Processed {} elements total.", s.received);
        // SAFETY: Single-threaded Wasm component, no reentrancy.
        unsafe { STATE = None; }
    }
}

impl SinkGuest for SlowSink {
    fn push(element: StreamElement) -> Result<BackpressureSignal, ProcessError> {
        let s = state();

        // Simulate slow processing with a busy loop.
        // In a real sink, this delay would come from I/O (database writes,
        // network calls, disk writes, etc.).
        let mut acc: u64 = 0;
        for i in 0..s.delay_iterations {
            acc = acc.wrapping_add(i);
        }
        // Prevent the optimizer from removing the loop.
        if acc == u64::MAX { println!("{acc}"); }

        let bytes = element.payload.read_all();
        let text = String::from_utf8_lossy(&bytes);
        if s.received % 100 == 0 {
            println!("[slow-sink] seq={}: {} (every 100th logged)", element.meta.sequence, text);
        }

        s.received += 1;
        Ok(BackpressureSignal::Ready)
    }

    fn complete() -> Result<(), ProcessError> {
        println!("[slow-sink] Stream complete.");
        Ok(())
    }
}

export!(SlowSink);
