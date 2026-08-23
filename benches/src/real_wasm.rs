//! Real-Wasm benchmark harness.
//!
//! Everything here drives genuine `cargo component`-built WebAssembly
//! Components through [`WasmtimeEngine`] and [`WasmtimeInvoker`] — the same
//! engine, invoker, resource manager, and flow driver the shipping runtime
//! uses. There is no mock anywhere in this module.
//!
//! # What the measured region covers
//!
//! Per element, a Source → Processor → Sink flow executes:
//!
//! 1. `source.pull` — a guest call that allocates a host buffer through the
//!    `buffer-allocator` import and writes `payload_bytes` into it
//!    (**copy 1**, `ComponentToHost`).
//! 2. `processor.process` — a guest call that reads the borrowed input
//!    (**copy 2**, `HostToComponent`), allocates a fresh buffer, and writes
//!    the bytes through (**copy 3**, `ComponentToHost`).
//! 3. `sink.push` — a guest call that reads the borrowed payload
//!    (**copy 4**, `HostToComponent`).
//!
//! plus the bounded-queue transfer and demand-driven scheduling between each
//! pair of stages, and the Canonical ABI marshalling on every one of those
//! six host/guest boundary crossings. A Source → Sink flow drops steps 2 and
//! its two copies, leaving two.
//!
//! That is the honest cost of the runtime. The mock-invoker benchmarks
//! elsewhere in this crate execute none of it.
//!
//! # What the measured region excludes
//!
//! Component compilation and instantiation. Wasmtime compilation is cached
//! by the engine, and per-flow instantiation is a startup cost, not a
//! per-element one — so [`WasmFixtures::build_flow`] is called outside the
//! timed region and instantiation is measured separately by the
//! `real_wasm_instantiation` group. Benchmarks that time the hot path must
//! use `iter_custom` and start the clock at [`RealFlow::run`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use torvyn_engine::{
    CompiledComponent, ComponentInstance, ComponentInvoker, ComponentLimits, WasiConfiguration,
    WasmEngine, WasmtimeEngine, WasmtimeEngineConfig, WasmtimeInvoker,
};
use torvyn_integration_tests::real_wasm::{
    echo_sink_wasm, echo_source_wasm, identity_processor_wasm,
};
use torvyn_integration_tests::{
    make_streams, ComponentId, ComponentRole, FlowCancellation, FlowCompletionStats, FlowConfig,
    FlowDriver, FlowId, FlowState, FlowTopology, NoopEventSink, ReactorEvent, StageDefinition,
    StreamConfig, StreamConnection,
};
use torvyn_resources::DefaultResourceManager;

/// Capacity of the per-flow reactor event channel.
///
/// The driver emits progress events with `try_send` and tolerates drops, so
/// this only needs to be large enough that the receiver — which the harness
/// holds but never drains — is not the thing under test. It matches the
/// coordinator's own channel capacity.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Payload sizes swept by the scaling benchmark, in bytes.
///
/// Chosen to land in distinct buffer-pool tiers so the sweep shows what the
/// tiering actually costs:
///
/// | Size    | Pool tier | Note                                          |
/// |---------|-----------|-----------------------------------------------|
/// | 8 B     | `Small`   | Just the sequence number — the minimum element |
/// | 256 B   | `Small`   | Tier capacity exactly; the comparison payload  |
/// | 4 KiB   | `Medium`  | Tier capacity exactly                          |
/// | 64 KiB  | `Large`   | Tier capacity exactly                          |
pub const PAYLOAD_SWEEP_BYTES: &[u64] = &[8, 256, 4 * 1024, 64 * 1024];

/// The shape of the pipeline under test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Source → Sink. Two measured copies per element.
    SourceSink,
    /// Source → Processor → Sink. Four measured copies per element — the
    /// canonical topology the project's copy-accounting invariant is stated
    /// against.
    SourceProcessorSink,
}

impl Shape {
    /// Number of payload copies the resource manager must record per
    /// element for this shape.
    #[must_use]
    pub const fn copies_per_element(self) -> u64 {
        match self {
            Self::SourceSink => 2,
            Self::SourceProcessorSink => 4,
        }
    }

    /// Number of streams (bounded queues) in this shape.
    ///
    /// `FlowCompletionStats::total_elements` sums the per-stream element
    /// counts rather than counting source elements, so a correct N-element
    /// run through this shape reports `N × stream_count()`, not `N`.
    #[must_use]
    pub const fn stream_count(self) -> u64 {
        match self {
            Self::SourceSink => 1,
            Self::SourceProcessorSink => 2,
        }
    }

    /// Stable identifier used in criterion benchmark names. Changing these
    /// invalidates stored baselines and `thresholds.json`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SourceSink => "source_sink",
            Self::SourceProcessorSink => "source_processor_sink",
        }
    }
}

/// One benchmark configuration: a pipeline shape, an element count, and a
/// payload size.
#[derive(Clone, Copy, Debug)]
pub struct RunSpec {
    /// Pipeline shape.
    pub shape: Shape,
    /// Number of elements the source produces before completing.
    pub elements: u64,
    /// Bytes in every element's payload.
    pub payload_bytes: u64,
}

impl RunSpec {
    /// A spec for `shape`, `elements`, and `payload_bytes`.
    #[must_use]
    pub const fn new(shape: Shape, elements: u64, payload_bytes: u64) -> Self {
        Self {
            shape,
            elements,
            payload_bytes,
        }
    }

    /// The `lifecycle.init` config string handed to the source component.
    #[must_use]
    fn source_init_config(&self) -> String {
        format!(
            "{{\"count\":{},\"payload_bytes\":{}}}",
            self.elements, self.payload_bytes
        )
    }

    /// Total copy operations a correct run must record.
    #[must_use]
    pub const fn expected_copy_ops(&self) -> u64 {
        self.shape.copies_per_element() * self.elements
    }

    /// Total copied payload bytes a correct run must record.
    #[must_use]
    pub const fn expected_payload_bytes(&self) -> u64 {
        self.expected_copy_ops() * self.payload_bytes
    }

    /// Value `FlowCompletionStats::total_elements` must report for a correct
    /// run — the per-stream counts summed, i.e. `elements × stream_count`.
    #[must_use]
    pub const fn expected_stream_elements(&self) -> u64 {
        self.elements * self.shape.stream_count()
    }
}

/// A compiled, ready-to-instantiate set of Wasm component fixtures sharing
/// one engine.
///
/// Construct this **once** per benchmark binary: the engine caches compiled
/// components, and reusing it means each `build_flow` pays only
/// instantiation, never compilation. The resource manager is likewise
/// shared, which is what makes the buffer pool warm — as it is in a
/// long-running host.
pub struct WasmFixtures {
    engine: WasmtimeEngine,
    invoker: Arc<WasmtimeInvoker>,
    source: CompiledComponent,
    processor: CompiledComponent,
    sink: CompiledComponent,
    next_flow_id: AtomicU64,
}

impl WasmFixtures {
    /// Read and compile the three Rust component fixtures.
    ///
    /// # Panics
    /// Panics if the engine cannot start, a fixture is missing from disk, or
    /// a fixture fails to compile. All three mean the benchmark cannot run
    /// at all, and a loud failure is the only honest outcome.
    #[must_use]
    pub fn new() -> Self {
        let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default())
            .expect("WasmtimeEngine must initialise");

        let source = compile(&engine, echo_source_wasm());
        let processor = compile(&engine, identity_processor_wasm());
        let sink = compile(&engine, echo_sink_wasm());

        Self {
            engine,
            invoker: Arc::new(WasmtimeInvoker::new()),
            source,
            processor,
            sink,
            // Flow ids start at 1; 0 is the engine's "unassigned" sentinel.
            next_flow_id: AtomicU64::new(1),
        }
    }

    /// The resource manager these fixtures allocate from. Copy accounting
    /// for any flow built here is queryable through it.
    #[must_use]
    pub fn resource_manager(&self) -> Arc<DefaultResourceManager> {
        self.engine.resource_manager()
    }

    /// Number of buffers the shared pool currently has outstanding.
    ///
    /// A cleanly drained pipeline returns every buffer, so this settles back
    /// to zero between runs. Used by [`Self::validate`] to prove the
    /// benchmark is not quietly leaking across iterations.
    #[must_use]
    pub fn live_buffers(&self) -> u32 {
        self.engine.resource_manager().live_resource_count()
    }

    /// Retire a finished flow: return anything it still holds to the pool
    /// and drop its copy-ledger entry.
    ///
    /// A benchmark runs the same configuration thousands of times against
    /// one shared resource manager. Without this the ledger's per-flow map
    /// would grow without bound for the length of the run, and the growth —
    /// not the runtime — would start showing up in the measurement. Call it
    /// **outside** the timed region, after any stats have been read.
    pub fn retire_flow(&self, flow_id: FlowId) {
        let _ = self
            .engine
            .resource_manager()
            .release_flow_resources(flow_id);
    }

    /// Instantiate a fresh flow for `spec`: new component instances, new
    /// `lifecycle.init`, new streams, new driver.
    ///
    /// This is the **untimed** part. Call it outside the measured region.
    ///
    /// # Panics
    /// Panics if instantiation or `lifecycle.init` fails.
    pub async fn build_flow(&self, spec: &RunSpec) -> RealFlow {
        let flow_id = FlowId::new(self.next_flow_id.fetch_add(1, Ordering::Relaxed));
        let topology = self.topology(spec);
        let config = FlowConfig::default_with_topology(topology.clone());
        let streams = make_streams(&topology, flow_id, &config);

        // Register the flow with the copy ledger *before* instantiating any
        // stage. `CopyLedger::record_copy` drops events for flows it has no
        // entry for, so registration must precede the first resource
        // operation — this mirrors `ReactorCoordinator::spawn_flow_with_
        // instances`, which registers before spawning the driver.
        self.engine.resource_manager().register_flow(flow_id);

        let mut instances = Vec::with_capacity(topology.stages.len());
        for stage in &topology.stages {
            instances.push(self.instantiate_stage(flow_id, stage).await);
        }

        let cancellation = FlowCancellation::new();
        let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            Arc::clone(&self.invoker),
            NoopEventSink,
            cancellation.clone(),
            event_tx,
        );

        RealFlow {
            flow_id,
            driver,
            _cancellation: cancellation,
            _events: event_rx,
        }
    }

    /// Run one flow for `spec` and assert every invariant the runtime
    /// promises: clean completion, the exact element count, the exact copy
    /// count and byte total for the shape, and a buffer pool that returns to
    /// zero outstanding buffers.
    ///
    /// Call this once per configuration *before* benchmarking it. A
    /// benchmark that reports a fast number for a run that silently dropped
    /// elements, skipped copies, or leaked buffers is worse than no
    /// benchmark, and this is what rules that out.
    ///
    /// # Panics
    /// Panics with a diagnostic on the first invariant that does not hold.
    pub async fn validate(&self, spec: &RunSpec) {
        let flow = self.build_flow(spec).await;
        let flow_id = flow.flow_id();
        let (state, stats) = flow.run().await;

        assert_eq!(
            state,
            FlowState::Completed,
            "flow for {spec:?} must reach Completed"
        );
        assert_eq!(
            stats.total_elements,
            spec.expected_stream_elements(),
            "flow for {spec:?} moved {} stream-elements, expected {} \
             ({} elements across {} stream(s))",
            stats.total_elements,
            spec.expected_stream_elements(),
            spec.elements,
            spec.shape.stream_count(),
        );

        let copies = self.engine.resource_manager().flow_copy_stats(flow_id);
        assert_eq!(
            copies.total_copy_ops,
            spec.expected_copy_ops(),
            "{spec:?} must record exactly {} copies ({} per element); got {}",
            spec.expected_copy_ops(),
            spec.shape.copies_per_element(),
            copies.total_copy_ops,
        );
        assert_eq!(
            copies.total_payload_bytes,
            spec.expected_payload_bytes(),
            "{spec:?} must copy exactly {} payload bytes; got {}",
            spec.expected_payload_bytes(),
            copies.total_payload_bytes,
        );

        // Deliberately checked *before* `retire_flow`: a healthy pipeline
        // returns every buffer as each element is consumed, so this must
        // already be zero without the terminal sweep having to rescue it.
        let live = self.live_buffers();
        assert_eq!(
            live, 0,
            "{spec:?} left {live} buffer(s) outstanding; a benchmark that leaks \
             buffers measures pool exhaustion, not the hot path",
        );

        self.retire_flow(flow_id);
    }

    /// Build the reactor topology for `spec`.
    fn topology(&self, spec: &RunSpec) -> FlowTopology {
        // Component ids mirror `instantiate_pipeline`: node index + 1.
        let mut stages = vec![StageDefinition {
            component_id: ComponentId::new(1),
            role: ComponentRole::Source,
            fuel_budget: None,
            config: spec.source_init_config(),
        }];
        if spec.shape == Shape::SourceProcessorSink {
            stages.push(StageDefinition {
                component_id: ComponentId::new(2),
                role: ComponentRole::Processor,
                fuel_budget: None,
                config: String::new(),
            });
        }
        let sink_idx = stages.len();
        stages.push(StageDefinition {
            component_id: ComponentId::new(sink_idx as u64 + 1),
            role: ComponentRole::Sink,
            fuel_budget: None,
            config: String::new(),
        });

        let connections = (0..sink_idx)
            .map(|from| StreamConnection {
                from_stage: from,
                to_stage: from + 1,
                config: StreamConfig::default(),
            })
            .collect();

        let topology = FlowTopology {
            stages,
            connections,
        };
        topology
            .validate()
            .expect("benchmark topology must be valid");
        topology
    }

    /// Instantiate one stage and run its `lifecycle.init`, mirroring
    /// `torvyn_pipeline::instantiate_pipeline` (including stamping the
    /// reactor-assigned `FlowId` onto the store, which is what attributes
    /// the stage's copies to this flow in the ledger).
    async fn instantiate_stage(
        &self,
        flow_id: FlowId,
        stage: &StageDefinition,
    ) -> ComponentInstance {
        let compiled = match stage.role {
            ComponentRole::Source => &self.source,
            ComponentRole::Processor => &self.processor,
            ComponentRole::Sink => &self.sink,
            other => panic!("benchmark topologies use no {other:?} stages"),
        };

        let mut instance = self
            .engine
            .instantiate(
                compiled,
                self.engine.default_imports(),
                stage.component_id,
                &WasiConfiguration::deny_all(),
                // Mirror the real pipeline: a stage's own budget, where it
                // sets one, is what the store is built with.
                &ComponentLimits {
                    fuel_budget: stage.fuel_budget,
                    max_memory_bytes: None,
                },
            )
            .await
            .expect("component instantiation must succeed");
        instance.set_flow_id(flow_id);

        self.invoker
            .invoke_init(&mut instance, stage.component_id, &stage.config)
            .await
            .expect("lifecycle.init must succeed");

        instance
    }
}

impl Default for WasmFixtures {
    fn default() -> Self {
        Self::new()
    }
}

/// An instantiated, ready-to-run real-Wasm flow.
///
/// [`run`](Self::run) is the measured region: start the clock immediately
/// before it and stop immediately after.
pub struct RealFlow {
    flow_id: FlowId,
    driver: FlowDriver<Arc<WasmtimeInvoker>, NoopEventSink>,
    /// Held so the driver's cancellation token stays valid for the run.
    _cancellation: FlowCancellation,
    /// Held so the driver's `try_send` event emissions have a live receiver,
    /// matching how the coordinator owns the channel in production.
    _events: mpsc::Receiver<ReactorEvent>,
}

impl RealFlow {
    /// The reactor-assigned flow id, for querying copy accounting.
    #[must_use]
    pub const fn flow_id(&self) -> FlowId {
        self.flow_id
    }

    /// Drive the flow to completion. **This is the measured region.**
    pub async fn run(self) -> (FlowState, FlowCompletionStats) {
        let (_id, state, stats) = self.driver.run().await;
        (state, stats)
    }
}

/// Time `iters` runs of `spec`, excluding per-iteration instantiation.
///
/// Returns the summed duration of the measured regions only — exactly the
/// contract criterion's `Bencher::iter_custom` expects. Every benchmark that
/// times the real-Wasm hot path should go through this, so all of them
/// measure the same region.
///
/// # Panics
/// Panics if a run fails or delivers the wrong number of elements. The check
/// happens after the clock stops, so it costs the measurement nothing and
/// stops a run that silently dropped work from contributing a fast sample.
pub async fn timed_runs(fixtures: &WasmFixtures, spec: RunSpec, iters: u64) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        // Untimed: instantiate fresh guests and run `lifecycle.init`.
        let flow = fixtures.build_flow(&spec).await;
        let flow_id = flow.flow_id();

        let start = Instant::now();
        let (state, stats) = flow.run().await;
        total += start.elapsed();

        assert_eq!(state, FlowState::Completed, "flow must complete cleanly");
        assert_eq!(
            stats.total_elements,
            spec.expected_stream_elements(),
            "flow moved {} stream-elements, expected {}",
            stats.total_elements,
            spec.expected_stream_elements(),
        );
        fixtures.retire_flow(flow_id);
    }
    total
}

/// Read and compile a component fixture from disk.
fn compile(engine: &WasmtimeEngine, path: &str) -> CompiledComponent {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| panic!("cannot read Wasm component fixture '{path}': {e}"));
    engine
        .compile_component(&bytes)
        .unwrap_or_else(|e| panic!("cannot compile Wasm component fixture '{path}': {e}"))
}
