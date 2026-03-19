//! Identity processor: passes elements through unchanged.

wit_bindgen::generate!({
    world: "transform",
    path: "../../../crates/torvyn-contracts/wit/torvyn-streaming",
});

struct IdentityProcessor;

impl Guest for IdentityProcessor {
    fn process(element: StreamElement) -> Result<ProcessResult, ProcessError> {
        Ok(ProcessResult::Emit(OutputElement {
            meta: element.meta,
            payload: element.payload,
        }))
    }
}

export!(IdentityProcessor);
