# Benchmark Comparison

## What It Demonstrates

A controlled performance comparison of the same logical pipeline implemented three ways: in Torvyn (Wasm component boundary), over gRPC localhost, and over Unix domain sockets. Includes a benchmark harness that measures latency percentiles, throughput, memory usage, and copy counts.

## Concepts Covered

- Torvyn's performance relative to conventional IPC mechanisms
- Benchmark methodology and reproducibility
- Using `torvyn bench` for profiling
- Statistical rigor (percentiles, not averages)

## Methodology

All three implementations perform the same work: a source produces 100,000 64-byte messages, a processor appends an 8-byte timestamp, and a sink consumes the result. The benchmark measures end-to-end latency (time from source.pull() to sink.push() completion) and throughput (elements per second).

The comparison is designed to be fair. The gRPC and Unix socket implementations serialize the same payload format and perform the same transformation logic. The Torvyn implementation uses the standard WIT interfaces.

## File Listing

```
examples/benchmark-comparison/
├── Torvyn.toml
├── Makefile
├── README.md
├── bench.sh                     # Runs all three and produces report
├── torvyn/                      # Torvyn implementation
│   └── ... (standard Torvyn components)
├── grpc-baseline/               # gRPC localhost implementation
│   ├── Cargo.toml
│   ├── proto/pipeline.proto
│   └── src/main.rs
├── unix-socket-baseline/        # Unix domain socket implementation
│   ├── Cargo.toml
│   └── src/main.rs
└── results/
    └── .gitkeep
```

## Benchmark Script

**`bench.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail

ITERATIONS=${1:-100000}
PAYLOAD_SIZE=${2:-64}
RESULTS_DIR="results"

mkdir -p "$RESULTS_DIR"

echo "=== Torvyn Benchmark ==="
echo "Iterations: $ITERATIONS | Payload: ${PAYLOAD_SIZE}B"
echo ""

# 1. Torvyn pipeline benchmark
echo "--- Torvyn (Wasm component boundary) ---"
cd torvyn
torvyn bench flow.main \
    --iterations "$ITERATIONS" \
    --payload-size "$PAYLOAD_SIZE" \
    --warmup 1000 \
    --output json > "../$RESULTS_DIR/torvyn.json"
cd ..

# 2. gRPC localhost benchmark
echo "--- gRPC localhost ---"
cd grpc-baseline
cargo run --release -- \
    --iterations "$ITERATIONS" \
    --payload-size "$PAYLOAD_SIZE" \
    --warmup 1000 \
    --output json > "../$RESULTS_DIR/grpc.json"
cd ..

# 3. Unix domain socket benchmark
echo "--- Unix domain socket ---"
cd unix-socket-baseline
cargo run --release -- \
    --iterations "$ITERATIONS" \
    --payload-size "$PAYLOAD_SIZE" \
    --warmup 1000 \
    --output json > "../$RESULTS_DIR/unix-socket.json"
cd ..

# Generate comparison report
echo ""
echo "=== Comparison Report ==="
echo ""
printf "%-25s %12s %12s %12s %12s\n" "Method" "p50 (us)" "p99 (us)" "Throughput" "Copies"
printf "%-25s %12s %12s %12s %12s\n" "-------" "--------" "--------" "----------" "------"

for method in torvyn grpc unix-socket; do
    file="$RESULTS_DIR/${method}.json"
    if [ -f "$file" ]; then
        # Parse JSON results (requires jq)
        p50=$(jq -r '.latency_p50_us' "$file")
        p99=$(jq -r '.latency_p99_us' "$file")
        throughput=$(jq -r '.throughput_eps' "$file")
        copies=$(jq -r '.total_copies // "N/A"' "$file")
        printf "%-25s %12s %12s %12s %12s\n" "$method" "$p50" "$p99" "$throughput" "$copies"
    fi
done

echo ""
echo "Full results in $RESULTS_DIR/"
echo "Hardware: $(uname -m) | OS: $(uname -s) | Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
```

## Expected Results (Design Targets)

These are design targets based on the architecture's performance goals. Actual measurements will vary by hardware.

| Method | p50 Latency | p99 Latency | Throughput | Copies/Element |
|--------|-------------|-------------|------------|----------------|
| Torvyn (Wasm boundary) | ~5-15 us | ~25-50 us | ~200K-500K elem/s | 2 (measured) |
| gRPC localhost | ~50-200 us | ~500-2000 us | ~20K-100K elem/s | 4+ (serialization) |
| Unix domain socket | ~10-30 us | ~50-150 us | ~100K-300K elem/s | 3 (socket + serialization) |

Torvyn's design target for per-element host overhead is < 5 us (`MAX_HOT_PATH_NS`). The additional latency comes from Wasm boundary crossing and buffer operations. The key advantage over gRPC is the elimination of network stack overhead, protobuf serialization, and HTTP/2 framing for same-node communication.

## Commentary

This benchmark is not intended to prove that Torvyn is universally faster than gRPC. gRPC is designed for distributed communication; comparing it to a same-node runtime on localhost is inherently unfair to gRPC. The purpose is to quantify the overhead difference for the specific use case Torvyn targets: same-node, low-latency component composition.

The benchmark methodology follows the vision document's requirement for "rigorous methodology and reproducible benchmarks" (Section 14.3). All results include hardware specifications, OS version, iteration counts, warmup periods, and percentile distributions — not just averages.

## Learn More

- [Benchmark Methodology](docs/benchmarks/methodology.md)
- [Architecture Guide: Performance Model](docs/architecture.md#performance)
- [CLI Reference: `torvyn bench`](docs/cli.md#torvyn-bench)
