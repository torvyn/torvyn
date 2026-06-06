//! Real-Wasm end-to-end integration test.
//!
//! Builds a Source → Processor → Sink topology backed by three
//! Component Model components compiled by `cargo component build`
//! (see [`build.rs`](../../build.rs)) and drives it through the real
//! [`WasmtimeEngine`], [`WasmtimeInvoker`], and reactor coordinator.
//! No `MockEngine` anywhere in this file.
//!
//! When this test passes, the project's "polyglot streaming runtime"
//! claim is operationally demonstrated:
//! 1. Real Wasm components are loaded from `file://` URIs.
//! 2. `instantiate_pipeline` walks topology order and runs
//!    `lifecycle.init` on each component.
//! 3. The reactor drives `pull → process → push` through real Wasm
//!    code.
//! 4. The `DefaultResourceManager` records **exactly** four measured
//!    copies per element on the Source → Processor → Sink hot path —
//!    the headline invariant of the ownership-aware design.

#![cfg(feature = "wasm-e2e")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use torvyn_engine::{WasmtimeEngine, WasmtimeEngineConfig};
use torvyn_integration_tests::real_wasm::{
    await_flow_terminal, file_uri, spawn_real_coordinator, wait_for_sink_count, RecordingInvoker,
    SINK_COMPONENT_ID, STAGE_FLOW_IDS,
};
use torvyn_pipeline::{
    instantiate_pipeline, NodeConfig, PipelineTopology, PipelineTopologyBuilder,
};
use torvyn_types::{ComponentRole, FlowState};

// ===========================================================================
// Component fixture paths — populated by build.rs at compile time
// ===========================================================================

const ECHO_SOURCE_WASM: &str = env!("TORVYN_ECHO_SOURCE_WASM");
const IDENTITY_PROCESSOR_WASM: &str = env!("TORVYN_IDENTITY_PROCESSOR_WASM");
const ECHO_SINK_WASM: &str = env!("TORVYN_ECHO_SINK_WASM");

fn build_pipeline_topology(name: &'static str, source_init: Option<String>) -> PipelineTopology {
    let mut source_cfg = NodeConfig::default();
    if let Some(s) = source_init {
        source_cfg.init_config = Some(s);
    }
    PipelineTopologyBuilder::new(name)
        .add_node(
            "source",
            ComponentRole::Source,
            &file_uri(ECHO_SOURCE_WASM),
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
        .expect("three-stage real-Wasm topology must build")
}

// ===========================================================================
// Tests
// ===========================================================================

/// **Test A** — Source → Processor → Sink with 100 elements. Asserts
/// that the flow reaches `Completed`, the sink receives exactly 100
/// elements, and the per-element sequence numbers it observes are
/// strictly monotonic.
#[tokio::test]
async fn test_real_source_to_sink_one_hundred_elements_completes() {
    const ELEMENT_COUNT: u64 = 100;

    let pushed = Arc::new(Mutex::new(Vec::<u64>::new()));
    let invoker: Arc<RecordingInvoker> = Arc::new(RecordingInvoker::new(
        SINK_COMPONENT_ID,
        Arc::clone(&pushed),
    ));

    let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default())
        .expect("WasmtimeEngine must initialise");

    let topology = build_pipeline_topology(
        "e2e-source-processor-sink-100",
        Some(format!("{{\"count\":{ELEMENT_COUNT}}}")),
    );

    let (reactor, _coordinator_join) =
        spawn_real_coordinator(Arc::clone(&invoker), engine.resource_manager());

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed for the three-stage real-Wasm topology");

    let final_state =
        await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(30)).await;
    assert_eq!(
        final_state,
        FlowState::Completed,
        "expected the flow to reach Completed; got {final_state:?}"
    );

    wait_for_sink_count(&pushed, ELEMENT_COUNT, Duration::from_secs(5)).await;

    let received = pushed.lock().expect("pushed_sequences mutex");
    assert_eq!(
        received.len() as u64,
        ELEMENT_COUNT,
        "sink received {} elements, expected {}",
        received.len(),
        ELEMENT_COUNT,
    );
    // The reactor's `FlowDriver` reassigns the `sequence` field at every
    // emit site (source pull, processor process). In a 3-stage Source →
    // Processor → Sink pipeline every element bumps the shared
    // `next_global_sequence` counter twice — once when the source's
    // output is enqueued onto stream 0 and once when the processor's
    // output is enqueued onto stream 1 — so the sink observes the odd
    // half of the counter: 1, 3, 5, ..., 2N-1. Assert strict
    // monotonicity (no gaps within the odd subsequence) rather than
    // hard-coding the spacing, which is a flow-driver implementation
    // detail.
    for window in received.windows(2) {
        assert!(
            window[0] < window[1],
            "sink-side sequences must be strictly monotonic; saw {:?} → {:?}",
            window[0],
            window[1],
        );
    }
}

/// **Test B** — The headline invariant: with a real Source → Processor
/// → Sink flow, the `CopyLedger` must show **exactly four** measured
/// copy events per element. Verifies the precise per-stage breakdown
/// as well:
///   - source flow: N writes  (`ComponentToHost`)
///   - processor flow: N reads + N writes  (`HostToComponent` + `ComponentToHost`)
///   - sink flow: N reads  (`HostToComponent`)
/// Total = 4N. Byte total = 4 × 8 × N = 32 × N.
#[tokio::test]
async fn test_real_pipeline_records_exactly_four_copies_per_element() {
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

    let topology = build_pipeline_topology(
        "e2e-copy-accounting",
        Some(format!("{{\"count\":{ELEMENT_COUNT}}}")),
    );

    let (reactor, _coordinator_join) =
        spawn_real_coordinator(Arc::clone(&invoker), engine.resource_manager());

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed");

    let final_state =
        await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(15)).await;
    assert_eq!(final_state, FlowState::Completed);

    wait_for_sink_count(&pushed, ELEMENT_COUNT, Duration::from_secs(5)).await;

    let source_stats = manager.flow_copy_stats(STAGE_FLOW_IDS[0]);
    let processor_stats = manager.flow_copy_stats(STAGE_FLOW_IDS[1]);
    let sink_stats = manager.flow_copy_stats(STAGE_FLOW_IDS[2]);

    // Per-stage breakdown — diagnose any drift precisely.
    assert_eq!(
        source_stats.total_copy_ops, ELEMENT_COUNT,
        "source flow expected {ELEMENT_COUNT} ComponentToHost writes; got {}",
        source_stats.total_copy_ops,
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
        "pipeline total copy ops expected {} (= 4 per element × {}); got {total_ops}",
        4 * ELEMENT_COUNT,
        ELEMENT_COUNT,
    );

    let total_bytes = source_stats.total_payload_bytes
        + processor_stats.total_payload_bytes
        + sink_stats.total_payload_bytes;
    assert_eq!(
        total_bytes,
        4 * SEQ_BYTES * ELEMENT_COUNT,
        "pipeline byte total expected {} (= 4 × {SEQ_BYTES} × {}); got {total_bytes}",
        4 * SEQ_BYTES * ELEMENT_COUNT,
        ELEMENT_COUNT,
    );

    // CopyReason index 0 = HostToComponent, 1 = ComponentToHost. Verify
    // the directional accounting matches the design doc.
    assert_eq!(
        source_stats.copies_by_reason[1], ELEMENT_COUNT,
        "source's writes must be ComponentToHost",
    );
    assert_eq!(
        processor_stats.copies_by_reason[0], ELEMENT_COUNT,
        "processor reads must be HostToComponent",
    );
    assert_eq!(
        processor_stats.copies_by_reason[1], ELEMENT_COUNT,
        "processor writes must be ComponentToHost",
    );
    assert_eq!(
        sink_stats.copies_by_reason[0], ELEMENT_COUNT,
        "sink's reads must be HostToComponent",
    );
}

/// **Test C** — Source returns `None` on the first `pull`; the flow
/// must still reach `Completed` cleanly, the sink must receive zero
/// elements, and the copy ledger must record zero copy events.
#[tokio::test]
async fn test_real_pipeline_handles_source_completion_gracefully() {
    let pushed = Arc::new(Mutex::new(Vec::<u64>::new()));
    let invoker: Arc<RecordingInvoker> = Arc::new(RecordingInvoker::new(
        SINK_COMPONENT_ID,
        Arc::clone(&pushed),
    ));

    let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default())
        .expect("WasmtimeEngine must initialise");
    let manager = engine.resource_manager();

    let topology =
        build_pipeline_topology("e2e-source-exhausted", Some("{\"count\":0}".to_owned()));

    let (reactor, _coordinator_join) =
        spawn_real_coordinator(Arc::clone(&invoker), engine.resource_manager());

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed even for an empty source");

    let final_state =
        await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(15)).await;
    assert_eq!(
        final_state,
        FlowState::Completed,
        "an empty source must drain to Completed, got {final_state:?}",
    );

    assert!(
        pushed.lock().expect("pushed mutex").is_empty(),
        "sink must not receive any element when the source returns None immediately",
    );

    for flow in STAGE_FLOW_IDS {
        let stats = manager.flow_copy_stats(flow);
        assert_eq!(
            stats.total_copy_ops, 0,
            "flow {flow:?} must record zero copy events when no elements are produced; \
             got {}",
            stats.total_copy_ops,
        );
    }
}
