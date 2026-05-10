//! Side-by-side comparison: Torvyn Source → Sink vs. gRPC unary on localhost.
//!
//! This benchmark answers the headline question for prospective users:
//! *"How does Torvyn's per-element overhead compare to a minimal in-process
//! gRPC transport on the same machine?"*
//!
//! Both arms use:
//!
//! - The same Tokio runtime (single shared `Runtime` for the whole bench
//!   group, to avoid runtime-construction noise).
//! - The same payload size (256 bytes per element).
//! - The same element counts (100, 1 000, 10 000) — chosen to span the range
//!   where setup overhead, steady-state work, and tail latency all become
//!   relevant in turn.
//! - Identical criterion configuration (`sample_size`, `measurement_time`).
//! - Identical correctness guarantees: every element must succeed; assertion
//!   failures will fail the benchmark loudly.
//!
//! ## Methodology choices documented inline
//!
//! - The gRPC server is spawned **once** per benchmark group, with a random
//!   `127.0.0.1` port. The shared server isolates the comparison to
//!   per-call transport cost (HTTP/2 framing, protobuf encode/decode) rather
//!   than measuring server startup, which would dominate small-N counts.
//! - The gRPC client `Channel` is also constructed **once** per arm and
//!   cloned per iteration. `Channel` is internally `Arc`-shared, so cloning
//!   is the canonical way to get an independent client handle for an
//!   iteration without paying for a fresh TCP/HTTP-2 handshake.
//! - Torvyn's arm follows the existing `latency.rs` pattern: `build_driver`
//!   is invoked inside `iter`, mirroring the cost model of the rest of
//!   `benches/benches/`. (We are measuring "do the work", not "amortize the
//!   driver across N runs".)
//! - We do **not** install a custom payload-bytes value via the user's
//!   `TestInvoker`: the inner work of the Torvyn arm is dominated by queue
//!   transfer + invoker hop, not payload movement, just like the existing
//!   benchmarks. The 256-byte payload is meaningful for the gRPC arm
//!   (HTTP/2 framing + protobuf encode/decode scales with payload size) and
//!   harmless for the Torvyn arm.
//!
//! ## Comparison interpretation
//!
//! The benchmark is **transport-only** — neither arm runs real WebAssembly.
//! Torvyn's arm uses the mock invoker, the same as the rest of
//! `benches/benches/`; gRPC's arm uses an echo handler. The takeaway is the
//! cost ratio of *Torvyn's per-element overhead* vs *gRPC localhost transport
//! per call*. Adding real Wasm to either side is a separate, larger task
//! (Item 2 of the project's Tier-1 plan).

use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::{Request, Response, Status};

use torvyn_integration_tests::{
    build_driver, conn, sink, source, FlowConfig, FlowId, FlowState, FlowTopology, TestInvoker,
};

// Generated tonic / prost code for the Echo service. The lints below are for
// the auto-generated module only — the workspace-level `clippy::all = "deny"`
// would otherwise reject derive-macro output that the benchmark does not own.
#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::restriction,
    missing_docs,
    rustdoc::all,
    unused
)]
mod echo_proto {
    tonic::include_proto!("torvyn.bench.echo");
}

use echo_proto::{
    echo_client::EchoClient,
    echo_server::{Echo, EchoServer},
    Payload,
};

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Payload size used by both arms. Chosen to be comfortably above the
/// gRPC framing-overhead floor while still matching realistic small-element
/// streaming workloads.
const PAYLOAD_BYTES: usize = 256;

/// Element counts used for the latency bench group. 100 captures the
/// startup-dominated regime, 1 000 the work-dominated regime, 10 000 the
/// throughput-dominated regime.
const LATENCY_ELEMENT_COUNTS: &[u64] = &[100, 1_000, 10_000];

/// Element count for the throughput bench (single point — criterion's
/// `Throughput::Elements` already reports elements/second).
const THROUGHPUT_ELEMENT_COUNT: u64 = 10_000;

// ---------------------------------------------------------------------------
// gRPC server
// ---------------------------------------------------------------------------

/// Trivial echo handler. Returns the request unchanged. Server-side cost is
/// dominated by transport (HTTP/2 framing, protobuf encode/decode) rather
/// than application logic — exactly what we want as the "transport baseline."
struct EchoService;

#[tonic::async_trait]
impl Echo for EchoService {
    async fn process(&self, req: Request<Payload>) -> Result<Response<Payload>, Status> {
        Ok(Response::new(req.into_inner()))
    }
}

/// Spawn an in-process Tonic server on a random `127.0.0.1` port. Returns
/// the bound `host:port` URI string and a oneshot sender that, when fired,
/// cleanly shuts the server down.
///
/// The server runs on the supplied runtime as an independent task. Binding
/// is done synchronously (`std::net::TcpListener::bind`) so the bound port
/// is known before this function returns and the caller does not need to
/// poll/sleep before connecting.
fn spawn_grpc_server(rt: &Runtime) -> (String, oneshot::Sender<()>) {
    // Bind synchronously *outside* the runtime so the bound port is known
    // before this function returns and the caller does not need to poll.
    // The conversion to a Tokio listener must happen inside a Tokio context,
    // which we get by deferring it to the spawned task body.
    let std_listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0 must succeed");
    std_listener
        .set_nonblocking(true)
        .expect("listener nonblocking");
    let addr = std_listener.local_addr().expect("listener local_addr");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    rt.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(std_listener)
            .expect("convert std listener to tokio listener inside the Tokio runtime");
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        Server::builder()
            .add_service(EchoServer::new(EchoService))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("Tonic server crashed");
    });

    let endpoint = format!("http://{addr}");
    (endpoint, shutdown_tx)
}

/// Connect a single gRPC `Channel` to the running server. Cloning the
/// returned `Channel` shares the underlying connection (it is internally
/// `Arc`-backed by Tonic).
fn connect_grpc_channel(rt: &Runtime, endpoint: &str) -> Channel {
    rt.block_on(async move {
        Endpoint::from_shared(endpoint.to_owned())
            .expect("valid endpoint URI")
            .connect_timeout(Duration::from_secs(5))
            .connect()
            .await
            .expect("gRPC client must connect")
    })
}

// ---------------------------------------------------------------------------
// Torvyn arm helpers
// ---------------------------------------------------------------------------

fn torvyn_source_sink_topology() -> FlowTopology {
    FlowTopology {
        stages: vec![source(1), sink(2)],
        connections: vec![conn(0, 1)],
    }
}

// ---------------------------------------------------------------------------
// Latency bench group: side-by-side per-element latency
// ---------------------------------------------------------------------------

fn bench_latency_torvyn_vs_grpc(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let (endpoint, shutdown_tx) = spawn_grpc_server(&rt);
    let channel = connect_grpc_channel(&rt, &endpoint);

    let mut group = c.benchmark_group("source_to_sink_vs_grpc_unary_localhost");
    // Match the existing `latency.rs` configuration so reports are visually
    // aligned.
    group.sample_size(50);
    group.measurement_time(Duration::from_secs(10));

    for &count in LATENCY_ELEMENT_COUNTS {
        // --- Torvyn arm ---
        group.bench_with_input(BenchmarkId::new("torvyn", count), &count, |b, &count| {
            b.to_async(&rt).iter(|| async move {
                let invoker = TestInvoker::new(count);
                let topology = torvyn_source_sink_topology();
                topology
                    .validate()
                    .expect("topology must be valid for the bench");
                let config = FlowConfig::default_with_topology(topology.clone());
                let flow_id = FlowId::new(1);

                let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
                let (_id, state, stats) = driver.run().await;

                assert_eq!(state, FlowState::Completed);
                assert_eq!(stats.total_elements, count);
            });
        });

        // --- gRPC arm ---
        let payload_bytes = Arc::new(vec![0u8; PAYLOAD_BYTES]);
        let channel_for_arm = channel.clone();
        group.bench_with_input(BenchmarkId::new("grpc", count), &count, |b, &count| {
            let payload_bytes = Arc::clone(&payload_bytes);
            let channel_for_arm = channel_for_arm.clone();
            b.to_async(&rt).iter(move || {
                let mut client = EchoClient::new(channel_for_arm.clone());
                let payload_bytes = Arc::clone(&payload_bytes);
                async move {
                    for seq in 0..count {
                        let request = Request::new(Payload {
                            data: payload_bytes.as_ref().clone(),
                            sequence: seq,
                        });
                        let resp = client
                            .process(request)
                            .await
                            .expect("gRPC unary call must succeed");
                        // Echo invariant: server returns the same sequence we sent.
                        assert_eq!(resp.into_inner().sequence, seq);
                    }
                }
            });
        });
    }

    group.finish();

    // Tear down the server cleanly. Ignore the result — if the receiver was
    // already dropped (server already shutting down) we have nothing to do.
    let _ = shutdown_tx.send(());
}

// ---------------------------------------------------------------------------
// Throughput bench group: side-by-side elements/second
// ---------------------------------------------------------------------------

fn bench_throughput_torvyn_vs_grpc(c: &mut Criterion) {
    let rt = Runtime::new().expect("Tokio runtime");
    let (endpoint, shutdown_tx) = spawn_grpc_server(&rt);
    let channel = connect_grpc_channel(&rt, &endpoint);

    let mut group = c.benchmark_group("throughput_vs_grpc_unary_localhost");
    // Match the existing `throughput.rs` configuration.
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(15));
    group.throughput(criterion::Throughput::Elements(THROUGHPUT_ELEMENT_COUNT));

    // --- Torvyn arm ---
    group.bench_function("torvyn_source_sink", |b| {
        b.to_async(&rt).iter(|| async {
            let invoker = TestInvoker::new(THROUGHPUT_ELEMENT_COUNT);
            let topology = torvyn_source_sink_topology();
            topology.validate().expect("topology must be valid");
            let config = FlowConfig::default_with_topology(topology.clone());
            let flow_id = FlowId::new(1);

            let (driver, _cancel, _rx) = build_driver(invoker, flow_id, topology, config).await;
            let (_id, state, stats) = driver.run().await;

            assert_eq!(state, FlowState::Completed);
            assert_eq!(stats.total_elements, THROUGHPUT_ELEMENT_COUNT);
        });
    });

    // --- gRPC arm ---
    {
        let payload_bytes = Arc::new(vec![0u8; PAYLOAD_BYTES]);
        let channel_for_arm = channel.clone();
        group.bench_function("grpc_unary_localhost", |b| {
            let payload_bytes = Arc::clone(&payload_bytes);
            let channel_for_arm = channel_for_arm.clone();
            b.to_async(&rt).iter(move || {
                let mut client = EchoClient::new(channel_for_arm.clone());
                let payload_bytes = Arc::clone(&payload_bytes);
                async move {
                    for seq in 0..THROUGHPUT_ELEMENT_COUNT {
                        let request = Request::new(Payload {
                            data: payload_bytes.as_ref().clone(),
                            sequence: seq,
                        });
                        let resp = client
                            .process(request)
                            .await
                            .expect("gRPC unary call must succeed");
                        assert_eq!(resp.into_inner().sequence, seq);
                    }
                }
            });
        });
    }

    group.finish();

    let _ = shutdown_tx.send(());
}

criterion_group!(
    benches,
    bench_latency_torvyn_vs_grpc,
    bench_throughput_torvyn_vs_grpc
);
criterion_main!(benches);
