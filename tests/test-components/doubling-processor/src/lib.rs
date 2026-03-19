//! Doubling processor: doubles each byte in the payload.

wit_bindgen::generate!({
    world: "transform",
    path: "../../../crates/torvyn-contracts/wit/torvyn-streaming",
});

struct DoublingProcessor;

impl Guest for DoublingProcessor {
    fn process(element: StreamElement) -> Result<ProcessResult, ProcessError> {
        let doubled: Vec<u8> = element
            .payload
            .iter()
            .map(|&b| b.wrapping_mul(2))
            .collect();

        Ok(ProcessResult::Emit(OutputElement {
            meta: element.meta,
            payload: doubled,
        }))
    }
}

export!(DoublingProcessor);
