# Contributing to Torvyn

Thank you for your interest in contributing to Torvyn. This project is built on the belief that high-performance streaming infrastructure should be safe, composable, and observable — and that achieving this goal requires a strong community working together.

Every contribution matters. Whether you are reporting a bug, improving documentation, adding a benchmark, writing a blog post, or implementing a new feature — you are helping make Torvyn better for everyone.

---

## Project Philosophy

Before contributing, it helps to understand what Torvyn values:

- **Contract-first design.** Every component interaction is defined through typed WIT interfaces. Contracts are the center of the product.
- **Ownership-aware correctness.** Data movement is explicit. Copies are bounded, measurable, and visible. The runtime never promises universal zero-copy — it promises discipline.
- **Measurable performance.** Every design decision should be benchmarkable. Claims must be backed by reproducible evidence.
- **Safety by default.** Components run in WebAssembly sandboxes with capability-based isolation. Unsafe Rust is minimized, isolated, and justified.
- **Operational realism.** The runtime is designed for production environments, not only idealized demos.

Contributions that align with these values are welcome. Contributions that conflict with them will receive feedback explaining why, and guidance on how to bring them into alignment.

---

## Types of Contributions

Torvyn welcomes contributions in many forms:

- **Code:** Bug fixes, new features, performance improvements, test coverage.
- **Documentation:** Guides, API documentation, tutorials, architecture explanations.
- **Examples:** Sample pipelines, component templates, integration patterns.
- **Bug reports:** Clear, reproducible issue reports with environment details.
- **Benchmarks:** New benchmarks, benchmark methodology improvements, performance regression reports.
- **Design discussion:** Participating in RFCs, reviewing architectural proposals, suggesting improvements.
- **Triage:** Helping categorize and reproduce issues.

---

## Development Environment Setup

### Prerequisites

- **Rust toolchain:** Install via [rustup](https://rustup.rs/). Torvyn targets the latest stable Rust release. The current minimum supported Rust version (MSRV) is 1.78.
- **Wasm target:** Install the WebAssembly compilation target.
- **Wasmtime:** The host runtime embeds Wasmtime, which is pulled as a Cargo dependency. No separate installation is required.
- **cargo-component:** Required for building WebAssembly components from Rust source.

### Setup Commands

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the Wasm target
rustup target add wasm32-wasip2

# Install cargo-component for building Wasm components
cargo install cargo-component

# Clone the repository
git clone https://github.com/torvyn/torvyn.git
cd torvyn

# Build the entire workspace
cargo build --workspace

# Run the full test suite
cargo test --workspace

# Run the lint checks
cargo clippy --workspace --all-targets -- -D warnings

# Verify formatting
cargo fmt --all -- --check
```

### IDE Recommendations

Torvyn development works well with any editor that supports rust-analyzer. The repository includes configuration files for VS Code (`.vscode/settings.json`) with recommended settings and extensions.

---

## Coding Standards

### Rust Style

- **Formatting:** All Rust code must pass `cargo fmt` with the project's `rustfmt.toml` configuration. Run `cargo fmt --all` before committing.
- **Linting:** All code must pass `cargo clippy --workspace --all-targets -- -D warnings` with zero warnings.
- **Naming:** Follow standard Rust naming conventions: `snake_case` for functions and variables, `CamelCase` for types and traits, `SCREAMING_SNAKE_CASE` for constants.
- **Error handling:** Use `Result` and `Error` types from `torvyn-types`. Never use `.unwrap()` or `.expect()` in library code. Panics are acceptable only in test code and in cases where the invariant violation indicates a bug in Torvyn itself (document these with a comment explaining why the invariant holds).
- **Unsafe code:** Minimize and isolate. Every `unsafe` block must have a `// SAFETY:` comment explaining why the operation is sound. Unsafe code requires review from a maintainer.

### Documentation Requirements

- Every public type, trait, function, and module must have a doc comment (`///`).
- Doc comments should explain **what** the item does, **why** it exists, and **when** to use it. Include code examples for complex APIs.
- Modules should have a top-level `//!` doc comment explaining the module's purpose and its relationship to the Torvyn architecture.

### Test Requirements

- Every bug fix must include a regression test.
- New features must include unit tests and, where applicable, integration tests.
- Performance-sensitive code paths should include benchmarks (using `criterion`).
- Tests should be deterministic. Avoid time-dependent tests where possible; use simulated time when testing async scheduling.

### Commit Message Format

Torvyn uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `test`: Adding or updating tests
- `bench`: Benchmark changes
- `refactor`: Code restructuring without behavior change
- `perf`: Performance improvement
- `ci`: CI/CD changes
- `chore`: Maintenance tasks (dependency updates, tooling)

**Scope:** The affected crate or subsystem (e.g., `reactor`, `resources`, `contracts`, `cli`, `host`).

**Examples:**
```
feat(reactor): add weighted fairness scheduling policy

fix(resources): prevent double-free on buffer pool return

docs(contracts): add WIT evolution examples to versioning guide

perf(reactor): reduce wakeup overhead with batched demand signals
```

The first line must be 72 characters or fewer. The body should explain **why** the change is being made, not just what changed.

---

## Pull Request Process

### 1. Fork and Branch

Fork the repository and create a branch from `main`:

```bash
git checkout -b feat/my-feature main
```

Branch naming convention: `<type>/<short-description>` (e.g., `fix/buffer-leak`, `feat/weighted-scheduling`, `docs/getting-started-update`).

### 2. Implement

Write your code following the coding standards above. Keep commits focused — each commit should represent a single logical change.

### 3. Test

Run the full test suite before submitting:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

If your change is performance-sensitive, include benchmark results showing the impact:

```bash
cargo bench -p torvyn-reactor  # example for reactor changes
```

### 4. Submit

Open a pull request against `main`. The PR description should include:

- **What** this PR does (a concise summary).
- **Why** this change is needed (link to an issue if applicable).
- **How** it works (brief explanation of the approach, especially for non-obvious designs).
- **Testing** (what tests were added or modified, and how to verify).
- **Breaking changes** (if any, with migration guidance).

### 5. Review

All pull requests require at least one approving review from a committer or maintainer. For changes to performance-critical paths (reactor, resource transfers, engine dispatch), a maintainer review is required.

**What reviewers look for:**
- Correctness: Does the change do what it claims?
- Safety: Is unsafe code justified and sound? Are error cases handled?
- Performance: Does the change affect hot-path performance? Are there unnecessary allocations or copies?
- Contracts: Does the change respect the existing contract surface? If it changes contracts, has an RFC been filed?
- Tests: Are the tests sufficient? Do they cover edge cases?
- Documentation: Are public APIs documented? Are complex logic paths commented?
- Style: Does the code follow project conventions?

**Typical turnaround time:** Maintainers aim to provide initial review feedback within 5 business days. Complex changes may take longer. If your PR has not received attention after 7 days, it is appropriate to leave a polite comment requesting review.

### 6. Merge

Once approved and CI passes, a committer or maintainer merges the PR. We use squash-merge for feature branches to keep the main branch history clean. The squash commit message should follow the Conventional Commits format.

---

## Architecture Overview for New Contributors

Torvyn is organized as a Cargo workspace with focused crates:

| Crate | Purpose | Hot/Cold Path |
|-------|---------|---------------|
| `torvyn-types` | Shared type definitions (`ComponentId`, `FlowId`, etc.) | N/A (leaf) |
| `torvyn-config` | Configuration parsing and validation | Cold |
| `torvyn-contracts` | WIT loading, validation, compatibility checking | Cold |
| `torvyn-engine` | Wasm engine abstraction (Wasmtime implementation) | Hot |
| `torvyn-resources` | Buffer pools, ownership tracking, data transfer | Hot |
| `torvyn-reactor` | Stream scheduling, backpressure, demand propagation | Hot |
| `torvyn-observability` | Tracing, metrics, diagnostics (OpenTelemetry) | Hot (metrics) / Warm (traces) |
| `torvyn-security` | Capability model and enforcement | Cold (grant resolution) / Hot (checks) |
| `torvyn-linker` | Component linking and composition | Cold |
| `torvyn-pipeline` | Pipeline topology definition and instantiation | Cold |
| `torvyn-host` | Binary entry point, lifecycle orchestration | Cold |
| `torvyn-cli` | CLI frontend | Cold |

For a deeper understanding, see the [Architecture Guide](documents/ARCHITECTURE.md) and the design documents in `docs/design/`.

---

## Good First Issues

Issues labeled `good-first-issue` are specifically selected for new contributors. These issues:

- Have a clear scope and well-defined acceptance criteria.
- Include a pointer to the relevant code area.
- Do not require deep architectural knowledge.
- Have a maintainer available to provide guidance.

If you are picking up a good-first-issue, leave a comment on the issue to let others know you are working on it. If you get stuck, ask questions directly on the issue — maintainers will respond.

---

## Communication Channels

- **GitHub Issues:** For bug reports, feature requests, and task tracking.
- **GitHub Discussions:** For design questions, architecture discussion, help requests, and general conversation.
- **RFC Pull Requests:** For proposing significant changes (see [GOVERNANCE.md](GOVERNANCE.md)).

---

## Recognition

All contributors are acknowledged in the project. Significant contributions are highlighted in release notes. We believe that every contribution — code, documentation, bug reports, design review — deserves recognition.

---

## Questions?

If anything in this guide is unclear, open a GitHub Discussion or file an issue. We want contributing to Torvyn to be a good experience, and we will improve this guide based on feedback.
