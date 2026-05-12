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
    await_flow_terminal, file_uri, spawn_real_coordinator, wait_for_sink_count, RecordingInvoker,
    SINK_COMPONENT_ID, STAGE_FLOW_IDS,
};
use torvyn_pipeline::{instantiate_pipeline, NodeConfig, PipelineTopologyBuilder};
use torvyn_types::{ComponentRole, FlowState};

// ===========================================================================
// Component fixture paths — populated by build.rs at compile time
// ===========================================================================

const GO_ECHO_SOURCE_WASM: &str = env!("TORVYN_GO_ECHO_SOURCE_WASM");
const IDENTITY_PROCESSOR_WASM: &str = env!("TORVYN_IDENTITY_PROCESSOR_WASM");
const ECHO_SINK_WASM: &str = env!("TORVYN_ECHO_SINK_WASM");

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
            &file_uri(GO_ECHO_SOURCE_WASM),
            source_cfg,
        )
        .add_node(
            "processor",
            ComponentRole::Processor,
            &file_uri(IDENTITY_PROCESSOR_WASM),
            NodeConfig::default(),
        )
        .add_node(
            "sink",
            ComponentRole::Sink,
            &file_uri(ECHO_SINK_WASM),
            NodeConfig::default(),
        )
        .add_edge("source", "output", "processor", "input")
        .add_edge("processor", "output", "sink", "input")
        .build()
        .expect("polyglot 3-stage topology must build");

    let (reactor, _coordinator_join) = spawn_real_coordinator(Arc::clone(&invoker));

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

    let source_stats = manager.flow_copy_stats(STAGE_FLOW_IDS[0]);
    let processor_stats = manager.flow_copy_stats(STAGE_FLOW_IDS[1]);
    let sink_stats = manager.flow_copy_stats(STAGE_FLOW_IDS[2]);

    // The headline polyglot guarantee: a Go-emitted buffer is
    // indistinguishable from a Rust-emitted buffer as far as the
    // host's copy accounting is concerned.
    assert_eq!(
        source_stats.total_copy_ops, ELEMENT_COUNT,
        "Go source flow expected {ELEMENT_COUNT} ComponentToHost writes; got {}",
        source_stats.total_copy_ops,
    );
    assert_eq!(
        source_stats.copies_by_reason[1], ELEMENT_COUNT,
        "Go source's writes must be tagged as ComponentToHost, identical to the Rust pipeline",
    );
    assert_eq!(
        source_stats.total_payload_bytes,
        SEQ_BYTES * ELEMENT_COUNT,
        "Go source's payload byte total expected {} (= {SEQ_BYTES} × {}); got {}",
        SEQ_BYTES * ELEMENT_COUNT,
        ELEMENT_COUNT,
        source_stats.total_payload_bytes,
    );

    assert_eq!(
        processor_stats.total_copy_ops,
        2 * ELEMENT_COUNT,
        "processor flow expected {} copy ops (read + write per element); got {}",
        2 * ELEMENT_COUNT,
        processor_stats.total_copy_ops,
    );
    assert_eq!(
        sink_stats.total_copy_ops, ELEMENT_COUNT,
        "sink flow expected {ELEMENT_COUNT} HostToComponent reads; got {}",
        sink_stats.total_copy_ops,
    );

    let total_ops =
        source_stats.total_copy_ops + processor_stats.total_copy_ops + sink_stats.total_copy_ops;
    assert_eq!(
        total_ops,
        4 * ELEMENT_COUNT,
        "polyglot pipeline total copy ops expected {} (= 4 per element × {}); got {total_ops}",
        4 * ELEMENT_COUNT,
        ELEMENT_COUNT,
    );

    let total_bytes = source_stats.total_payload_bytes
        + processor_stats.total_payload_bytes
        + sink_stats.total_payload_bytes;
    assert_eq!(
        total_bytes,
        4 * SEQ_BYTES * ELEMENT_COUNT,
        "polyglot pipeline byte total expected {} (= 4 × {SEQ_BYTES} × {}); got {total_bytes}",
        4 * SEQ_BYTES * ELEMENT_COUNT,
        ELEMENT_COUNT,
    );
}
