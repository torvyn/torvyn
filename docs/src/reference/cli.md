# CLI Reference

The `torvyn` CLI is a single statically-linked binary with a subcommand dispatch model. All commands support `--format json` for machine-readable output.

## Global Options

```
torvyn [OPTIONS] <COMMAND>

Options:
  --format <FORMAT>    Output format for all commands: human (default), json
  --color <WHEN>       Color output: auto (default), always, never
  --quiet              Suppress non-essential output
  --verbose            Increase output verbosity
  --help               Print help information
  --version            Print version information
```

**Environment variables:**
- `NO_COLOR` — If set (to any value), disables color output. Follows the no-color.org convention.
- `TORVYN_LOG` — Controls log verbosity (overridden by `--verbose` / `--quiet`).

## Commands

### `torvyn init`

Create a new Torvyn project with correct structure, valid manifest, WIT contracts, and a working starting point.

```
torvyn init [PROJECT_NAME] [OPTIONS]

Arguments:
  [PROJECT_NAME]         Directory name and project name
                         (default: current directory name)

Options:
  --template <TEMPLATE>  Project template
                         Values: source, sink, transform, filter, router,
                         aggregator, full-pipeline, empty
                         Default: transform
  --language <LANG>      Implementation language
                         Values: rust, go, python, zig
                         Default: rust
  --no-git               Skip git repository initialization
  --no-example           Generate contract stubs only, skip example implementation
  --contract-version <V> Torvyn contract version to target (default: 0.1.0)
  --interactive          Launch interactive wizard for guided setup
  --force                Overwrite existing directory contents
```

**Example:**
```
$ torvyn init my-transform --template transform --language rust
✓ Created project "my-transform" with template "transform"

  Next steps:
    cd my-transform
    $EDITOR wit/world.wit     # Review your component's contract
    $EDITOR src/lib.rs        # Implement your component
    torvyn check              # Validate contracts and manifest
    torvyn build              # Compile to WebAssembly component
```

**Exit codes:** 0 (success), 1 (error — directory exists, invalid template, etc.)

### `torvyn check`

Validate WIT contracts, manifest, and project structure. Does not compile or execute anything.

```
torvyn check [OPTIONS]

Options:
  --manifest <PATH>    Path to Torvyn.toml (default: ./Torvyn.toml)
  --strict             Treat warnings as errors
```

Runs a seven-step validation pipeline: manifest parse, manifest schema validation, WIT syntax validation, WIT resolution, world consistency, capability cross-check, and deprecation warnings.

**Exit codes:** 0 (all checks passed), 1 (errors found), 2 (warnings found, only with `--strict`)

### `torvyn build`

Compile the project's declared components to WebAssembly.

```
torvyn build [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml
  --component <NAME>      Build only this component (default: all)
  --debug                 Build without optimisations, overriding `[build] release`
```

Builds every component in the manifest's `[[component]]` list with the toolchain for its `language`, and copies each artifact to `.torvyn/build/<name>.wasm`.

That destination is the contract with `torvyn run`: a flow node naming a component resolves to exactly this path. It is why the artifact is copied rather than read from the component's own `target/` directory — a compiler names its output after the component's package, which need not match the name the manifest gives it.

Rust components build with `cargo component` out of the box. For another language, set `build_command` on the `[[component]]` entry to the command that produces a WebAssembly Component:

```toml
[[component]]
name = "tokenizer"
path = "components/tokenizer"
language = "go"
build_command = "tinygo build -target=wasip2 -o tokenizer.wasm ."
```

The `[build]` table sets the target triple, profile, and any extra arguments passed to the toolchain.

**Exit codes:** 0 (built), 1 (manifest or configuration error), 2 (a component's build failed)

### `torvyn link`

Verify that a pipeline's components are compatible and can be composed.

```
torvyn link [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml with flow definition
  --flow <NAME>           Specific flow to check (default: all flows)
  --components <DIR>      Directory containing compiled .wasm components
  --verbose               Show full interface compatibility details
```

Validates interface compatibility for every edge in the flow graph, DAG structure, role consistency, capability satisfaction, and contract version range intersection.

This is a static check over the manifest: it reads the flow's nodes, edges, and `[security.grants]` entries and needs no compiled Wasm, so it can run before `torvyn build`. Each node's role comes from the interface it declares (`torvyn:streaming/source` is a source, `.../sink` a sink, and so on); a node that declares no interface is treated as a processor.

**Exit codes:** 0 (links successfully), 1 (incompatible), 2 (missing components)

### `torvyn build`

Compile source code into a WebAssembly component.

```
torvyn build [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml
  --release               Build with optimizations
  --target <COMPONENT>    Specific component to build (multi-component projects)
  --all                   Build all components
```

Runs `torvyn check` before compilation. For Rust, invokes `cargo component build` (if available) or falls back to `cargo build --target wasm32-wasip2` + `wasm-tools component new`.

**Exit codes:** 0 (build succeeded), 1 (check failed), 2 (compilation failed)

### `torvyn run`

Execute a pipeline locally for development and testing.

```
torvyn run [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml
  --flow <NAME>           Flow to execute (default: first defined flow)
  --input <SOURCE>        Override source input (file path, stdin, or generator)
  --output <SINK>         Override sink output (file path, stdout)
  --limit <N>             Process at most N elements then exit
  --timeout <DURATION>    Maximum execution time (e.g., 30s, 5m)
  --config <KEY=VALUE>    Override component configuration values
  --log-level <LEVEL>     Log verbosity: error, warn, info, debug, trace
```

Runs `torvyn check` and `torvyn link` implicitly before execution. Displays real-time throughput and error counters. Prints summary statistics on completion or Ctrl+C.

**Exit codes:** 0 (completed successfully), 1 (pipeline error), 2 (validation failed), 130 (interrupted by Ctrl+C)

### `torvyn trace`

Execute a pipeline with per-element tracing enabled, producing diagnostic output for each element's path through the pipeline.

```
torvyn trace [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml
  --flow <NAME>           Flow to trace
  --input <SOURCE>        Override source input
  --limit <N>             Report at most N elements
  --output-trace <PATH>   Write trace data to file (default: stdout)
  --trace-format <FMT>    Trace output: pretty (default), json, otlp
  --show-buffers          Include buffer content snapshots (not implemented)
  --show-backpressure     Include per-stream backpressure detail
```

Runs the flow at Diagnostic-level observability, which is the level at which the runtime retains a span per component invocation instead of folding invocations into aggregate histograms. Head sampling is set to 1.0 so the flow being traced is always traced.

Output looks like:

```
  elem-0  ┬─ greeter        pull     461.7µs
          └─ printer        push     346.1µs
          in-component total: 807.8µs

  -- Trace Summary ---------------------------------------------------
  Elements traced:  5
  End-to-end latency:  mean 98.2µs (p50: 37.5µs, p99: 500.0µs)
  Copies:  10 (250 bytes)
  Backpressure:  0 events
  Trace ID:  42394cb3729956c5b0b16c619c3a5e6b
```

**The two latency figures measure different things.** A span's duration is one guest invocation, timed by the reactor around the call; `in-component total` sums an element's spans, so it is the time that element spent *inside* components. The summary's end-to-end percentiles come from the flow's latency histogram, measured from an element's pipeline-entry timestamp to its consumption at the sink — so they *do* include time queued between stages, and are larger.

**`--limit` bounds the report, not the run.** A source decides how many elements it produces; the limit selects the first N for display. The summary always covers the whole run, and the report says so when the two differ.

**Trace identity.** Every span carries a W3C span id under the flow's trace id, and components can read the same identifiers through `flow-context.trace-id()` and `flow-context.span-id()` — so a component's own telemetry can be correlated with the host's trace.

**`--trace-format otlp`** emits an OTLP/HTTP `ExportTraceServiceRequest` body with absolute epoch timestamps, ready to POST to a collector's `/v1/traces` endpoint. Torvyn does not open the connection itself; pair it with `--output-trace` and your own transport.

**`--show-buffers` is not implemented** and exits with an error rather than being silently ignored. Buffer content snapshots would require the runtime to retain payload bytes after an element is consumed, which it deliberately does not do — buffers return to the pool as soon as a stage releases them. Per-element copy counts and byte totals are in the summary.

**Exit codes:** Same as `torvyn run`.

### `torvyn bench`

Run a pipeline under sustained load and produce a performance report.

```
torvyn bench [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml
  --flow <NAME>           Flow to benchmark
  --duration <DURATION>   Benchmark duration (default: 10s)
  --warmup <DURATION>     Warmup period excluded from results (default: 2s)
  --input <SOURCE>        Override source input for reproducible benchmarks
  --report <PATH>         Write report to file (default: stdout)
  --report-format <FMT>   Report format: pretty (default), json, csv, markdown
  --compare <PATH>        Compare against a previous benchmark result
  --baseline <NAME>       Save result as a named baseline
```

Reports throughput, latency percentiles, per-component breakdown, queue statistics, buffer reuse rate, copy accounting, and scheduling metrics.

A benchmark assumes a source that keeps producing. If the flow finishes on its own — which every example and the scaffolded project does, in milliseconds — both the warmup and the measurement window end at that point, and the report covers the whole run rather than the empty window that would otherwise follow it. The report carries a `completion_note` saying so. To benchmark under sustained load, use a source that does not terminate.

Reports are written to `.torvyn/bench/<timestamp>.json`.

**Exit codes:** 0 (benchmark completed), 1 (pipeline error), 3 (regression detected when comparing)

### `torvyn pack`

Package a compiled component as an OCI-compatible artifact.

```
torvyn pack [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml
  --component <NAME>      Specific component to pack (default: all)
  --output <PATH>         Output artifact path (default: .torvyn/artifacts/)
  --tag <TAG>             OCI tag (default: derived from manifest version)
  --include-source        No effect: WIT contracts are always included
  --sign                  Sign artifact (requires signing key configuration)
```

Collects each component's compiled `.wasm` binary from `.torvyn/build/`, its WIT contracts, and an artifact manifest derived from `Torvyn.toml`, and writes a gzip-compressed tar to `.torvyn/artifacts/<name>-<version>.torvyn`. Every layer carries its SHA-256 digest, and an in-toto provenance record names the build tool that produced the binary.

The capabilities recorded in the artifact manifest are the union of the `[security.grants]` entries for the flow nodes that run the component, so an artifact declares what it will need wherever it is deployed.

Every selected component must be built first. If any is not, `pack` names all of them and writes nothing.

`--sign` is accepted but not yet implemented: the artifact is packed unsigned and the command warns. `--include-source` is likewise a no-op — an artifact always carries the component's WIT contracts, since a contract-first runtime cannot verify a component without them.

**Exit codes:** 0 (packed), 1 (a component is unbuilt, or packaging failed), 2 (manifest or contract error)

### `torvyn publish`

Publish a packaged artifact to an OCI registry.

```
torvyn publish [OPTIONS]

Options:
  --artifact <PATH>       Path to packed artifact
  --registry <URL>        Target registry URL
  --tag <TAG>             Override tag
  --dry-run               Validate without pushing
  --force                 Overwrite existing tag
```

With no `--artifact`, publishes the most recently modified artifact in `.torvyn/artifacts/`.

Only a local directory registry is implemented — `--registry local:<dir>`, defaulting to `local:.torvyn/registry`. The artifact is copied there and read back to confirm it arrived intact. A remote registry URL fails with an explanation rather than reporting a push that did not happen.

The reported digest is the SHA-256 of the artifact's bytes, so it can be checked against `sha256sum`. `--dry-run` reports the same digest a real publish would.

**Exit codes:** 0 (published), 1 (remote registry unsupported, or the artifact could not be published), 2 (artifact not found)

### `torvyn inspect`

Display metadata about a compiled component or packaged artifact.

```
torvyn inspect <TARGET> [OPTIONS]

Arguments:
  <TARGET>                Path to .wasm file, OCI artifact, or registry reference

Options:
  --show <SECTION>        What to show: all (default), interfaces, capabilities,
                          metadata, size, contracts, benchmarks
```

Imports and exports are read from the binary's Component Model type section, so they are what the component declares rather than what a manifest claims about it. Interfaces appear under their fully-qualified WIT name, for example `torvyn:streaming/source@0.1.0`. A bare `.wasm` has no manifest, so its version reads `unknown` and only what the binary itself carries is reported.

**Exit codes:** 0 (success), 1 (target not found or invalid)

### `torvyn doctor`

Check the developer's environment for required tools and common misconfigurations.

```
torvyn doctor [OPTIONS]

Options:
  --fix                   Attempt to fix common issues automatically
```

Checks: Torvyn CLI version, Rust toolchain and `wasm32-wasip2` target, `cargo-component`, `wasm-tools`, `wasmtime` (optional), project structure, WIT dependencies, registry connectivity.

A tool counts as present only if it runs *and* exits zero; a binary on `PATH` that fails to execute is reported as a failure, not as found.

**Exit codes:** 0 (all checks passed), 1 (issues found)

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `TORVYN_LOG` | Log filter (e.g., `info`, `torvyn_reactor=debug`) | `info` |
| `TORVYN_HOME` | Torvyn global config and cache directory | `~/.config/torvyn/` |
| `TORVYN_RUNTIME_WORKER_THREADS` | Number of Tokio worker threads | CPU count |
| `TORVYN_RUNTIME_MAX_MEMORY_PER_COMPONENT` | Memory limit per component | `64MiB` |
| `TORVYN_OBSERVABILITY_LEVEL` | Observability level: off, production, diagnostic | `production` |
| `TORVYN_STATE_DIR` | Runtime state directory (inspection socket) | `$XDG_RUNTIME_DIR/torvyn/` |
| `NO_COLOR` | Disable terminal color output | unset |

Environment variables follow the pattern `TORVYN_` + uppercase section + `_` + uppercase key. Example: `runtime.worker_threads` → `TORVYN_RUNTIME_WORKER_THREADS`.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error (validation, compilation, runtime failure) |
| 2 | Missing input or prerequisite |
| 3 | Regression detected (bench comparison) or publish conflict |
| 130 | Interrupted (Ctrl+C / SIGINT) |

All commands produce structured JSON output with `--format json`, including an `exit_code` field, an `errors` array, and command-specific result fields.
