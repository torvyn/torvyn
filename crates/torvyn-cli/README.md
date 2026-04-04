# torvyn-cli

[![crates.io](https://img.shields.io/crates/v/torvyn-cli.svg)](https://crates.io/crates/torvyn-cli)
[![docs.rs](https://docs.rs/torvyn-cli/badge.svg)](https://docs.rs/torvyn-cli)
[![license](https://img.shields.io/crates/l/torvyn-cli.svg)](https://github.com/torvyn/torvyn/blob/main/LICENSE)

Developer CLI for the [Torvyn](https://github.com/torvyn/torvyn) streaming runtime.

## Overview

`torvyn-cli` is the primary user-facing interface to the Torvyn runtime. It provides subcommands covering the complete developer workflow: project scaffolding, contract validation, component linking, pipeline execution, diagnostic tracing, benchmarking, artifact packaging, and environment diagnostics.

The binary is named `torvyn` and is built on [clap 4.5](https://docs.rs/clap) with derive-based argument parsing. Output supports both human-readable (styled terminal with colors, tables, and progress bars) and machine-readable (JSON) formats.

## Position in the Architecture

This crate sits at **Tier 6 (Entry Point)** alongside `torvyn-host`. It translates CLI arguments into calls to the Torvyn subsystem crates. The CLI itself contains no domain logic -- it is a thin dispatch layer with output formatting.

## Subcommands

| Command | Description |
|---------|-------------|
| `torvyn init` | Scaffold a new project from a template (source, sink, transform, filter, router, aggregator, full-pipeline, or empty) |
| `torvyn check` | Validate WIT contracts, manifest schema, and project structure |
| `torvyn link` | Verify component interface compatibility and pipeline topology |
| `torvyn run` | Execute a pipeline locally with optional element limits and timeouts |
| `torvyn trace` | Run with full diagnostic tracing (per-stage latency, resource transfers, backpressure) |
| `torvyn bench` | Benchmark a pipeline with warmup, producing p50/p95/p99/p99.9 latency and throughput reports |
| `torvyn pack` | Assemble components into a distributable `.torvyn` artifact |
| `torvyn publish` | Push a packaged artifact to an OCI-compatible registry |
| `torvyn inspect` | Display metadata, interfaces, capabilities, and size for a component or artifact |
| `torvyn doctor` | Check the development environment for required tools and common misconfigurations |
| `torvyn completions` | Generate shell completions (bash, zsh, fish, powershell) |

## Global Options

All subcommands accept the following global flags:

| Flag | Short | Description |
|------|-------|-------------|
| `--verbose` | `-v` | Show debug-level output |
| `--quiet` | `-q` | Suppress non-essential output (errors only) |
| `--format` | | Output format: `human` (default) or `json` |
| `--color` | | Color control: `auto` (default), `always`, or `never` |

## Project Templates

The `init` subcommand supports multiple templates via `--template`:

| Template | Description |
|----------|-------------|
| `source` | Data producer (no input, one output) |
| `sink` | Data consumer (one input, no output) |
| `transform` | Stateless data transformer (default) |
| `filter` | Content filter/guard |
| `router` | Multi-output router |
| `aggregator` | Stateful windowed aggregator |
| `full-pipeline` | Complete multi-component pipeline |
| `empty` | Minimal skeleton for experienced users |

## Key Exports

This is primarily a binary crate. The library target exposes types for integration testing:

- **`Cli`** -- Top-level clap parser struct
- **`GlobalOpts`** -- Global option struct
- **`Command`** -- Enum of all subcommands
- **`OutputFormat`** -- `Human` or `Json`
- **`ColorChoice`** -- `Auto`, `Always`, or `Never`

## Quick Start

```bash
# Create a new transform component
torvyn init my-transform --template transform --language rust

# Validate contracts and manifest
cd my-transform
torvyn check

# Build the Wasm component (standard cargo workflow)
cargo component build --release

# Verify interface compatibility
torvyn link

# Run the pipeline
torvyn run --limit 100

# Benchmark
torvyn bench --duration 30s --warmup 5s

# Package for distribution
torvyn pack --sign
torvyn publish --registry oci://registry.example.com/torvyn
```

## Dependencies

The CLI depends on the following Torvyn subsystem crates: `torvyn-types`, `torvyn-config`, `torvyn-contracts`, `torvyn-engine`, `torvyn-host`, `torvyn-packaging`, and `torvyn-linker`. External dependencies include `clap`, `console`, `indicatif`, `tabled`, `chrono`, and `tokio`.

## Installation

```bash
cargo install torvyn-cli
```

Or install the umbrella crate which includes the CLI by default:

```bash
cargo install torvyn
```

## Repository

This crate is part of the [Torvyn](https://github.com/torvyn/torvyn) project.
See the main repository for architecture documentation and contribution guidelines.
