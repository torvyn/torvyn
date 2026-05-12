# `torvyn-integration-tests`

Workspace-level integration tests that exercise multiple Torvyn crates
together (engine + reactor + pipeline + resources).

## Test layout

| Test file | Engine | Toolchain required |
|---|---|---|
| `tests/test_pipeline_instantiation.rs` | `MockEngine` | Rust stable only |
| `tests/test_real_wasm_e2e.rs` (feature-gated) | `WasmtimeEngine` + real `.wasm` | Rust stable, `wasm32-wasip2` target, `cargo-component` |
| `tests/test_polyglot_e2e.rs` (feature-gated) | `WasmtimeEngine` + a TinyGo source + Rust processor + Rust sink | All of the above, plus `tinygo` and `wit-bindgen-go` |

## Running

### Default (Mock-engine tests only)
```bash
cargo test -p torvyn-integration-tests
```
This is what the `integration` CI job runs. No Wasm toolchain required.

### Real-Wasm end-to-end (Rust-only pipeline)
```bash
rustup target add wasm32-wasip2
cargo install cargo-component --locked
cargo test -p torvyn-integration-tests --features wasm-e2e --test test_real_wasm_e2e
```
This is what the `wasm-e2e` CI job runs. The `build.rs` invokes
`cargo component build --release --target wasm32-wasip2` for each of
the three test fixtures under `tests/test-components/` and emits
`TORVYN_<NAME>_WASM` environment variables that the test file reads via
`env!()`.

### Polyglot end-to-end (Go source + Rust processor + Rust sink)
```bash
rustup target add wasm32-wasip2
cargo install cargo-component --locked
# TinyGo via your package manager (Homebrew, apt, the official .deb, …)
go install go.bytecodealliance.org/cmd/wit-bindgen-go@latest
cargo test -p torvyn-integration-tests --features wasm-polyglot --test test_polyglot_e2e
```
This is what the `wasm-polyglot-e2e` CI job runs. The `build.rs` adds
two extra steps to the standard pipeline:
1. Stages a WIT tree under `tests/test-components/go-echo-source/wit/deps/`
   by copying the canonical `torvyn:streaming` contracts and TinyGo's
   bundled WASI Preview-2 deps. This avoids requiring a network call
   to a WIT registry at build time.
2. Generates Go bindings via `wit-bindgen-go`, then compiles the
   component with `tinygo build -target=wasip2 -scheduler=none`.

The test asserts the same "exactly 4 measured copies per element"
invariant as the Rust-only pipeline, proving that the Go-emitted
buffer is indistinguishable from the Rust-emitted buffer as far as the
host's `DefaultResourceManager` and `CopyLedger` are concerned.

## Escape hatch

If you don't have `cargo-component` installed and want to run only the
non-Wasm tests, the build is automatically skipped:
- `build.rs` detects `cargo-component`'s absence and emits a warning
  rather than failing the build.
- The `wasm-e2e` test target is gated behind `required-features =
  ["wasm-e2e"]`, so a plain `cargo test --workspace` will not try to
  compile or run it.

To explicitly skip the Wasm build even when `cargo-component` is
installed:
```bash
TORVYN_SKIP_WASM_BUILD=1 cargo test -p torvyn-integration-tests
```

## What the real-Wasm test proves

`tests/test_real_wasm_e2e.rs` is the keystone end-to-end test for
Item 2 of the project's Tier-1 list. When it passes, the project's
"polyglot streaming runtime" claim is operationally demonstrated:

1. Three real Component Model components are compiled by
   `cargo-component`, loaded by the engine, instantiated by
   `wasmtime::component::bindgen!`-typed bindings, and have their
   `lifecycle.init` exports invoked.
2. The reactor drives the Source → Processor → Sink pipeline through
   real Wasm code on every element.
3. The `DefaultResourceManager` records **exactly four** measured copy
   events per element on the hot path — the ownership-aware
   "Ownership-Aware Is Not Zero-Copy" invariant.
