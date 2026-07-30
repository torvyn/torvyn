//! Shared in-process gRPC baseline used by every arm that compares Torvyn
//! against a conventional same-node service boundary.
//!
//! One implementation, used by both the mock-invoker comparison and the
//! real-Wasm comparison, so the gRPC side of the ratio is byte-for-byte
//! identical in each. Anything that would change the gRPC arm's cost —
//! payload size, connection reuse, server topology — must change here, once,
//! for both.
//!
//! The server is a trivial echo handler on a random `127.0.0.1` port. Its
//! per-call cost is dominated by transport (HTTP/2 framing, protobuf
//! encode/decode) rather than application logic, which is exactly what a
//! transport baseline should be.

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::{Request, Response, Status};

// Generated tonic / prost code for the Echo service. The lints below are for
// the auto-generated module only — the crate-level `clippy::all = "deny"`
// would otherwise reject derive-macro output that this crate does not own.
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

pub use echo_proto::{echo_client::EchoClient, Payload};

use echo_proto::echo_server::{Echo, EchoServer};

/// Payload size used by every arm of every Torvyn-vs-gRPC comparison.
///
/// Chosen to sit comfortably above the gRPC framing-overhead floor while
/// still matching realistic small-element streaming workloads. It is also
/// exactly the Torvyn buffer pool's `Small` tier capacity, so the Torvyn arm
/// allocates from a pre-warmed tier rather than paying an on-demand
/// allocation.
pub const PAYLOAD_BYTES: usize = 256;

/// Element counts for latency comparisons. 100 captures the
/// startup-dominated regime, 1 000 the work-dominated regime, 10 000 the
/// throughput-dominated regime.
pub const LATENCY_ELEMENT_COUNTS: &[u64] = &[100, 1_000, 10_000];

/// Element count for throughput comparisons (a single point — criterion's
/// `Throughput::Elements` already reports elements/second).
pub const THROUGHPUT_ELEMENT_COUNT: u64 = 10_000;

/// Trivial echo handler. Returns the request unchanged.
struct EchoService;

#[tonic::async_trait]
impl Echo for EchoService {
    async fn process(&self, req: Request<Payload>) -> Result<Response<Payload>, Status> {
        Ok(Response::new(req.into_inner()))
    }
}

/// A running in-process gRPC echo server plus a connected client channel.
///
/// Both are created once per benchmark group: the server so we measure
/// per-call transport cost rather than server startup, the channel so we do
/// not pay a fresh TCP/HTTP-2 handshake per iteration. `Channel` is
/// internally `Arc`-backed, so cloning it yields an independent client
/// handle over the same connection.
///
/// Dropping the harness shuts the server down.
pub struct GrpcBaseline {
    channel: Channel,
    shutdown: Option<oneshot::Sender<()>>,
}

impl GrpcBaseline {
    /// Spawn the server on `rt` and connect a client channel to it.
    ///
    /// # Panics
    /// Panics if the listener cannot bind, or if the client cannot connect
    /// within five seconds — either means the benchmark cannot produce a
    /// meaningful baseline and should fail loudly rather than report a
    /// one-sided comparison.
    #[must_use]
    pub fn spawn(rt: &Runtime) -> Self {
        // Bind synchronously *outside* the runtime so the bound port is known
        // before this function returns and the caller does not need to poll.
        // The conversion to a Tokio listener must happen inside a Tokio
        // context, so it is deferred to the spawned task body.
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

            // TCP_NODELAY on every accepted socket. `serve_with_incoming`
            // hands tonic a stream of already-accepted connections, so
            // tonic's own `Server::tcp_nodelay` setting never reaches them —
            // they keep Nagle's algorithm enabled.
            //
            // That is not a cosmetic default. Nagle on the server side plus
            // the client's delayed-ACK timer stalls every request/response
            // round trip until the timer fires: on Linux that is 40 ms, and
            // this benchmark measured a flat ~41 ms per unary call on CI
            // runners while the same code ran in ~53 us on macOS. The
            // baseline was reporting a kernel timer, not gRPC's transport
            // cost, and the resulting Torvyn-vs-gRPC ratio was inflated by
            // roughly three orders of magnitude.
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener)
                .map(|accepted| accepted.inspect(|stream| drop(stream.set_nodelay(true))));

            Server::builder()
                .add_service(EchoServer::new(EchoService))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("Tonic server crashed");
        });

        let endpoint = format!("http://{addr}");
        let channel = rt.block_on(async move {
            Endpoint::from_shared(endpoint)
                .expect("valid endpoint URI")
                // Explicit on the client side too. Tonic defaults this to
                // true, but a baseline whose validity depends on a library
                // default going unchanged is not a baseline.
                .tcp_nodelay(true)
                .connect_timeout(Duration::from_secs(5))
                .connect()
                .await
                .expect("gRPC client must connect")
        });

        Self {
            channel,
            shutdown: Some(shutdown_tx),
        }
    }

    /// A client handle sharing this baseline's connection.
    #[must_use]
    pub fn client(&self) -> EchoClient<Channel> {
        EchoClient::new(self.channel.clone())
    }
}

impl Drop for GrpcBaseline {
    fn drop(&mut self) {
        // Ignore the result: if the receiver is already gone the server is
        // already shutting down and there is nothing to do.
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// A reusable `PAYLOAD_BYTES`-sized request body.
///
/// Shared across iterations so the gRPC arm is not charged for allocating
/// its payload — only for serialising and transporting it.
#[must_use]
pub fn payload_template() -> Arc<Vec<u8>> {
    Arc::new(vec![0u8; PAYLOAD_BYTES])
}

/// Issue `count` sequential unary calls, asserting the echo invariant on
/// each. This is the measured body of every gRPC arm.
///
/// # Panics
/// Panics if any call fails or the server echoes back the wrong sequence
/// number — a benchmark that silently drops work is worse than no benchmark.
pub async fn drive_unary_calls(
    mut client: EchoClient<Channel>,
    payload: &Arc<Vec<u8>>,
    count: u64,
) {
    for seq in 0..count {
        let request = Request::new(Payload {
            data: payload.as_ref().clone(),
            sequence: seq,
        });
        let resp = client
            .process(request)
            .await
            .expect("gRPC unary call must succeed");
        assert_eq!(
            resp.into_inner().sequence,
            seq,
            "echo server returned a mismatched sequence number"
        );
    }
}
