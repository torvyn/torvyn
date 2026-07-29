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
    await_flow_terminal, file_uri, spawn_real_coordinator, wait_for_sink_count,
    wait_for_zero_live_buffers, RecordingInvoker, SINK_COMPONENT_ID,
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

/// **Regression** — a long-running pipeline must keep processing after a
/// component's *cumulative* fuel consumption exceeds `default_fuel`.
///
/// `default_fuel` is documented as a budget **per invocation**, but Wasmtime
/// never replenishes fuel on its own (its async yield interval yields without
/// adding fuel). Before the invoker refuelled each guest call, the initial
/// allocation therefore acted as a per-*lifetime* cap: this very pipeline
/// delivered only ~1,600 of its elements and then died with `Trap::OutOfFuel`,
/// silently dropping the rest. The element count here is deliberately an order
/// of magnitude past that cliff, so a regression fails loudly.
#[tokio::test]
async fn test_long_running_pipeline_survives_cumulative_fuel_use() {
    const ELEMENT_COUNT: u64 = 20_000;

    let pushed = Arc::new(Mutex::new(Vec::<u64>::new()));
    let invoker: Arc<RecordingInvoker> = Arc::new(RecordingInvoker::new(
        SINK_COMPONENT_ID,
        Arc::clone(&pushed),
    ));

    let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default())
        .expect("WasmtimeEngine must initialise");

    let topology = build_pipeline_topology(
        "e2e-long-running-fuel",
        Some(format!("{{\"count\":{ELEMENT_COUNT}}}")),
    );

    let (reactor, _coordinator_join) =
        spawn_real_coordinator(Arc::clone(&invoker), engine.resource_manager());

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed");

    let final_state =
        await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(120)).await;
    assert_eq!(
        final_state,
        FlowState::Completed,
        "the long-running flow must drain cleanly; got {final_state:?}",
    );

    // The real regression signal: every element reaches the sink. Fuel
    // exhaustion manifests as a partial delivery, not an error here.
    wait_for_sink_count(&pushed, ELEMENT_COUNT, Duration::from_secs(30)).await;
    let delivered = pushed.lock().expect("pushed mutex").len() as u64;
    assert_eq!(
        delivered, ELEMENT_COUNT,
        "sink received {delivered} of {ELEMENT_COUNT} elements; a shortfall means \
         components ran out of fuel mid-flow",
    );
}

/// **Test B** — The headline invariant: with a real Source → Processor
/// → Sink flow, the `CopyLedger` must record **exactly four** measured
/// copy events per element, all attributed to the flow's single
/// reactor-assigned `FlowId` (the reactor stamps it onto every component's
/// store before the driver runs). The per-reason breakdown pins the model:
///   - 2 `ComponentToHost` writes per element (source output, processor output)
///   - 2 `HostToComponent` reads  per element (processor input, sink input)
/// Total = 4N ops, 4 × 8 × N = 32N payload bytes.
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

    // Every copy is attributed to the flow's single reactor-assigned id (the
    // reactor stamps it onto each component's store before the driver runs),
    // so the whole pipeline's copy accounting lives under one ledger entry.
    let stats = manager.flow_copy_stats(handle.flow_id());

    assert_eq!(
        stats.total_copy_ops,
        4 * ELEMENT_COUNT,
        "pipeline must record exactly 4 copies per element (= {}); got {}",
        4 * ELEMENT_COUNT,
        stats.total_copy_ops,
    );
    assert_eq!(
        stats.total_payload_bytes,
        4 * SEQ_BYTES * ELEMENT_COUNT,
        "pipeline byte total expected {} (= 4 × {SEQ_BYTES} × {}); got {}",
        4 * SEQ_BYTES * ELEMENT_COUNT,
        ELEMENT_COUNT,
        stats.total_payload_bytes,
    );

    // CopyReason index 0 = HostToComponent, 1 = ComponentToHost. The four
    // copies per element are two reads (processor input, sink input) and two
    // writes (source output, processor output). Together with the total above,
    // this also pins CrossComponent and PoolReturn copies to zero.
    assert_eq!(
        stats.copies_by_reason[0],
        2 * ELEMENT_COUNT,
        "expected {} HostToComponent reads (processor + sink input); got {}",
        2 * ELEMENT_COUNT,
        stats.copies_by_reason[0],
    );
    assert_eq!(
        stats.copies_by_reason[1],
        2 * ELEMENT_COUNT,
        "expected {} ComponentToHost writes (source + processor output); got {}",
        2 * ELEMENT_COUNT,
        stats.copies_by_reason[1],
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

    let stats = manager.flow_copy_stats(handle.flow_id());
    assert_eq!(
        stats.total_copy_ops, 0,
        "the flow must record zero copy events when no elements are produced; got {}",
        stats.total_copy_ops,
    );
}

// ===========================================================================
// Host-driven end-to-end: start a real pipeline through `TorvynHost`
// ===========================================================================

/// Build the canonical source → processor → sink `FlowDef` used by the
/// host-driven end-to-end tests, with the source configured to emit
/// `element_count` elements.
fn host_e2e_flow(element_count: u64) -> torvyn_config::FlowDef {
    use std::collections::BTreeMap;
    use torvyn_config::{EdgeDef, EdgeEndpoint, FlowDef, NodeDef};

    fn node(component: String, interface: &str, init: Option<String>) -> NodeDef {
        NodeDef {
            component,
            interface: interface.to_owned(),
            config: init,
            ..NodeDef::default()
        }
    }
    fn edge(from_node: &str, to_node: &str) -> EdgeDef {
        EdgeDef {
            from: EdgeEndpoint {
                node: from_node.to_owned(),
                port: "output".to_owned(),
            },
            to: EdgeEndpoint {
                node: to_node.to_owned(),
                port: "input".to_owned(),
            },
            queue_depth: None,
            backpressure: None,
        }
    }

    let mut nodes = BTreeMap::new();
    nodes.insert(
        "source".to_owned(),
        node(
            file_uri(ECHO_SOURCE_WASM),
            "torvyn:streaming/source",
            Some(format!("{{\"count\":{element_count}}}")),
        ),
    );
    nodes.insert(
        "processor".to_owned(),
        node(
            file_uri(IDENTITY_PROCESSOR_WASM),
            "torvyn:streaming/processor",
            None,
        ),
    );
    nodes.insert(
        "sink".to_owned(),
        node(file_uri(ECHO_SINK_WASM), "torvyn:streaming/sink", None),
    );

    FlowDef {
        nodes,
        edges: vec![edge("source", "processor"), edge("processor", "sink")],
        ..FlowDef::default()
    }
}

/// The host runtime starts a real source → processor → sink pipeline from a
/// flow definition — the same path `torvyn run` uses — and drives it to
/// completion. Unlike the tests above (which call `instantiate_pipeline`
/// directly), this exercises `TorvynHost::start_flow`, which resolves the flow
/// definition, builds the topology, and instantiates it through the reactor.
#[tokio::test]
async fn test_host_start_flow_runs_real_pipeline_to_completion() {
    use torvyn_host::HostBuilder;

    const ELEMENT_COUNT: u64 = 50;

    let mut host = HostBuilder::new()
        .with_flow_definition("e2e", host_e2e_flow(ELEMENT_COUNT))
        .build()
        .await
        .expect("host must build");

    let flow_id = host.start_flow("e2e").await.expect("flow must start");

    // Poll the host until the flow reaches a terminal state.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let final_state = loop {
        let state = host
            .flow_state(flow_id)
            .await
            .expect("the started flow must be known to the host");
        if state.is_terminal() {
            break state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "flow did not reach a terminal state within 30s (last: {state:?})",
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    assert_eq!(
        final_state,
        FlowState::Completed,
        "the host-driven pipeline must complete cleanly",
    );

    // Observability: the run must have recorded metrics for this flow through
    // the collector the host wires into the reactor as its event sink. Before
    // this wiring the host installed a `NoopEventSink` and recorded nothing.
    // Metrics are retained post-terminal, so the snapshot is available after
    // completion.
    let snapshot = host
        .observability()
        .snapshot(flow_id)
        .expect("a completed flow must have recorded metrics");

    // `on_flow_start` registered all three stages (source, processor, sink).
    assert_eq!(
        snapshot.components.len(),
        3,
        "all three pipeline stages must be registered for observation",
    );
    // Every stage was actually invoked and observed.
    assert!(
        snapshot.components.iter().all(|c| c.invocations > 0),
        "every stage must record at least one invocation: {:?}",
        snapshot
            .components
            .iter()
            .map(|c| c.invocations)
            .collect::<Vec<_>>(),
    );
    // Element-level recording happened, with no invocation errors on a clean run.
    assert!(
        snapshot.elements_total > 0,
        "the pipeline processed {ELEMENT_COUNT} elements but recorded zero invocations",
    );
    assert_eq!(
        snapshot.errors_total, 0,
        "a clean run must record no invocation errors",
    );
    // Data-copy accounting reaches the collector too: the host wires it as the
    // resource manager's copy sink, and the reactor stamps the real flow id
    // onto every store, so all copies land under this flow — exactly four per
    // element (2 reads + 2 writes). A zero here would mean copies never reached
    // the collector; an undercount would mean they were misattributed.
    assert_eq!(
        snapshot.copies_total,
        4 * ELEMENT_COUNT,
        "the collector must record 4 copies per element (= {}); got {}",
        4 * ELEMENT_COUNT,
        snapshot.copies_total,
    );

    let _ = host.shutdown().await;
}

/// `TorvynHost::run` — the loop behind `torvyn run` — must return on its own
/// when a finite pipeline completes, without any external signal. Before flow
/// completion was detected, `run()` blocked on the shutdown signal and hung
/// forever on a finite pipeline. The wrapping timeout is the assertion: if
/// `run()` fails to detect completion, it never returns and the timeout fires.
#[tokio::test]
async fn test_host_run_returns_on_finite_pipeline_completion() {
    use torvyn_host::{HostBuilder, HostStatus};

    const ELEMENT_COUNT: u64 = 30;

    let mut host = HostBuilder::new()
        .with_flow_definition("e2e", host_e2e_flow(ELEMENT_COUNT))
        .build()
        .await
        .expect("host must build");

    // run() starts the flow, waits for it to reach a terminal state, shuts
    // down, and returns — all without a signal.
    tokio::time::timeout(Duration::from_secs(30), host.run())
        .await
        .expect("run() must return once the finite pipeline completes")
        .expect("run() must complete without error");

    assert_eq!(
        host.status(),
        HostStatus::Stopped,
        "run() must leave the host stopped after completion",
    );

    // Confirm run() returned because the pipeline genuinely completed (not a
    // spurious early exit): its recorded metrics show elements were processed.
    let flows = host.list_flows().await;
    assert_eq!(flows.len(), 1, "exactly one flow should have been started");
    let snapshot = host
        .observability()
        .snapshot(flows[0].flow_id)
        .expect("the completed flow must have recorded metrics");
    assert!(
        snapshot.elements_total > 0,
        "run() returned but the pipeline recorded no processed elements",
    );
    assert_eq!(
        snapshot.errors_total, 0,
        "a clean run must record no invocation errors",
    );
}

/// Exercises the exact sequence `torvyn run` uses after the CLI fix: start only
/// the selected flow, wait for it via `wait_for_all_flows`, then read its
/// recorded metrics for the run summary. Verifies the flow runs exactly once
/// (no double-start) and that the snapshot yields the real numbers the CLI
/// reports — elements processed, zero errors, and the stage/edge counts.
#[tokio::test]
async fn test_host_run_single_flow_pattern_reports_real_metrics() {
    use torvyn_host::HostBuilder;

    const ELEMENT_COUNT: u64 = 40;

    let mut host = HostBuilder::new()
        .with_flow_definition("e2e", host_e2e_flow(ELEMENT_COUNT))
        .build()
        .await
        .expect("host must build");

    // Start ONLY the selected flow (no `run()`, which would start every
    // configured flow and double-start this one).
    let flow_id = host.start_flow("e2e").await.expect("flow must start");

    // Wait for just this flow to finish.
    tokio::time::timeout(Duration::from_secs(30), host.wait_for_all_flows())
        .await
        .expect("the finite flow must reach a terminal state");

    // Exactly one flow ran — the selected one — with no accidental second start.
    let flows = host.list_flows().await;
    assert_eq!(
        flows.len(),
        1,
        "the selected flow must run exactly once, not be double-started",
    );

    let _ = host.shutdown().await;

    // The run summary the CLI builds is sourced entirely from this snapshot.
    let snapshot = host
        .observability()
        .snapshot(flow_id)
        .expect("a completed flow must have recorded metrics");
    assert!(
        snapshot.elements_total > 0,
        "the run summary would report zero elements processed",
    );
    assert_eq!(snapshot.errors_total, 0, "a clean run reports no errors");
    assert_eq!(
        snapshot.components.len(),
        3,
        "component_count in the summary comes from the recorded stages",
    );
    assert_eq!(
        snapshot.streams.len(),
        2,
        "edge_count in the summary comes from the recorded stream connections",
    );

    // End-to-end latency is recorded at the sink from each element's
    // pipeline-entry timestamp (preserved across the processor), so the
    // flow-level latency percentiles `torvyn bench` reports are populated and
    // monotonically ordered — not a dead metric.
    assert!(
        snapshot.latency_p50_ns > 0,
        "end-to-end latency must be recorded (was a dead metric before)",
    );
    // Percentiles from the same histogram are monotonically non-decreasing.
    // (Percentile-vs-max is not asserted: bucketed percentiles are boundary
    // estimates that can exceed the true max for small sample counts.)
    assert!(snapshot.latency_p50_ns <= snapshot.latency_p90_ns);
    assert!(snapshot.latency_p90_ns <= snapshot.latency_p95_ns);
    assert!(snapshot.latency_p95_ns <= snapshot.latency_p99_ns);
    assert!(snapshot.latency_max_ns > 0, "max latency must be recorded");
}

/// **Test G** — the ownership invariant: after a real Source → Processor
/// → Sink flow runs to completion, the resource manager must hold **zero**
/// live buffers.
///
/// This is the end-to-end statement of the claim that the host owns and
/// accounts for every buffer. It is asserted here, against real Wasm and
/// the real reactor, because no unit test can reach the sequence that
/// breaks it: a buffer is only orphaned once it has been *transferred*
/// from a component to the host on its way downstream, and that transfer
/// happens inside `WasmtimeInvoker` on the hot path.
///
/// Two distinct leaks are covered:
///
/// 1. **Per element.** The consumer of an element receives a borrow, so no
///    guest-side drop ever fires for it, and the buffer is owned by the
///    host rather than by any component, so component-keyed cleanup cannot
///    see it either. Without the invoker reclaiming each consumed element,
///    this grows by one buffer per element per stage for the whole life of
///    the flow — unbounded on a long-running stream.
/// 2. **At terminal.** Anything still queued when the flow ends is only
///    reachable through the flow index, which the coordinator's
///    flow-keyed sweep walks.
///
/// A element count well above the pool's tier size is used deliberately:
/// if buffers were not being recycled, the run would be visibly
/// accumulating rather than steady-state.
#[tokio::test]
async fn test_real_pipeline_leaks_no_buffers() {
    const ELEMENT_COUNT: u64 = 100;

    let pushed = Arc::new(Mutex::new(Vec::<u64>::new()));
    let invoker: Arc<RecordingInvoker> = Arc::new(RecordingInvoker::new(
        SINK_COMPONENT_ID,
        Arc::clone(&pushed),
    ));

    let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default())
        .expect("WasmtimeEngine must initialise");
    let manager = engine.resource_manager();

    let topology = build_pipeline_topology(
        "e2e-no-buffer-leak",
        Some(format!("{{\"count\":{ELEMENT_COUNT}}}")),
    );

    let (reactor, _coordinator_join) =
        spawn_real_coordinator(Arc::clone(&invoker), engine.resource_manager());

    let handle = instantiate_pipeline(&topology, &engine, &invoker, &reactor)
        .await
        .expect("instantiate_pipeline must succeed");

    let final_state =
        await_flow_terminal(&reactor, handle.flow_id(), Duration::from_secs(30)).await;
    assert_eq!(final_state, FlowState::Completed);

    wait_for_sink_count(&pushed, ELEMENT_COUNT, Duration::from_secs(5)).await;

    // Every buffer allocated over the run must have returned to the pool.
    wait_for_zero_live_buffers(&manager, Duration::from_secs(10)).await;

    // The copy ledger is retained through terminal reclamation, so the
    // flow's accounting is still readable — and it confirms the run really
    // did move data rather than completing early with nothing to free.
    let stats = manager.flow_copy_stats(handle.flow_id());
    assert_eq!(
        stats.total_copy_ops,
        4 * ELEMENT_COUNT,
        "the flow must have actually processed {ELEMENT_COUNT} elements",
    );
}
