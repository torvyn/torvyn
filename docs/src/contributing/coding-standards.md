# Coding Standards

These standards apply to all code contributed to the Torvyn project. They supplement `rustfmt` and `clippy` — those tools enforce mechanical formatting and common lints. This document covers the conventions, patterns, and policies that tools cannot enforce automatically.

## Rust Style

### Naming Conventions

Torvyn follows standard Rust naming conventions with the following project-specific additions:

- **Identity types** are suffixed with `Id`: `ComponentId`, `FlowId`, `StreamId`, `ResourceId`.
- **Configuration structs** are suffixed with `Config`: `WasmtimeEngineConfig`, `FlowConfig`, `TierConfig`.
- **Builder types** are suffixed with `Builder`: `HostBuilder`, `PipelineTopologyBuilder`.
- **Error types** are suffixed with `Error`: `EngineError`, `FlowCreationError`, `LinkerError`.
- **Trait implementations** for the primary production implementation use the `Default` prefix: `DefaultResourceManager`, `DefaultSandboxConfigurator`. This leaves the trait name clean for the abstraction.

### Module Organization

Each crate follows a consistent internal structure:

1. `src/lib.rs` — Crate root. Contains `#![forbid(unsafe_code)]` (unless the crate has justified unsafe), `#![deny(missing_docs)]`, and re-exports of the public API. No logic beyond re-exports.
2. `src/error.rs` or `src/errors.rs` — All crate-specific error types, grouped in one place.
3. Domain modules — one file per major concept. Keep files under 500 lines when possible. If a module grows beyond 800 lines, consider splitting it.
4. `tests/` — Integration tests in the standard Cargo test directory.

### Error Handling Patterns

Torvyn uses a layered error model:

- **Crate-level errors** (e.g., `EngineError`, `LinkerError`) — defined per crate using `thiserror` derives. Every variant has a `Display` implementation that produces a complete, human-readable message.
- **Cross-crate error type** — `TorvynError` in `torvyn-types` acts as the top-level error. Every crate-level error implements `From<CrateError> for TorvynError`.
- **Error codes** — Error types that correspond to user-facing diagnostics include structured error codes (e.g., `E0100`–`E0199` for contract errors). These codes appear in CLI output and documentation.
- **Never use `unwrap()` or `expect()` in library code.** These are acceptable only in tests and in CLI `main()` paths where the error is immediately displayed to the user.
- **Use `?` for propagation.** Every fallible function returns `Result<T, E>` where `E` is either the crate-level error or `TorvynError`.

### Documentation Standards

Every public item must have a documentation comment. The `#![deny(missing_docs)]` lint is enabled on all crates and enforced in CI.

Documentation comments follow this structure:

```rust
/// One-sentence summary of what this type/function does.
///
/// Longer explanation if the summary is not sufficient. Include
/// context about when and why this is used.
///
/// # Invariants (for types)
/// - List any invariants that must hold.
///
/// # Errors (for fallible functions)
/// Returns `SomeError::Variant` if the specific condition occurs.
///
/// # Panics (only if the function can panic)
/// Panics if the specific condition occurs.
///
/// # Examples
/// ```
/// // At least one runnable example for types and public functions.
/// ```
```

## Performance Annotations

Torvyn uses inline comments to mark code by its performance sensitivity. This helps reviewers and future contributors understand which code is latency-critical.

- **`// HOT PATH`** — Executes for every stream element. Must not allocate, must not lock, must not block. Observability overhead budget: 500ns per element at Production level.
- **`// WARM PATH`** — Executes per scheduling cycle or per backpressure event. Allocation is acceptable if amortized. Locks are acceptable if uncontended.
- **`// COLD PATH`** — Executes during startup, shutdown, or configuration changes. No performance constraints.

When adding code to a function marked `HOT PATH`, you must verify that your change does not introduce allocations or blocking. When in doubt, benchmark.

## Test Writing Standards

### Test Naming

Test names use the pattern `test_<unit>_<condition>_<expected_outcome>`:

```rust
#[test]
fn test_flow_state_transition_from_running_to_draining_succeeds() { ... }

#[test]
fn test_resource_id_stale_generation_returns_error() { ... }
```

### Test Structure

Tests follow the Arrange-Act-Assert pattern:

```rust
#[test]
fn test_buffer_pool_allocates_from_correct_tier() {
    // Arrange
    let pool = BufferPoolSet::new(default_tier_config());

    // Act
    let handle = pool.allocate(256).unwrap();

    // Assert
    assert_eq!(handle.tier(), PoolTier::Small);
}
```

### Coverage Expectations

There is no hard coverage percentage target, but every public function should have at least one test that exercises its happy path and one that exercises its primary error path. State machines (`FlowState`, `ResourceState`) must have tests for every valid transition and at least one test for each invalid transition.

## Commit Message Format

Torvyn uses [Conventional Commits](https://www.conventionalcommits.org/). Every commit message follows this format:

```
<type>(<scope>): <short description>

<optional body>

<optional footer>
```

**Types:**

- `feat` — A new feature or capability
- `fix` — A bug fix
- `perf` — A performance improvement
- `refactor` — Code restructuring without behavior change
- `test` — Adding or updating tests
- `docs` — Documentation changes
- `chore` — Build system, CI, dependencies, or tooling changes
- `ci` — CI configuration changes

**Scope** is the crate name without the `torvyn-` prefix: `types`, `contracts`, `config`, `engine`, `resources`, `reactor`, `observability`, `security`, `linker`, `pipeline`, `packaging`, `host`, `cli`.

Examples:

```
feat(reactor): add weighted fair queuing policy
fix(resources): prevent double-free on stale buffer handle
perf(engine): cache compiled components by content hash
docs(contracts): document WIT interface evolution rules
test(types): add property tests for state machine transitions
chore(deps): update wasmtime to 26.0.0
```

## Pull Request Description Standards

Every pull request description must include:

1. **Summary** — one or two sentences describing the change.
2. **Motivation** — why this change is needed. Link to the issue if one exists.
3. **Design decisions** — if the change involves a non-obvious design choice, explain it. If you considered alternatives, note them briefly.
4. **Testing** — what tests were added or updated. How to verify the change manually if applicable.
5. **Performance impact** — if the change touches hot-path code, describe the expected impact and any benchmark results.

## Unsafe Code Policy

Unsafe code is forbidden by default. The `#![forbid(unsafe_code)]` attribute is set on all crates except `torvyn-resources`, which requires unsafe for buffer allocation and deallocation.

If you need to add unsafe code:

1. **Justify it.** Explain in the PR description why safe alternatives are insufficient.
2. **Isolate it.** Unsafe code must live in a dedicated module, never mixed with safe logic.
3. **Document every block.** Every `unsafe` block must have a `// SAFETY:` comment that explains why the specific safety invariants are upheld.
4. **Test the invariants.** Every unsafe block must have corresponding tests that exercise the boundary conditions.
5. **Minimize scope.** The unsafe block should contain the absolute minimum number of statements necessary.

Example of acceptable unsafe documentation:

```rust
// SAFETY: `ptr` is valid because it was allocated by `alloc::alloc` in
// `BufferPool::allocate` with a layout of `self.capacity` bytes, and
// `self.capacity` is guaranteed > 0 by the TierConfig constructor.
// The allocation has not been freed because this function holds the
// only `BufferEntry` that references it, and `BufferEntry::drop`
// is the only code path that frees it.
unsafe { std::ptr::write_bytes(ptr, 0, self.capacity) };
```

## Dependency Addition Policy

Adding a new external dependency requires justification in the PR description. The following must be addressed:

1. **Purpose** — what does this dependency provide that cannot be done with existing dependencies or standard library code?
2. **License** — the dependency must be compatible with Apache-2.0. Acceptable licenses: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib. Any other license requires explicit approval from a maintainer.
3. **Maintenance status** — is the crate actively maintained? When was the last release? Are there open security advisories?
4. **Size impact** — does this dependency pull in a large transitive dependency tree?
5. **Feature flags** — enable only the features you need. Do not enable default features unless all of them are required.

Run `cargo deny check` (if configured) to verify license compliance before submitting.
