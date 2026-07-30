//! Polyglot real-Wasm end-to-end test.
//!
//! Builds a Source → Processor → Sink topology where the **source is a
//! TinyGo component** and the processor + sink remain the existing Rust
//! components. Drives it through the real [`WasmtimeEngine`],
//! [`WasmtimeInvoker`], and reactor coordinator.
//!
//! When this test passes, Torvyn's polyglot streaming-runtime claim is
//! operationally demonstrated: a Go-authored component participates as
//! a first-class peer in the same pipeline as Rust components, sharing
//! the same buffer pool, the same ownership state machine, and the
//! same `CopyLedger` accounting — with the "exactly 4 measured copies
//! per element" invariant preserved across the language boundary.

#![cfg(feature = "wasm-polyglot")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use torvyn_engine::{WasmtimeEngine, WasmtimeEngineConfig};
use torvyn_integration_tests::real_wasm::{
    await_flow_terminal, echo_sink_wasm, file_uri, go_echo_source_wasm, identity_processor_wasm,
    spawn_real_coordinator, wait_for_sink_count, RecordingInvoker, SINK_COMPONENT_ID,
};
use torvyn_pipeline::{instantiate_pipeline, NodeConfig, PipelineTopologyBuilder};
use torvyn_types::{ComponentRole, FlowState};

/// **Polyglot proof** — Go-source → Rust-processor → Rust-sink. Asserts
/// the same "exactly 4 measured copies per element" invariant that the
/// all-Rust pipeline satisfies, plus a per-stage breakdown that proves
/// the Go side hits the host's `write_payload` path on the manager's
/// flow ledger identically to the Rust side.
#[tokio::test]
async fn test_polyglot_go_source_to_rust_sink_records_four_copies_per_element() {
    const ELEMENT_COUNT: u64 = 10;
    const SEQ_BYTES: u64 = 8;

    let pushed = Arc::new(Mutex::new(Vec::<u64>::new()));
    let invoker: Arc<RecordingInvoker> = Arc::new(RecordingInvoker::new(
        SINK_COMPONENT_ID,
        Arc::clone(&pushed),
    ));

    let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default())
        .expect("WasmtimeEngine must initialise");
    let manager = engine.resource_manager();

    let source_cfg = NodeConfig {
        init_config: Some(format!("{{\"count\":{ELEMENT_COUNT}}}")),
        ..NodeConfig::default()
    };

    let topology = PipelineTopologyBuilder::new("polyglot-go-source")
        .add_node(
            "source",
            ComponentRole::Source,
            &file_uri(go_echo_source_wasm()),
            source_cfg,
        )
        .add_node(
            "processor",
            ComponentRole::Processor,
            &file_uri(identity_processor_wasm()),
            NodeConfig::default(),
        )
        .add_node(
            "sink",
            ComponentRole::Sink,
            &file_uri(echo_sink_wasm()),
            NodeConfig::default(),
        )
        .add_edge("source", "output", "processor", "input")
        .add_edge("processor", "output", "sink", "input")
        .build()
        .expect("polyglot 3-stage topology must build");

    let (reactor, _coordinator_join) =
        spawn_real_coordinator(Arc::clone(&invoker), engine.resource_manager());

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed for the polyglot topology");

    let final_state =
        await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(30)).await;
    assert_eq!(
        final_state,
        FlowState::Completed,
        "expected the polyglot flow to reach Completed; got {final_state:?}"
    );

    wait_for_sink_count(&pushed, ELEMENT_COUNT, Duration::from_secs(10)).await;

    let received = pushed.lock().expect("pushed_sequences mutex");
    assert_eq!(
        received.len() as u64,
        ELEMENT_COUNT,
        "sink received {} elements, expected {}",
        received.len(),
        ELEMENT_COUNT,
    );
    for window in received.windows(2) {
        assert!(
            window[0] < window[1],
            "sink-side sequences must be strictly monotonic; saw {:?} → {:?}",
            window[0],
            window[1],
        );
    }
    drop(received);

    // The headline polyglot guarantee: a Go-emitted buffer is indistinguishable
    // from a Rust-emitted buffer as far as the host's copy accounting is
    // concerned. Every copy is attributed to the flow's single reactor-assigned
    // id, exactly as in the all-Rust pipeline.
    let stats = manager.flow_copy_stats(handle.flow_id());

    assert_eq!(
        stats.total_copy_ops,
        4 * ELEMENT_COUNT,
        "polyglot pipeline must record exactly 4 copies per element (= {}); got {}",
        4 * ELEMENT_COUNT,
        stats.total_copy_ops,
    );
    assert_eq!(
        stats.total_payload_bytes,
        4 * SEQ_BYTES * ELEMENT_COUNT,
        "polyglot pipeline byte total expected {} (= 4 × {SEQ_BYTES} × {}); got {}",
        4 * SEQ_BYTES * ELEMENT_COUNT,
        ELEMENT_COUNT,
        stats.total_payload_bytes,
    );

    // CopyReason index 0 = HostToComponent, 1 = ComponentToHost. Two writes
    // (Go source output, Rust processor output) and two reads (processor input,
    // sink input) per element — the Go-emitted buffer is tagged identically to
    // a Rust-emitted one.
    assert_eq!(
        stats.copies_by_reason[1],
        2 * ELEMENT_COUNT,
        "expected {} ComponentToHost writes (Go source + Rust processor output); got {}",
        2 * ELEMENT_COUNT,
        stats.copies_by_reason[1],
    );
    assert_eq!(
        stats.copies_by_reason[0],
        2 * ELEMENT_COUNT,
        "expected {} HostToComponent reads (processor + sink input); got {}",
        2 * ELEMENT_COUNT,
        stats.copies_by_reason[0],
    );
}
