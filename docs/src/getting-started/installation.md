# Installation

This guide walks you through installing Torvyn and verifying that your development environment is ready.

**Time required:** 5–10 minutes.

## Prerequisites

Torvyn components are compiled to WebAssembly using the Rust toolchain. You need the following tools installed before proceeding.

### Rust Toolchain

Torvyn requires **Rust 1.78 or later** (the minimum supported Rust version). The `wasm32-wasip2` compilation target must be available.

If you do not have Rust installed, use [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After installation, add the WebAssembly target:

```bash
rustup target add wasm32-wasip2
```

Verify your Rust version:

```bash
rustc --version
# Expected: rustc 1.78.0 or later
```

### cargo-component

Torvyn uses `cargo-component` to compile Rust code into WebAssembly components that conform to the WebAssembly Component Model.

```bash
cargo install cargo-component
```

Verify:

```bash
cargo component --version
```

### wasm-tools

The `wasm-tools` suite provides utilities for inspecting and manipulating WebAssembly binaries. Torvyn uses it for component inspection and validation.

```bash
cargo install wasm-tools
```

Verify:

```bash
wasm-tools --version
```

## Installing Torvyn

### From Source (Recommended)

Install the `torvyn` CLI from source using Cargo:

```bash
cargo install torvyn-cli
```

This compiles and installs the `torvyn` binary into your Cargo bin directory (typically `~/.cargo/bin/`).

### From Prebuilt Binaries

Prebuilt binaries for Linux (x86_64, aarch64), macOS (x86_64, Apple Silicon), and Windows (x86_64) are available on the [GitHub Releases](https://github.com/torvyn/torvyn/releases) page.

Download the archive for your platform, extract it, and place the `torvyn` binary on your `PATH`:

```bash
# Example for Linux x86_64 — adjust the URL for your platform and version
curl -L https://github.com/torvyn/torvyn/releases/latest/download/torvyn-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv torvyn /usr/local/bin/
```

> **Note:** Prebuilt binaries are planned for the first stable release. During pre-release development, install from source.

### Via Homebrew (macOS and Linux)

```bash
brew install torvyn
```

> **Note:** The Homebrew formula is planned for the first stable release. During pre-release development, install from source.

### Via Nix

```bash
nix profile install nixpkgs#torvyn
```

> **Note:** The Nix package is planned for the first stable release. During pre-release development, install from source.

## Verification

After installation, verify that the `torvyn` binary is accessible:

```bash
torvyn --version
```

Expected output:

```
torvyn 0.1.0
```

## Environment Check

The `torvyn doctor` command inspects your environment and reports any missing or misconfigured tools:

```bash
torvyn doctor
```

Expected output when everything is correctly installed:

```
  Torvyn CLI
    ✓ torvyn 0.1.0 (up to date)

  Rust Toolchain
    ✓ rustc 1.78.0 (or later)
    ✓ wasm32-wasip2 target installed
    ✓ cargo-component installed

  WebAssembly Tools
    ✓ wasm-tools installed

  All checks passed!
```

If any checks fail, `torvyn doctor` displays the issue and a suggested fix. You can also run `torvyn doctor --fix` to attempt automatic repair — for example, installing missing Rust targets.

## Shell Completions

Torvyn can generate shell completion scripts for Bash, Zsh, Fish, and PowerShell:

```bash
# Bash
torvyn completions bash > ~/.bash_completion.d/torvyn

# Zsh
torvyn completions zsh > ~/.zfunc/_torvyn

# Fish
torvyn completions fish > ~/.config/fish/completions/torvyn.fish

# PowerShell
torvyn completions powershell > $PROFILE.CurrentUserAllHosts
```

## Troubleshooting

### `torvyn` command not found

Ensure your Cargo bin directory is on your `PATH`. Add the following to your shell profile (`.bashrc`, `.zshrc`, etc.):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Then restart your shell or run `source ~/.bashrc`.

### `wasm32-wasip2` target not available

If `rustup target add wasm32-wasip2` fails, ensure you are using a recent enough Rust version:

```bash
rustup update stable
rustup target add wasm32-wasip2
```

The `wasm32-wasip2` target requires Rust 1.78 or later.

### `cargo-component` build failures

The `cargo-component` tool depends on a compatible version of the `wit-bindgen` crate. If you encounter version conflicts during builds, ensure your project's `Cargo.toml` specifies `wit-bindgen = "0.36"` (the version used by Torvyn's templates). Check the [Torvyn compatibility matrix](https://docs.torvyn.dev/reference/compatibility) for the current recommended versions.

### Proxy or firewall issues

If `cargo install` fails due to network issues, ensure that `crates.io` and `github.com` are accessible from your machine. If you are behind a corporate proxy, configure Cargo's HTTP proxy settings in `~/.cargo/config.toml`:

```toml
[http]
proxy = "http://your-proxy:port"
```

## Next Steps

Your environment is ready. Continue to the [Quickstart](quickstart.md) to create and run your first Torvyn project.
