//! Integration test: real Component-Model component instantiation + init.
//!
//! Loads `tests/fixtures/init_smoke.wasm` (a `data-sink`-world component
//! built by `cargo component build`; see `tests/fixtures/build.sh`),
//! compiles it through [`WasmtimeEngine`], instantiates it via the bindgen
//! plumbing introduced in Session 2.1, and invokes
//! `lifecycle.init` through the typed [`WasmtimeInvoker`] path.

#![cfg(feature = "wasmtime-backend")]

use torvyn_engine::{
    ComponentInvoker, WasiConfiguration, WasmEngine, WasmtimeEngine, WasmtimeEngineConfig,
    WasmtimeInvoker,
};
use torvyn_types::ComponentId;

const FIXTURE_BYTES: &[u8] = include_bytes!("fixtures/init_smoke.wasm");

#[tokio::test]
async fn data_sink_fixture_instantiates_and_runs_init() {
    let engine = WasmtimeEngine::new(WasmtimeEngineConfig::default()).expect("engine creation");

    let compiled = engine
        .compile_component(FIXTURE_BYTES)
        .expect("compile init_smoke.wasm");

    let imports = engine.default_imports();
    let component_id = ComponentId::new(1);

    let mut instance = engine
        .instantiate(
            &compiled,
            imports,
            component_id,
            &WasiConfiguration::deny_all(),
        )
        .await
        .expect("instantiate data-sink component");

    // The fixture targets the `data-sink` world, which exports `sink` and
    // `lifecycle`. Detection should populate exactly those archetype flags.
    assert!(
        instance.has_sink(),
        "data-sink component should be detected as a sink"
    );
    assert!(
        instance.has_lifecycle(),
        "data-sink component should expose lifecycle hooks"
    );
    assert!(
        !instance.has_source(),
        "data-sink fixture must not register as a source"
    );
    assert!(
        !instance.has_processor(),
        "data-sink fixture must not register as a processor"
    );

    let invoker = WasmtimeInvoker::new();
    invoker
        .invoke_init(&mut instance, component_id, "{}")
        .await
        .expect("lifecycle.init returns Ok(())");

    invoker.invoke_teardown(&mut instance, component_id).await;
}
