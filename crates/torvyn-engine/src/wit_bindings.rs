//! Wasmtime [`bindgen!`] outputs for the four Torvyn streaming worlds.
//!
//! Each world (`data-source`, `data-sink`, `transform`, `managed-transform`)
//! gets its own submodule. The first invocation (`data_sink`) generates the
//! canonical `torvyn::streaming::types` interface bindings; the subsequent
//! invocations redirect to that module via `with:` so the generated `Host`
//! traits are deduplicated and `HostState` only needs one implementation per
//! interface.
//!
//! The `data_source` invocation similarly produces the canonical
//! `torvyn::streaming::buffer_allocator` bindings; `transform` and
//! `managed_transform` redirect to it.
//!
//! All imports run async and are trappable so the host can surface allocation
//! failures and resource-table errors as Wasm traps.
//!
//! [`bindgen!`]: wasmtime::component::bindgen

#![allow(missing_docs)]

pub(crate) mod data_sink {
    wasmtime::component::bindgen!({
        path: "../torvyn-contracts/wit/torvyn-streaming",
        world: "data-sink",
        imports: { default: async | trappable },
        exports: { default: async },
        with: {
            "torvyn:streaming/types.buffer": crate::host_state::HostBuffer,
            "torvyn:streaming/types.mutable-buffer":
                crate::host_state::HostMutableBuffer,
            "torvyn:streaming/types.flow-context":
                crate::host_state::HostFlowContext,
        },
    });
}

pub(crate) mod data_source {
    wasmtime::component::bindgen!({
        path: "../torvyn-contracts/wit/torvyn-streaming",
        world: "data-source",
        imports: { default: async | trappable },
        exports: { default: async },
        with: {
            "torvyn:streaming/types": super::data_sink::torvyn::streaming::types,
        },
    });
}

pub(crate) mod transform {
    wasmtime::component::bindgen!({
        path: "../torvyn-contracts/wit/torvyn-streaming",
        world: "transform",
        imports: { default: async | trappable },
        exports: { default: async },
        with: {
            "torvyn:streaming/types": super::data_sink::torvyn::streaming::types,
            "torvyn:streaming/buffer-allocator":
                super::data_source::torvyn::streaming::buffer_allocator,
        },
    });
}

pub(crate) mod managed_transform {
    wasmtime::component::bindgen!({
        path: "../torvyn-contracts/wit/torvyn-streaming",
        world: "managed-transform",
        imports: { default: async | trappable },
        exports: { default: async },
        with: {
            "torvyn:streaming/types": super::data_sink::torvyn::streaming::types,
            "torvyn:streaming/buffer-allocator":
                super::data_source::torvyn::streaming::buffer_allocator,
        },
    });
}
