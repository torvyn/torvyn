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

Execute a pipeline with full tracing enabled, producing per-element diagnostic output.

```
torvyn trace [OPTIONS]

Options:
  --manifest <PATH>       Path to Torvyn.toml
  --flow <NAME>           Flow to trace
  --input <SOURCE>        Override source input
  --limit <N>             Trace at most N elements
  --output-trace <PATH>   Write trace data to file (default: stdout)
  --trace-format <FMT>    Trace output: pretty (default), json, otlp
  --show-buffers          Include buffer content snapshots
  --show-backpressure     Highlight backpressure events
```

Same as `run` but with Diagnostic-level observability enabled. Every element's path through the pipeline is traced with timing, buffer operations, and copy events.

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
  --include-source        Include source WIT contracts in artifact metadata
  --sign                  Sign artifact (requires signing key configuration)
```

Runs `torvyn check`, collects the compiled `.wasm` binary, contract metadata, and benchmark metadata (if available), and assembles an OCI artifact.

**Exit codes:** 0 (packed), 1 (check failed), 2 (packaging error)

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

**Exit codes:** 0 (published), 1 (authentication failed), 2 (push failed), 3 (artifact invalid)

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

**Exit codes:** 0 (success), 1 (target not found or invalid)

### `torvyn doctor`

Check the developer's environment for required tools and common misconfigurations.

```
torvyn doctor [OPTIONS]

Options:
  --fix                   Attempt to fix common issues automatically
```

Checks: Torvyn CLI version, Rust toolchain and `wasm32-wasip2` target, `cargo-component`, `wasm-tools`, `wasmtime` (optional), project structure, WIT dependencies, registry connectivity.

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
