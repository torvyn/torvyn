//! Compile the Wasm Component Model test fixtures used by the
//! `test_real_wasm_e2e` and `test_polyglot_e2e` integration tests.
//!
//! Two toolchains are exercised:
//!
//! 1. **`cargo-component`** — builds the Rust source/processor/sink
//!    fixtures (`echo-source`, `identity-processor`, `echo-sink`)
//!    under `tests/test-components/`.
//! 2. **TinyGo + `wit-bindgen-go`** — builds the Go source fixture
//!    (`go-echo-source`) for the polyglot proof.
//!
//! Each toolchain's presence is probed independently; missing
//! toolchains emit `cargo:warning=…` and skip the corresponding
//! component. The test files that depend on a particular fixture are
//! gated behind Cargo features (`wasm-e2e` for the Rust pipeline,
//! `wasm-polyglot` for the Go-source variant), so a plain
//! `cargo test --workspace` on a checkout without either toolchain
//! still passes.
//!
//! # Prerequisites for a successful build
//!
//! For the Rust pipeline (`--features wasm-e2e`):
//! - `rustup target add wasm32-wasip2`
//! - `cargo install cargo-component --locked`
//!
//! For the polyglot pipeline (`--features wasm-polyglot`):
//! - The above, plus
//! - `tinygo` (https://tinygo.org)
//! - `wit-bindgen-go` (`go install go.bytecodealliance.org/cmd/wit-bindgen-go@latest`)
//!
//! # Escape hatch
//!
//! Setting `TORVYN_SKIP_WASM_BUILD=1` short-circuits this script
//! entirely.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A Rust test component built via `cargo component build`.
const RUST_COMPONENTS: &[(&str, &str)] = &[
    // (component-directory-name, generated crate name as Cargo emits
    //  it on the wasm32-wasip2 target — derived from the [package]
    //  name with hyphens replaced by underscores)
    ("echo-source", "test_echo_source"),
    ("identity-processor", "test_identity_processor"),
    ("echo-sink", "test_echo_sink"),
];

/// Directory name of the Go component (under
/// `tests/test-components/`).
const GO_COMPONENT_DIR: &str = "go-echo-source";

/// Name of the wrapper world declared in the Go component's
/// `wit/world.wit`. The wrapper extends the canonical `data-source`
/// world with the WASI Preview-2 imports that TinyGo's stdlib drags
/// in; the host runtime traps every WASI call at link time, so the
/// guest behaves identically to the Rust source from a copy-accounting
/// standpoint.
const GO_WRAPPER_WORLD: &str = "data-source-with-wasi";

fn main() {
    println!("cargo:rerun-if-env-changed=TORVYN_SKIP_WASM_BUILD");

    if std::env::var("TORVYN_SKIP_WASM_BUILD").is_ok() {
        println!(
            "cargo:warning=TORVYN_SKIP_WASM_BUILD set; skipping Wasm component build. \
             Wasm test targets (--features wasm-e2e, --features wasm-polyglot) will fail at runtime."
        );
        return;
    }

    let manifest_dir = PathBuf::from(env_or_panic("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env_or_panic("OUT_DIR"));
    let components_root = manifest_dir
        .parent()
        .expect("integration crate must have a parent directory")
        .join("test-components");
    let contracts_wit = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|root| root.join("crates/torvyn-contracts/wit/torvyn-streaming"))
        .expect("canonical WIT directory must resolve");

    if cargo_component_available() {
        for (dir_name, crate_name) in RUST_COMPONENTS {
            build_rust_component(&components_root, &out_dir, dir_name, crate_name);
        }
    } else {
        println!(
            "cargo:warning=cargo-component is not installed; skipping Rust component build. \
             Run `cargo install cargo-component --locked` and `rustup target add wasm32-wasip2` \
             to enable the `--features wasm-e2e` integration test."
        );
    }

    if tinygo_available() && wit_bindgen_go_available() {
        if let Some(tinygoroot) = tinygo_root() {
            build_go_component(&components_root, &contracts_wit, &out_dir, &tinygoroot);
        } else {
            println!(
                "cargo:warning=`tinygo env TINYGOROOT` did not return a usable path; \
                 skipping Go component build."
            );
        }
    } else {
        println!(
            "cargo:warning=tinygo or wit-bindgen-go is not installed; skipping Go component build. \
             Install TinyGo (https://tinygo.org) and run \
             `go install go.bytecodealliance.org/cmd/wit-bindgen-go@latest` \
             to enable the `--features wasm-polyglot` integration test."
        );
    }
}

// ---------------------------------------------------------------------------
// Rust components
// ---------------------------------------------------------------------------

fn build_rust_component(components_root: &Path, out_dir: &Path, dir_name: &str, crate_name: &str) {
    let src_dir = components_root.join(dir_name);
    let manifest = src_dir.join("Cargo.toml");
    let lib_rs = src_dir.join("src").join("lib.rs");
    let target_dir = out_dir.join("wasm-components-target");

    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", lib_rs.display());

    let status = Command::new("cargo")
        .args([
            "component",
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--target-dir",
        ])
        .arg(&target_dir)
        .args(["--manifest-path"])
        .arg(&manifest)
        .status();

    match status {
        Ok(s) if s.success() => {}
        _ => {
            println!(
                "cargo:warning=cargo-component build failed for '{dir_name}'; \
                 the `--features wasm-e2e` integration test will not be runnable. \
                 Ensure `wasm32-wasip2` is installed for the active toolchain \
                 (`rustup target add wasm32-wasip2`)."
            );
            return;
        }
    }

    // cargo-component currently emits .wasm under `wasm32-wasip1/release/`
    // even when the requested target is `wasm32-wasip2` — the binary is
    // adapted internally and still a valid Component Model artefact.
    // Probe both paths so this stays robust if a future cargo-component
    // release changes the output layout.
    let candidates = [
        target_dir
            .join("wasm32-wasip2")
            .join("release")
            .join(format!("{crate_name}.wasm")),
        target_dir
            .join("wasm32-wasip1")
            .join("release")
            .join(format!("{crate_name}.wasm")),
    ];
    let Some(wasm_path) = candidates.iter().find(|p| p.exists()) else {
        println!(
            "cargo:warning=cargo-component succeeded but produced no .wasm for '{dir_name}' \
             under any of: {candidates:?}; the `--features wasm-e2e` target will not be runnable."
        );
        return;
    };

    let env_key = env_key_for(dir_name);
    println!("cargo:rustc-env={env_key}={}", wasm_path.display());
}

// ---------------------------------------------------------------------------
// Go component
// ---------------------------------------------------------------------------

fn build_go_component(
    components_root: &Path,
    contracts_wit: &Path,
    out_dir: &Path,
    tinygoroot: &Path,
) {
    let src_dir = components_root.join(GO_COMPONENT_DIR);
    let main_go = src_dir.join("main.go");
    let go_mod = src_dir.join("go.mod");
    let world_wit = src_dir.join("wit").join("world.wit");
    let bindings_dir = src_dir.join("internal");
    let staged_wit = src_dir.join("wit");
    let staged_deps = staged_wit.join("deps");

    println!("cargo:rerun-if-changed={}", main_go.display());
    println!("cargo:rerun-if-changed={}", go_mod.display());
    println!("cargo:rerun-if-changed={}", world_wit.display());
    println!("cargo:rerun-if-changed={}", contracts_wit.display());

    // Stage the WIT deps tree the wrapper world depends on. The
    // canonical `torvyn:streaming` contracts come from the contracts
    // crate; the WASI Preview-2 contracts come from TinyGo's bundle so
    // we don't need a registry-backed `wkg wit fetch` at build time.
    if let Err(e) = stage_go_wit_deps(contracts_wit, tinygoroot, &staged_deps) {
        println!(
            "cargo:warning=Failed to stage WIT dependencies for the Go component: {e}; \
             the `--features wasm-polyglot` target will not be runnable."
        );
        return;
    }

    // Generate Go bindings from the canonical WIT. We point
    // `wit-bindgen-go` at the contracts directory directly (rather
    // than the staged wrapper) so the generated package paths match the
    // import paths hard-coded in `main.go`.
    if let Err(e) = std::fs::create_dir_all(&bindings_dir) {
        println!(
            "cargo:warning=Failed to create Go bindings directory {}: {e}; \
             the `--features wasm-polyglot` target will not be runnable.",
            bindings_dir.display(),
        );
        return;
    }
    let bindings_status = Command::new("wit-bindgen-go")
        .args(["generate", "--world", "data-source", "--out"])
        .arg(&bindings_dir)
        .args([
            "--package-root",
            "torvyn.dev/test-components/go-echo-source/internal",
        ])
        .arg(contracts_wit)
        .status();
    match bindings_status {
        Ok(s) if s.success() => {}
        _ => {
            println!(
                "cargo:warning=wit-bindgen-go failed; \
                 the `--features wasm-polyglot` target will not be runnable."
            );
            return;
        }
    }

    let output_wasm = out_dir.join("go_echo_source.wasm");

    // `-scheduler=none` is essential. The default asyncify scheduler
    // allocates per-goroutine task state during `_initialize`, which
    // for trivial single-goroutine components like ours wastes a
    // surprising amount of memory and trips wasmtime's
    // `trap_on_grow_failure` against the engine's 16 MiB store limit
    // on first GC pass. Forcing `scheduler=none` matches the
    // "single-shot guest function call" semantics the Component Model
    // already enforces and shrinks startup memory to a few hundred
    // KB.
    let tinygo_status = Command::new("tinygo")
        .args(["build", "-target=wasip2", "-scheduler=none", "-o"])
        .arg(&output_wasm)
        .arg("--wit-package")
        .arg(&staged_wit)
        .args(["--wit-world", GO_WRAPPER_WORLD, "."])
        .current_dir(&src_dir)
        .status();
    match tinygo_status {
        Ok(s) if s.success() => {}
        _ => {
            println!(
                "cargo:warning=tinygo build failed for the Go component; \
                 the `--features wasm-polyglot` target will not be runnable."
            );
            return;
        }
    }

    println!(
        "cargo:rustc-env=TORVYN_GO_ECHO_SOURCE_WASM={}",
        output_wasm.display()
    );
}

/// Mirror the canonical Torvyn WIT and TinyGo's WASI Preview-2 WIT into
/// the Go component's `wit/deps/` directory, where TinyGo expects to
/// resolve the wrapper world's `include` and `import` references.
fn stage_go_wit_deps(
    contracts_wit: &Path,
    tinygoroot: &Path,
    staged_deps: &Path,
) -> std::io::Result<()> {
    // Reset the deps tree so a previous run's stale dep doesn't shadow
    // an intentional removal.
    if staged_deps.exists() {
        std::fs::remove_dir_all(staged_deps)?;
    }
    std::fs::create_dir_all(staged_deps)?;

    // Canonical `torvyn:streaming` contracts.
    let dst_streaming = staged_deps.join("torvyn-streaming");
    copy_wit_dir(contracts_wit, &dst_streaming)?;

    // WASI Preview-2 contracts bundled by TinyGo. The wasi-cli world
    // files live at the top of `wit/`; the dependent WASI packages live
    // under `wit/deps/`.
    let wasi_cli_wit = tinygoroot.join("lib").join("wasi-cli").join("wit");

    // wasi:cli/* goes into deps/cli/
    let dst_cli = staged_deps.join("cli");
    std::fs::create_dir_all(&dst_cli)?;
    for entry in std::fs::read_dir(&wasi_cli_wit)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("wit") {
            let dst = dst_cli.join(path.file_name().expect("wit file has a name"));
            std::fs::copy(&path, &dst)?;
        }
    }

    // Remaining wasi:io/clocks/filesystem/random/sockets dependencies
    // live under `wit/deps/`.
    let wasi_deps_root = wasi_cli_wit.join("deps");
    for entry in std::fs::read_dir(&wasi_deps_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let dst = staged_deps.join(path.file_name().expect("deps subdir has a name"));
            copy_wit_dir(&path, &dst)?;
        }
    }

    Ok(())
}

/// Recursive copy of `.wit` files, recreating subdirectory structure.
fn copy_wit_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_wit_dir(&path, &dest)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("wit") {
            std::fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Toolchain probing
// ---------------------------------------------------------------------------

fn env_or_panic(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("expected environment variable {key} to be set by Cargo"))
}

fn env_key_for(dir_name: &str) -> String {
    format!("TORVYN_{}_WASM", dir_name.to_uppercase().replace('-', "_"))
}

fn cargo_component_available() -> bool {
    Command::new("cargo")
        .args(["component", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tinygo_available() -> bool {
    Command::new("tinygo")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wit_bindgen_go_available() -> bool {
    Command::new("wit-bindgen-go")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tinygo_root() -> Option<PathBuf> {
    let output = Command::new("tinygo")
        .args(["env", "TINYGOROOT"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    // `tinygo env TINYGOROOT` prints `TINYGOROOT="/path"`. Strip the
    // shell-style quoting if present.
    let trimmed = raw.trim();
    let cleaned = trimmed.trim_start_matches("TINYGOROOT=").trim_matches('"');
    if cleaned.is_empty() {
        None
    } else {
        Some(PathBuf::from(cleaned))
    }
}
