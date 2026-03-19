.PHONY: build run clean check trace

# Build all four components to WebAssembly.
build:
	torvyn check
	cd components/token-source && cargo component build --release
	cd components/content-filter && cargo component build --release
	cd components/sentence-aggregator && cargo component build --release
	cd components/output-sink && cargo component build --release

# Run the pipeline.
run: build
	torvyn run flow.main

# Run with tracing to see filter decisions and aggregation.
trace: build
	torvyn trace flow.main

# Validate contracts and topology without running.
check:
	torvyn check
	torvyn link flow.main

clean:
	cd components/token-source && cargo clean
	cd components/content-filter && cargo clean
	cd components/sentence-aggregator && cargo clean
	cd components/output-sink && cargo clean
