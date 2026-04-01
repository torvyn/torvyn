# Release Process

This document describes how Torvyn releases are made, versioned, and maintained.

## Version Number Conventions

Torvyn follows strict Semantic Versioning 2.0.0 (semver.org) for all crate versions.

```
MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]
```

- **MAJOR** — Incremented for breaking changes to public APIs, WIT interface changes that alter ownership semantics or remove existing types, or configuration format changes that are not backward-compatible.
- **MINOR** — Incremented for additive changes: new functions, new interfaces, new CLI commands, new configuration options with defaults that preserve existing behavior.
- **PATCH** — Incremented for bug fixes, documentation corrections, and performance improvements that do not change public API behavior.
- **Pre-release** — Versions with identifiers like `-alpha.1`, `-beta.2`, `-rc.1` are not considered stable and are excluded from dependency resolution by default.

All crates in the workspace share a single version number. When any crate has a change that warrants a version bump, all crates are bumped together. This simplifies dependency management and avoids version matrix confusion.

During Phase 0, all releases carry the `0.x.y` version range. A `0.x.y` version signals that the API is not yet stable and breaking changes may occur in minor versions.

## What Triggers a Release

Releases are triggered by the maintainer(s) when one of the following conditions is met:

- A milestone is completed (e.g., Phase 0 Source→Sink pipeline working).
- A security vulnerability is patched.
- A sufficient number of improvements have accumulated since the last release.
- A critical bug fix that affects production users.

Releases are not made on a fixed calendar schedule. Quality and completeness take priority over cadence.

## How Releases Are Made

### 1. Pre-release Verification

Before tagging a release, verify:

```bash
# All tests pass
cargo test --workspace

# No clippy warnings
cargo clippy --workspace --all-targets -- -D warnings

# Documentation builds cleanly
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Format is correct
cargo fmt --all -- --check

# Benchmarks run without regression (compare against previous release)
cargo bench --workspace
```

### 2. Update Version Numbers

Update the version in the workspace `Cargo.toml` and all crate `Cargo.toml` files. Update the `CHANGELOG.md` with a summary of changes since the last release, organized by category (Added, Changed, Fixed, Removed, Security).

### 3. Create Release Commit and Tag

```bash
git add -A
git commit -m "chore(release): prepare v0.2.0"
git tag -s v0.2.0 -m "Release v0.2.0"
git push origin main --tags
```

### 4. Publish to crates.io

Crates are published in dependency order:

```bash
cargo publish -p torvyn-types
cargo publish -p torvyn-contracts
cargo publish -p torvyn-config
cargo publish -p torvyn-engine
cargo publish -p torvyn-observability
cargo publish -p torvyn-resources
cargo publish -p torvyn-security
cargo publish -p torvyn-linker
cargo publish -p torvyn-reactor
cargo publish -p torvyn-pipeline
cargo publish -p torvyn-packaging
cargo publish -p torvyn-host
cargo publish -p torvyn-cli
```

### 5. Create GitHub Release

Create a GitHub release from the tag with the CHANGELOG entry as the release notes.

## How to Backport Fixes

For critical bug fixes or security patches that need to apply to an older release:

1. Create a branch from the release tag: `git checkout -b release/0.1.x v0.1.0`
2. Cherry-pick the fix commit: `git cherry-pick <commit-hash>`
3. Bump the patch version in all `Cargo.toml` files.
4. Run the full verification suite.
5. Tag and publish: `git tag -s v0.1.1 -m "Release v0.1.1"`

Backport branches are maintained only for the most recent minor release. Older releases receive backports only for security fixes.

## Release Checklist

- [ ] All CI checks pass on `main`
- [ ] `CHANGELOG.md` updated with categorized changes
- [ ] Version numbers updated in all `Cargo.toml` files
- [ ] Benchmarks show no unexpected regressions
- [ ] Release commit created and signed
- [ ] Git tag created and signed
- [ ] Tag pushed to origin
- [ ] Crates published to crates.io in dependency order
- [ ] GitHub release created with changelog
- [ ] Announcement posted (if applicable)
