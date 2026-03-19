.PHONY: build run clean check

# Build all three components to WebAssembly.
build:
	torvyn check
	cd components/json-source && cargo component build --release
	cd components/json-transform && cargo component build --release
	cd components/json-sink && cargo component build --release

# Run the pipeline.
run: build
	torvyn run flow.main

# Run with tracing enabled.
trace: build
	torvyn trace flow.main

# Validate contracts and topology without running.
check:
	torvyn check
	torvyn link flow.main

clean:
	cd components/json-source && cargo clean
	cd components/json-transform && cargo clean
	cd components/json-sink && cargo clean
