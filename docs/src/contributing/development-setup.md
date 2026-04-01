# Development Setup

This guide takes you from a clean machine to a fully working Torvyn development environment. Every command is copy-paste ready. If you hit a problem not covered here, check the [Troubleshooting](#troubleshooting) section at the bottom, or run `torvyn doctor` once the CLI is built.

## Prerequisites

You need four things installed before you start.

### 1. Rust Toolchain

Torvyn requires Rust 1.78 or later and targets the `wasm32-wasip2` compilation target.

```bash
# Install rustup if you do not have it
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Ensure you are on at least Rust 1.78
rustup update stable
rustc --version  # Should print 1.78.0 or later

# Add the WebAssembly compilation target
rustup target add wasm32-wasip2
```

### 2. wasm-tools

The `wasm-tools` binary is used for component model operations (composing, inspecting, and validating Wasm components).

```bash
cargo install wasm-tools
wasm-tools --version
```

### 3. Wasmtime CLI

Wasmtime is the reference WebAssembly runtime that Torvyn builds on. You need the CLI for running and testing components outside of the Torvyn host.

```bash
curl https://wasmtime.dev/install.sh -sSf | bash
wasmtime --version  # Should print 25.0.0 or later
```

If your platform does not support the install script, see [wasmtime.dev](https://wasmtime.dev/) for alternative installation methods.

### 4. cargo-component (recommended)

`cargo-component` simplifies building WebAssembly components from Rust. It is the recommended build tool for Torvyn guest components, though it is not required for building the host runtime itself.

```bash
cargo install cargo-component
cargo component --version
```

**Fallback:** If `cargo-component` is unavailable or unstable on your system, you can compile guest components manually with `cargo build --target wasm32-wasip2` followed by `wasm-tools component new`. The CLI supports both paths.

## Clone and Build

```bash
# Clone the repository
git clone https://github.com/torvyn/torvyn.git
cd torvyn

# Build the entire workspace in debug mode
cargo build --workspace

# Build in release mode (slower to compile, faster to run)
cargo build --workspace --release
```

A successful build produces two binaries of interest: the `torvyn` CLI (`target/debug/torvyn` or `target/release/torvyn`) and the `torvyn-host` runtime binary.

If the build fails, run `torvyn doctor` (once built) or check [Troubleshooting](#troubleshooting).

## Run the Test Suite

```bash
# Run all unit and integration tests across the workspace
cargo test --workspace

# Run tests for a specific crate
cargo test -p torvyn-types
cargo test -p torvyn-reactor

# Run a specific test by name
cargo test -p torvyn-types test_flow_state_full_happy_path

# Run tests with output visible (useful for debugging)
cargo test -p torvyn-types -- --nocapture
```

## IDE Setup

### rust-analyzer

Torvyn is a standard Cargo workspace. Any editor with rust-analyzer support works. No special configuration is required beyond pointing rust-analyzer at the workspace root.

If you use VS Code, add the following to `.vscode/settings.json` in the repository root:

```json
{
  "rust-analyzer.cargo.features": "all",
  "rust-analyzer.check.command": "clippy",
  "rust-analyzer.check.extraArgs": ["--workspace", "--", "-D", "warnings"],
  "rust-analyzer.inlayHints.parameterHints.enable": true,
  "rust-analyzer.inlayHints.typeHints.enable": true,
  "rust-analyzer.lens.run.enable": true,
  "rust-analyzer.lens.debug.enable": true,
  "[rust]": {
    "editor.formatOnSave": true,
    "editor.defaultFormatter": "rust-lang.rust-analyzer"
  }
}
```

### Recommended VS Code Extensions

These are not required, but they improve the development experience:

- **rust-analyzer** — Rust language server (essential)
- **Even Better TOML** — TOML syntax support (for `Torvyn.toml` and `Cargo.toml` files)
- **Error Lens** — Inline diagnostic display
- **CodeLLDB** — Debugger integration for Rust
- **WIT IDL** — Syntax highlighting for `.wit` files (if available)

### Other Editors

For Neovim, Helix, Zed, or other editors with LSP support: point your LSP client at rust-analyzer with the workspace root as the project directory. No additional configuration is required.

## Development Workflow

The standard edit-build-test cycle for Torvyn:

```bash
# 1. Edit source files in your editor

# 2. Build the affected crate(s)
cargo build -p torvyn-reactor

# 3. Run the affected tests
cargo test -p torvyn-reactor

# 4. Format your code
cargo fmt --all

# 5. Run the linter
cargo clippy --workspace -- -D warnings

# 6. Check documentation compiles cleanly
cargo doc --workspace --no-deps

# 7. Commit using Conventional Commits format
git add -A
git commit -m "feat(reactor): add weighted fair queuing policy"
```

### Running the Full CI Pipeline Locally

Before pushing a pull request, run the complete CI check locally. This is the same sequence that runs in CI:

```bash
# Format check (CI will reject unformatted code)
cargo fmt --all -- --check

# Clippy with warnings-as-errors
cargo clippy --workspace --all-targets -- -D warnings

# Full test suite
cargo test --workspace

# Documentation build (no warnings allowed)
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Build the Wasm test components (if you modified contracts or guest code)
# cargo component build --manifest-path examples/test-components/Cargo.toml --release
```

If all four commands succeed with no errors and no warnings, your change is ready for review.

## Troubleshooting

### `error[E0658]: use of unstable library feature`

You are on a Rust version older than 1.78. Run `rustup update stable` and try again.

### `error: target 'wasm32-wasip2' not found`

The WASI preview 2 target is not installed. Run `rustup target add wasm32-wasip2`.

### `cargo-component` build fails with version mismatch

The `cargo-component` tool evolves quickly and may have compatibility issues with specific `wasm-tools` versions. If you encounter errors, try updating both tools:

```bash
cargo install cargo-component --force
cargo install wasm-tools --force
```

If problems persist, use the fallback build path described in the Prerequisites section. File an issue if you believe the incompatibility should be documented.

### Linker errors on macOS with Wasmtime

If you see linker errors related to `wasmtime-runtime` on macOS, ensure you have the Xcode command-line tools installed:

```bash
xcode-select --install
```

### Tests fail with "component not found" or "artifact missing"

Some integration tests depend on pre-compiled Wasm test components. Build them first:

```bash
cd examples/test-components
cargo component build --release
cd ../..
cargo test --workspace
```

### Build is very slow

First builds compile the entire dependency tree including Wasmtime, which is large. Subsequent builds are incremental and much faster. If you are developing a single crate, build and test only that crate:

```bash
cargo test -p torvyn-types  # Much faster than --workspace
```

Consider using `cargo-watch` for automatic rebuilds:

```bash
cargo install cargo-watch
cargo watch -x "test -p torvyn-reactor"
```

### Something else is broken

Run `torvyn doctor` if the CLI is built. It checks for common environment issues including toolchain versions, missing targets, and configuration problems. If that does not resolve your issue, open a discussion on GitHub with the output of:

```bash
rustc --version
cargo --version
wasm-tools --version
wasmtime --version
uname -a
```
