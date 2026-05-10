//! Identity processor: reads the borrowed input payload, allocates a
//! fresh host buffer, writes the same bytes through, freezes, and
//! returns. This is the canonical 3-stage middle component that the
//! Source → Processor → Sink end-to-end test relies on to produce the
//! "exactly 4 measured copies per element" invariant — one
//! `HostToComponent` read of the source's buffer and one
//! `ComponentToHost` write into the new output buffer.

#[allow(warnings)]
mod bindings;

use bindings::exports::torvyn::streaming::processor;
use bindings::torvyn::streaming::buffer_allocator::allocate;
use bindings::torvyn::streaming::types::{OutputElement, ProcessError, ProcessResult, StreamElement};

struct IdentityProcessor;

impl processor::Guest for IdentityProcessor {
    fn process(element: StreamElement) -> Result<ProcessResult, ProcessError> {
        let bytes = element.payload.read_all();
        let cap = bytes.len() as u64;
        let cap_hint = cap.max(1);
        let mb = allocate(cap_hint).map_err(|_| ProcessError::Internal("allocate failed".into()))?;
        if !bytes.is_empty() {
            mb.write(0, &bytes)
                .map_err(|_| ProcessError::Internal("write failed".into()))?;
        }
        let buf = mb.freeze();
        Ok(ProcessResult::Emit(OutputElement {
            meta: element.meta,
            payload: buf,
        }))
    }
}

bindings::export!(IdentityProcessor with_types_in bindings);
