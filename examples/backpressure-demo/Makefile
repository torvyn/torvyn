.PHONY: build run clean check trace

# Build both components to WebAssembly.
build:
	torvyn check
	cd components/fast-source && cargo component build --release
	cd components/slow-sink && cargo component build --release

# Run the pipeline.
run: build
	torvyn run flow.main

# Run with tracing to observe backpressure events.
trace: build
	torvyn trace flow.main

# Validate contracts and topology without running.
check:
	torvyn check
	torvyn link flow.main

clean:
	cd components/fast-source && cargo clean
	cd components/slow-sink && cargo clean
