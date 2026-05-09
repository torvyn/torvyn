//! Pipeline instantiation: the cross-crate integration seam.
//!
//! [`instantiate_pipeline`] is the single function that takes a validated
//! `PipelineTopology` plus subsystem handles and produces a running flow:
//! it loads each component, compiles it via the engine, instantiates it,
//! calls `lifecycle.init`, then hands the assembled instances to the
//! reactor to drive.
//!
//! # Design Decision (per HLI Doc 10, Recommendation 2)
//! Component instances are owned by the reactor's flow driver while the
//! flow runs. The pipeline crate is responsible only for the cold-path
//! cross-crate orchestration: it does not retain instances after the
//! reactor takes ownership. Teardown during the running phase happens in
//! the flow driver; teardown during partial-instantiation failure happens
//! here, in reverse topological order, before propagating the error.

use std::sync::Arc;

use tracing::{info, instrument, warn};

use torvyn_engine::{ComponentInstance, ComponentInvoker, WasmEngine};
use torvyn_reactor::{
    config::{FlowConfig, StreamConfig},
    handle::ReactorHandle,
    topology::{FlowTopology, StageDefinition, StreamConnection},
};
use torvyn_types::ComponentId;

use crate::error::PipelineError;
use crate::handle::PipelineHandle;
use crate::topology::{EdgeConfig, PipelineTopology};

// ---------------------------------------------------------------------------
// instantiate_pipeline — the production cross-crate cold path
// ---------------------------------------------------------------------------

/// Instantiate a validated `PipelineTopology` and register the resulting
/// flow with the reactor.
///
/// # COLD PATH — called once per flow.
///
/// # Steps
/// 1. For each node in topological order:
///    a. Resolve `component_ref` to component bytes.
///    b. Compile via the supplied [`WasmEngine`].
///    c. Instantiate using the engine's `default_imports()`.
///    d. Invoke `lifecycle.init` with the node's configured init string.
/// 2. Translate the [`PipelineTopology`] into a reactor [`FlowTopology`].
/// 3. Build a default [`FlowConfig`] for the topology.
/// 4. Submit `(config, instances)` to the reactor via
///    [`ReactorHandle::spawn_flow`]; the reactor builds streams,
///    constructs a `FlowDriver`, and runs it on a Tokio task.
///
/// # Component reference resolution
/// Phase 0 supports two schemes:
/// - `file://<path>` — read the component bytes from disk.
/// - `mock://<name>` — produce empty bytes (suitable for engines such as
///   `torvyn_engine::mock::MockEngine` that ignore bytes during compile).
///
/// Other schemes return [`PipelineError::Subsystem`].
///
/// # Cleanup on failure
/// If any of compile, instantiate, or `lifecycle.init` fails, all
/// previously-initialized components are torn down via
/// [`ComponentInvoker::invoke_teardown`] in reverse topological order
/// before the error is returned. Instances drop after teardown, freeing
/// engine-side resources via RAII.
///
/// # Preconditions
/// - `topology` has been built via [`PipelineTopologyBuilder`](crate::PipelineTopologyBuilder)
///   and therefore satisfies all topology invariants.
///
/// # Postconditions
/// On success, the reactor is running the flow and the returned
/// [`PipelineHandle`] can be used with
/// [`shutdown_pipeline`](crate::shutdown::shutdown_pipeline) to stop it.
///
/// # Errors
/// Returns [`PipelineError`] with the specific stage and node on failure.
///
/// # Panics
/// Panics only if topology invariants are violated mid-flight — these
/// indicate corruption of an already-validated `PipelineTopology` and
/// should not happen in practice:
/// - if a `node_idx` produced by `execution_order()` does not resolve to
///   a node (`PipelineTopology::node` returns `None`), or
/// - if the topological walk completes without filling every slot
///   (which would mean some node was never reached, contradicting the
///   reachability invariant proved by `validate_topology`).
///
/// # Examples
/// See `tests/integration/tests/test_pipeline_instantiation.rs` for a
/// `MockEngine`-backed end-to-end usage example.
#[instrument(skip(topology, engine, invoker, reactor), fields(flow_name = %topology.name()))]
pub async fn instantiate_pipeline<E, I>(
    topology: &PipelineTopology,
    engine: &E,
    invoker: &Arc<I>,
    reactor: &ReactorHandle,
) -> Result<PipelineHandle, PipelineError>
where
    E: WasmEngine,
    I: ComponentInvoker,
{
    let flow_name = topology.name();
    let node_count = topology.node_count();
    let edge_count = topology.edge_count();

    info!(
        nodes = node_count,
        edges = edge_count,
        "Instantiating pipeline '{flow_name}'"
    );

    // Slot per node (by index in topology.nodes()) holding the live
    // instance. A slot is only filled after a successful
    // instantiate + lifecycle.init for that node, which makes
    // `cleanup_partial` correctly torn-down-only-what-was-init'd.
    let mut instances_by_idx: Vec<Option<ComponentInstance>> =
        (0..node_count).map(|_| None).collect();

    // Iterate in topological order so producers come before consumers.
    // (Validation already guarantees the order is a DAG-respecting one.)
    for &node_idx in topology.execution_order() {
        match instantiate_one_node(topology, node_idx, engine, invoker).await {
            Ok(instance) => {
                instances_by_idx[node_idx] = Some(instance);
            }
            Err(err) => {
                cleanup_partial(invoker, &mut instances_by_idx).await;
                return Err(err);
            }
        }
    }

    // Step 2: translate PipelineTopology -> FlowTopology. Stage indices
    // mirror topology.nodes() order so edge endpoints (already indices)
    // remain valid.
    let stages: Vec<StageDefinition> = topology
        .nodes()
        .iter()
        .enumerate()
        .map(|(idx, node)| StageDefinition {
            component_id: ComponentId::new((idx as u64).saturating_add(1)),
            role: node.role(),
            fuel_budget: node.config().fuel_budget,
            config: node.config().init_config.clone().unwrap_or_default(),
        })
        .collect();

    let connections: Vec<StreamConnection> = topology
        .edges()
        .iter()
        .map(|edge| StreamConnection {
            from_stage: edge.from_node(),
            to_stage: edge.to_node(),
            config: stream_config_from_edge(edge.edge_config()),
        })
        .collect();

    let flow_topology = FlowTopology {
        stages,
        connections,
    };
    let flow_config = FlowConfig::default_with_topology(flow_topology);

    // Step 3: collect instances in topology-node order. Every node in a
    // validated topology is reachable from a source, so the topological
    // walk above filled every slot.
    let instances: Vec<ComponentInstance> = instances_by_idx
        .into_iter()
        .enumerate()
        .map(|(idx, slot)| {
            slot.unwrap_or_else(|| {
                unreachable!(
                    "instances_by_idx[{idx}] is None after a successful \
                     instantiation walk over a validated topology"
                )
            })
        })
        .collect();

    // Step 4: hand off to the reactor. From this point the reactor owns
    // the instances (the flow driver consumes them) and is responsible
    // for invoking lifecycle.teardown during drain.
    let flow_id = reactor
        .spawn_flow(flow_config, instances)
        .await
        .map_err(|e| PipelineError::FlowRegistrationFailed {
            flow_name: flow_name.to_owned(),
            reason: e.to_string(),
        })?;

    info!(
        flow_id = %flow_id,
        "Pipeline '{flow_name}' instantiated and registered with reactor"
    );

    Ok(PipelineHandle::new(
        flow_id,
        flow_name.to_owned(),
        topology.clone(),
    ))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compile, instantiate, and `lifecycle.init` a single component.
/// Returns the live, initialized instance on success.
///
/// Per-flow component IDs are derived from the node's positional index
/// in the topology. The reactor enforces uniqueness within a flow but
/// places no ordering requirement on `ComponentId` values themselves.
///
/// On `lifecycle.init` failure the partially-initialized instance is
/// dropped here (its post-init state is undefined and teardown is not
/// safe to call); the caller is responsible for tearing down any
/// previously-initialized siblings via [`cleanup_partial`].
async fn instantiate_one_node<E, I>(
    topology: &PipelineTopology,
    node_idx: usize,
    engine: &E,
    invoker: &Arc<I>,
) -> Result<ComponentInstance, PipelineError>
where
    E: WasmEngine,
    I: ComponentInvoker,
{
    let flow_name = topology.name();
    let node = topology
        .node(node_idx)
        .expect("execution_order yields valid indices");
    let component_id = ComponentId::new((node_idx as u64).saturating_add(1));

    info!(
        node_name = node.name(),
        role = %node.role(),
        component_ref = node.component_ref(),
        "Instantiating component"
    );

    // Step 1: load bytes for this component.
    let bytes =
        load_component_bytes(node.component_ref()).map_err(|e| PipelineError::Subsystem {
            subsystem: "io",
            reason: format!(
                "Cannot load component '{node_name}' from '{component_ref}': {e}",
                node_name = node.name(),
                component_ref = node.component_ref(),
            ),
        })?;

    // Step 2: compile.
    let compiled =
        engine
            .compile_component(&bytes)
            .map_err(|e| PipelineError::CompilationFailed {
                flow_name: flow_name.to_owned(),
                node_name: node.name().to_owned(),
                reason: e.to_string(),
            })?;

    // Step 3: instantiate with default imports.
    let imports = engine.default_imports();
    let mut instance = engine
        .instantiate(&compiled, imports, component_id)
        .await
        .map_err(|e| PipelineError::InstantiationFailed {
            flow_name: flow_name.to_owned(),
            node_name: node.name().to_owned(),
            reason: e.to_string(),
        })?;

    // Step 4: lifecycle.init.
    let init_config = node.config().init_config.as_deref().unwrap_or("{}");
    if let Err(e) = invoker
        .invoke_init(&mut instance, component_id, init_config)
        .await
    {
        // Drop the instance whose init failed: its state is undefined,
        // so we must NOT call teardown on it. RAII frees engine resources.
        drop(instance);
        return Err(PipelineError::InitializationFailed {
            flow_name: flow_name.to_owned(),
            node_name: node.name().to_owned(),
            reason: e.to_string(),
        });
    }

    Ok(instance)
}

/// Resolve a component reference to its bytes.
///
/// Supported schemes (Phase 0):
/// - `file://<path>` — read from disk.
/// - `mock://<name>` — yield empty bytes (for engines that ignore them).
///
/// # Errors
/// Returns an [`std::io::Error`] for unreadable files or unsupported schemes.
fn load_component_bytes(component_ref: &str) -> std::io::Result<Vec<u8>> {
    if let Some(path) = component_ref.strip_prefix("file://") {
        std::fs::read(path)
    } else if component_ref.starts_with("mock://") {
        // Mock engines (notably `torvyn_engine::mock::MockEngine`) ignore
        // bytes during compile_component. Empty bytes keep the seam
        // compatible without requiring callers to fabricate placeholder
        // Wasm payloads.
        Ok(Vec::new())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!(
                "Unsupported component reference scheme in '{component_ref}'. \
                 Phase 0 supports 'file://<path>' and 'mock://<name>'."
            ),
        ))
    }
}

/// Convert a pipeline-level [`EdgeConfig`] into a reactor-level
/// [`StreamConfig`]. Pipeline edges do not (yet) expose the
/// low-watermark ratio; the reactor falls back to its flow default.
fn stream_config_from_edge(ec: &EdgeConfig) -> StreamConfig {
    StreamConfig {
        capacity: ec.queue_depth,
        backpressure_policy: ec.backpressure_policy,
        low_watermark_ratio: None,
    }
}

/// Tear down all already-initialized components, in reverse topological
/// order. Slots that are `None` were never successfully initialized and
/// are skipped. Called only on failure paths; the success path hands
/// instances to the reactor.
async fn cleanup_partial<I: ComponentInvoker>(
    invoker: &Arc<I>,
    instances_by_idx: &mut [Option<ComponentInstance>],
) {
    for (idx, slot) in instances_by_idx.iter_mut().enumerate().rev() {
        if let Some(instance) = slot.as_mut() {
            let component_id = instance.component_id();
            warn!(
                component_id = %component_id,
                node_idx = idx,
                "Tearing down component during partial-instantiation cleanup"
            );
            invoker.invoke_teardown(instance, component_id).await;
        }
    }
    // Slots are dropped along with the borrowed slice's owner; engine
    // resources (Wasmtime stores, etc.) are reclaimed via RAII.
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::PipelineTopologyBuilder;
    use crate::topology::NodeConfig;
    use torvyn_types::ComponentRole;

    #[test]
    fn test_load_component_bytes_mock_returns_empty() {
        let bytes = load_component_bytes("mock://hello").unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn test_load_component_bytes_unknown_scheme() {
        let err = load_component_bytes("oci://example.com/img").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        let msg = err.to_string();
        assert!(msg.contains("Phase 0"));
    }

    #[test]
    fn test_load_component_bytes_file_missing() {
        let err = load_component_bytes("file:///nonexistent/torvyn-test-stub.wasm").unwrap_err();
        // Either NotFound or PermissionDenied depending on platform.
        assert!(matches!(
            err.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
        ));
    }

    #[test]
    fn test_stream_config_from_edge_passthrough() {
        let ec = EdgeConfig {
            queue_depth: Some(128),
            backpressure_policy: Some(torvyn_types::BackpressurePolicy::DropOldest),
        };
        let sc = stream_config_from_edge(&ec);
        assert_eq!(sc.capacity, Some(128));
        assert_eq!(
            sc.backpressure_policy,
            Some(torvyn_types::BackpressurePolicy::DropOldest)
        );
        assert!(sc.low_watermark_ratio.is_none());
    }

    #[test]
    fn test_stream_config_from_edge_default_passes_none() {
        let ec = EdgeConfig::default();
        let sc = stream_config_from_edge(&ec);
        assert!(sc.capacity.is_none());
        assert!(sc.backpressure_policy.is_none());
        assert!(sc.low_watermark_ratio.is_none());
    }

    /// Sanity: the function's signature compiles against the trait
    /// bounds we expect. The actual end-to-end run lives in the
    /// workspace integration tests, where a `MockEngine`-backed
    /// reactor is spawned alongside.
    #[test]
    fn test_instantiate_signature_compiles() {
        fn _accepts<E: WasmEngine, I: ComponentInvoker>(
            _: &PipelineTopology,
            _: &E,
            _: &Arc<I>,
            _: &ReactorHandle,
        ) {
        }
    }

    #[test]
    fn test_pipeline_topology_builds_for_two_node_pipeline() {
        // Smoke check: the topology used by the integration test is
        // valid in isolation. Avoids confusing the integration test if
        // the topology itself has a bug.
        let topo = PipelineTopologyBuilder::new("test-flow")
            .add_node(
                "src",
                ComponentRole::Source,
                "mock://source",
                NodeConfig::default(),
            )
            .add_node(
                "snk",
                ComponentRole::Sink,
                "mock://sink",
                NodeConfig::default(),
            )
            .add_edge("src", "output", "snk", "input")
            .build()
            .unwrap();

        assert_eq!(topo.node_count(), 2);
        assert_eq!(topo.edge_count(), 1);
    }
}
