//! Compile the Wasm Component Model test fixtures used by the
//! `test_real_wasm_e2e` integration test.
//!
//! Runs `cargo component build --release --target wasm32-wasip2` for
//! each of `echo-source`, `identity-processor`, and `echo-sink`, then
//! emits a `cargo:rustc-env` for each so the test can locate the
//! produced `.wasm` via `env!()`.
//!
//! # Prerequisites for a successful build
//! - `rustup target add wasm32-wasip2`
//! - `cargo install cargo-component --locked` (or via
//!   `taiki-e/cache-cargo-install-action`)
//!
//! # Escape hatch
//! Setting `TORVYN_SKIP_WASM_BUILD=1` short-circuits this script.
//! The new test target is also gated behind `--features wasm-e2e`, so a
//! plain `cargo test --workspace` from a checkout without
//! `cargo-component` installed is a no-op here.

use std::path::PathBuf;
use std::process::Command;

const COMPONENTS: &[(&str, &str)] = &[
    // (component-directory-name, generated crate name as Cargo emits it
    //  on the wasm32-wasip2 target — derived from the [package] name
    //  with hyphens replaced by underscores)
    ("echo-source", "test_echo_source"),
    ("identity-processor", "test_identity_processor"),
    ("echo-sink", "test_echo_sink"),
];

fn main() {
    // Always re-emit the env-var rerun trigger so build.rs runs when
    // the developer toggles the escape hatch.
    println!("cargo:rerun-if-env-changed=TORVYN_SKIP_WASM_BUILD");

    if std::env::var("TORVYN_SKIP_WASM_BUILD").is_ok() {
        println!(
            "cargo:warning=TORVYN_SKIP_WASM_BUILD set; skipping Wasm component build. \
             The `test_real_wasm_e2e` test target (--features wasm-e2e) will fail at runtime."
        );
        return;
    }

    if !cargo_component_available() {
        println!(
            "cargo:warning=cargo-component is not installed; skipping Wasm component build. \
             Run `cargo install cargo-component --locked` and `rustup target add wasm32-wasip2` \
             to enable the `--features wasm-e2e` integration test."
        );
        return;
    }

    let manifest_dir = PathBuf::from(env_or_panic("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env_or_panic("OUT_DIR"));
    let target_dir = out_dir.join("wasm-components-target");

    let components_root = manifest_dir
        .parent()
        .expect("integration crate must have a parent directory")
        .join("test-components");

    for (dir_name, crate_name) in COMPONENTS {
        let src_dir = components_root.join(dir_name);
        let manifest = src_dir.join("Cargo.toml");
        let lib_rs = src_dir.join("src").join("lib.rs");

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
                // Soft-fail so `cargo check --workspace` / `cargo +1.91.0
                // check --workspace` still pass on machines that don't
                // have the `wasm32-wasip2` target installed for every
                // toolchain. The `wasm-e2e` test target's `env!()` calls
                // give a clear, late error if a developer tries to run
                // it without the prerequisites.
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
        // adapted internally and still a valid Component Model artifact.
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
        let wasm_path = candidates.iter().find(|p| p.exists()).unwrap_or_else(|| {
            panic!(
                "cargo-component succeeded but produced no .wasm for '{dir_name}' \
                     under either {candidates:?}",
            )
        });

        let env_key = format!("TORVYN_{}_WASM", dir_name.to_uppercase().replace('-', "_"));
        println!("cargo:rustc-env={env_key}={}", wasm_path.display());
    }
}

fn env_or_panic(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("expected environment variable {key} to be set by Cargo"))
}

fn cargo_component_available() -> bool {
    Command::new("cargo")
        .args(["component", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
