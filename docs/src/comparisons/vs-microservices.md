# Torvyn vs. gRPC for Local Pipelines

An honest comparison of two different approaches to composing processing stages on the same machine.

---

## What gRPC Provides

gRPC is a high-performance RPC framework with strong typing (via Protocol Buffers), bidirectional streaming, broad language support, and a large ecosystem. It is the standard for service-to-service communication in many organizations.

## Where Torvyn Is a Better Fit

**Same-node composition without network overhead.** gRPC is fundamentally a network protocol. Even when two services run on the same machine, gRPC communication traverses the network stack: Protocol Buffers serialization, HTTP/2 framing, TCP socket I/O (or Unix domain sockets, which are better but still involve kernel context switches). Torvyn components communicate through host-managed memory transfers without leaving the process. For high-frequency local pipelines, this eliminates a significant latency and CPU cost at each boundary.

**Ownership-aware resource transfer.** gRPC transfers data by serializing it into a wire format. Every boundary involves a full serialization/deserialization cycle. Torvyn tracks buffer ownership at the host level and only copies payload data when a component actually needs to read or transform it. Routing and metadata-only stages skip payload copies entirely.

**Built-in reactive back-pressure.** gRPC's flow control operates at the HTTP/2 stream level and is designed for network conditions. Torvyn's back-pressure operates at the stream element level with demand-driven scheduling, configurable overflow policies, and observable watermarks. It is designed specifically for fine-grained streaming pipeline stages.

**Contract validation before runtime.** Both gRPC and Torvyn use typed contracts (Protocol Buffers and WIT, respectively). Torvyn adds composition-level validation through `torvyn link`, which checks interface compatibility across an entire pipeline graph — including version compatibility, capability satisfaction, and topology correctness — before any code runs.

## Where gRPC Is a Better Fit

**Cross-network communication.** gRPC is designed for network-first communication. If your pipeline stages run on different machines, Torvyn is not a replacement for gRPC. Torvyn's current focus is same-node and edge-local composition.

**Ecosystem maturity and tooling.** gRPC has years of production use, extensive documentation, wide language support, and deep integration with service mesh infrastructure (Envoy, Istio). Torvyn is a new project in active development.

**Request-response patterns.** gRPC excels at request-response communication with well-defined service endpoints. If your architecture is not a streaming pipeline — if it is a set of services handling discrete requests — gRPC's model is more natural.

**Organizational scaling.** gRPC's service-per-team deployment model is well-suited for large organizations with independent team ownership. Torvyn's same-node composition model works best when pipeline stages are co-deployed and co-versioned.

## When to Use Which

**Use Torvyn when** your processing stages run on the same node, latency budgets are tight (single-digit milliseconds or less for the full pipeline), you need fine-grained back-pressure with observable queue behavior, and you value static contract validation across the full pipeline graph.

**Use gRPC when** your services run on different machines, you need mature ecosystem integration, you are building request-response APIs rather than streaming pipelines, or your organization requires independently deployable services with separate team ownership.

**Use both** when your architecture has a high-frequency local processing pipeline (Torvyn) that communicates with external services via network APIs (gRPC). The two approaches are complementary.
