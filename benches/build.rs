//! Build script for `torvyn-benchmarks`.
//!
//! Compiles `proto/echo.proto` into Rust types via `tonic-build`, using a
//! hermetic vendored `protoc` from `protoc-bin-vendored`. This avoids the
//! "user/CI must pre-install protobuf-compiler" footgun — `cargo bench`
//! works out of the box on any platform supported by `protoc-bin-vendored`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Point `prost-build` (transitively used by `tonic-build`) at the vendored
    // `protoc` binary. Setting `PROTOC` is honored by `prost-build` regardless
    // of platform.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);

    // Generated Rust lives under `OUT_DIR`; the benchmark consumes it via
    // `tonic::include_proto!("torvyn.bench.echo")`.
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(&["proto/echo.proto"], &["proto"])?;

    // Re-run only when the .proto changes. Without this, every build re-runs
    // build.rs (slowing incremental compiles).
    println!("cargo:rerun-if-changed=proto/echo.proto");
    println!("cargo:rerun-if-changed=build.rs");

    Ok(())
}
