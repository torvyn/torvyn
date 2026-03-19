.PHONY: build run clean check

# Build both components to WebAssembly.
build:
	torvyn check
	cd components/hello-source && cargo component build --release
	cd components/hello-sink && cargo component build --release

# Run the pipeline.
run: build
	torvyn run flow.main

# Run with tracing enabled to see flow lifecycle events.
trace: build
	torvyn trace flow.main

# Validate contracts and topology without running.
check:
	torvyn check
	torvyn link flow.main

# Benchmark this pipeline (latency, throughput, copies).
bench: build
	torvyn bench flow.main --iterations 1000

clean:
	cd components/hello-source && cargo clean
	cd components/hello-sink && cargo clean
