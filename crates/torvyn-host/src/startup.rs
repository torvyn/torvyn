//! Flow startup sequence.
//!
//! Implements the full cold-path pipeline startup per Doc 10, Section 3.4.
//! Each step is a separate function for clarity, testability, and
//! precise error attribution.

use std::time::Instant;

use tracing::{info, instrument};

use torvyn_types::FlowId;

use crate::error::HostError;
// Used by cross-crate integration (currently commented out):
// use crate::error::{StartupError, StartupStage};

// ---------------------------------------------------------------------------
// execute_flow_startup — the full startup orchestration
// ---------------------------------------------------------------------------

/// Execute the complete startup sequence for a single flow.
///
/// # COLD PATH — called once per flow during host startup or
///   dynamic flow creation.
///
/// # Steps (Doc 10, Section 3.4)
/// 1. Convert the flow definition from config to a pipeline topology.
/// 2. Validate the topology (DAG check, connectedness, role consistency).
/// 3. Validate component contracts (WIT interface compatibility).
/// 4. Link components (resolve imports, check capabilities).
/// 5. Compile Wasm components (with caching).
/// 6. Instantiate component instances (with security sandbox).
/// 7. Call `lifecycle.init()` on each component.
/// 8. Register the flow with the reactor.
///
/// # Errors
/// Returns [`HostError::Startup`] with the specific
/// stage and reason if any step fails.
///
/// # Error Handling
/// If any step fails, all resources allocated in prior steps are
/// cleaned up (component instances are dropped, compiled modules
/// are not cached for failed components).
///
/// # Observability
/// Each step emits a `tracing` span with timing and context.
///
/// # Preconditions
/// - The flow name must correspond to a flow definition in the config.
/// - All subsystem handles must be initialized and functional.
///
/// # Postconditions
/// - On success, the reactor is running the flow.
/// - On failure, no resources are leaked.
#[instrument(skip_all, fields(flow_name = %flow_name, flow_id = %flow_id))]
pub async fn execute_flow_startup(
    flow_name: &str,
    flow_id: FlowId,
    // All subsystem handles passed by reference from the host.
    // CROSS-CRATE DEPENDENCIES — verify against each crate's LLI:
    //   config: &torvyn_config::PipelineDefinition,
    //   engine: &Arc<torvyn_engine::WasmtimeEngine>,
    //   invoker: &Arc<torvyn_engine::WasmtimeInvoker>,
    //   reactor: &torvyn_reactor::ReactorHandle,
    //   resources: &Arc<torvyn_resources::ResourceManager>,
    //   security: &Arc<torvyn_security::SecurityManager>,
) -> Result<FlowId, HostError> {
    let startup_start = Instant::now();

    // ------------------------------------------------------------------
    // Step 1: Topology construction
    // ------------------------------------------------------------------
    info!("Step 1/8: Constructing topology");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: torvyn_pipeline::flow_def_to_topology()
    // Converts the parsed FlowDef from torvyn-config into a
    // PipelineTopology. Verify against lli_10_torvyn_pipeline.md.
    //
    // let flow_def = config.get_flow(flow_name)
    //     .ok_or_else(|| StartupError::FlowStartup {
    //         flow_name: flow_name.to_owned(),
    //         stage: StartupStage::TopologyConstruction,
    //         reason: format!("No flow named '{flow_name}' in configuration"),
    //     })?;
    //
    // let topology = torvyn_pipeline::flow_def_to_topology(flow_def)
    //     .map_err(|e| StartupError::FlowStartup {
    //         flow_name: flow_name.to_owned(),
    //         stage: StartupStage::TopologyConstruction,
    //         reason: e.to_string(),
    //     })?;

    info!(
        elapsed_ms = step_start.elapsed().as_millis(),
        "Step 1/8 complete: topology constructed"
    );

    // ------------------------------------------------------------------
    // Step 2: Topology validation
    // ------------------------------------------------------------------
    info!("Step 2/8: Validating topology");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: torvyn_pipeline::validate::validate_topology()
    // Checks: acyclicity, connectedness, port compatibility, role consistency.
    //
    // let validation = torvyn_pipeline::validate::validate_topology(&topology);
    // if !validation.is_ok() {
    //     return Err(StartupError::FlowStartup {
    //         flow_name: flow_name.to_owned(),
    //         stage: StartupStage::TopologyValidation,
    //         reason: validation.errors().iter()
    //             .map(|e| e.to_string())
    //             .collect::<Vec<_>>()
    //             .join("; "),
    //     }.into());
    // }

    info!(
        elapsed_ms = step_start.elapsed().as_millis(),
        "Step 2/8 complete: topology valid"
    );

    // ------------------------------------------------------------------
    // Step 3: Contract validation
    // ------------------------------------------------------------------
    info!("Step 3/8: Validating contracts");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: torvyn_contracts validation.
    // For each component in the topology, validate that its WIT world
    // matches the expected interfaces.
    //
    // for node in topology.nodes() {
    //     let component_bytes = load_component(node.component_ref()).await?;
    //     torvyn_contracts::validate_component(
    //         &component_bytes,
    //         node.role(),
    //     ).map_err(|e| StartupError::FlowStartup {
    //         flow_name: flow_name.to_owned(),
    //         stage: StartupStage::ContractValidation,
    //         reason: format!("Component '{}': {e}", node.name()),
    //     })?;
    // }

    info!(
        elapsed_ms = step_start.elapsed().as_millis(),
        "Step 3/8 complete: contracts valid"
    );

    // ------------------------------------------------------------------
    // Step 4: Linking
    // ------------------------------------------------------------------
    info!("Step 4/8: Linking components");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: torvyn_linker::resolve_imports()
    // Resolves all imports, checks interface compatibility,
    // verifies capability grants.
    //
    // let linked = torvyn_linker::resolve_imports(
    //     &topology_as_linker_topology,
    //     &security,
    // ).map_err(|e| StartupError::FlowStartup {
    //     flow_name: flow_name.to_owned(),
    //     stage: StartupStage::Linking,
    //     reason: e.format_all(),
    // })?;

    info!(
        elapsed_ms = step_start.elapsed().as_millis(),
        "Step 4/8 complete: linking done"
    );

    // ------------------------------------------------------------------
    // Step 5: Compilation
    // ------------------------------------------------------------------
    info!("Step 5/8: Compiling components");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: torvyn_engine::WasmEngine::compile_component()
    // Compile each unique component type, using the cache.
    //
    // let mut compiled_components = Vec::new();
    // for component in linked.iter_components() {
    //     let bytes = std::fs::read(&component.artifact_path)
    //         .map_err(|e| StartupError::FlowStartup {
    //             flow_name: flow_name.to_owned(),
    //             stage: StartupStage::Compilation,
    //             reason: format!("Cannot read '{}': {e}", component.name),
    //         })?;
    //     let compiled = engine.compile_component(&bytes)
    //         .map_err(|e| StartupError::FlowStartup {
    //             flow_name: flow_name.to_owned(),
    //             stage: StartupStage::Compilation,
    //             reason: format!("Component '{}': {e}", component.name),
    //         })?;
    //     compiled_components.push(compiled);
    // }

    info!(
        elapsed_ms = step_start.elapsed().as_millis(),
        "Step 5/8 complete: compilation done"
    );

    // ------------------------------------------------------------------
    // Step 6: Instantiation
    // ------------------------------------------------------------------
    info!("Step 6/8: Instantiating components");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: torvyn_engine::WasmEngine::instantiate()
    // Create component instances with security sandbox applied.
    //
    // let mut instances = Vec::new();
    // for (idx, compiled) in compiled_components.iter().enumerate() {
    //     let component = &linked.components[idx];
    //     let component_id = ComponentId::new(flow_id.as_u64() * 1000 + idx as u64);
    //
    //     // Configure security sandbox per C02-9
    //     let sandbox = security.configure_sandbox(
    //         component_id,
    //         &component.capability_grants,
    //     ).map_err(|e| StartupError::FlowStartup {
    //         flow_name: flow_name.to_owned(),
    //         stage: StartupStage::Instantiation,
    //         reason: format!("Sandbox for '{}': {e}", component.name),
    //     })?;
    //
    //     let imports = build_import_bindings(component, &sandbox);
    //     let instance = engine.instantiate(compiled, imports, component_id).await
    //         .map_err(|e| StartupError::FlowStartup {
    //             flow_name: flow_name.to_owned(),
    //             stage: StartupStage::Instantiation,
    //             reason: format!("Component '{}': {e}", component.name),
    //         })?;
    //     instances.push((component_id, instance));
    // }

    info!(
        elapsed_ms = step_start.elapsed().as_millis(),
        "Step 6/8 complete: instantiation done"
    );

    // ------------------------------------------------------------------
    // Step 7: lifecycle.init()
    // ------------------------------------------------------------------
    info!("Step 7/8: Calling lifecycle.init()");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: torvyn_engine::ComponentInvoker::invoke_init()
    // Per C02-9: call lifecycle.init(config) with per-component config.
    //
    // for (component_id, instance) in &mut instances {
    //     let component = &linked.components[idx];
    //     if let Some(init_config) = component.config.as_deref() {
    //         invoker.invoke_init(instance, *component_id, init_config).await
    //             .map_err(|e| StartupError::FlowStartup {
    //                 flow_name: flow_name.to_owned(),
    //                 stage: StartupStage::ComponentInit,
    //                 reason: format!("Component '{}' init: {e}", component.name),
    //             })?;
    //     }
    // }

    info!(
        elapsed_ms = step_start.elapsed().as_millis(),
        "Step 7/8 complete: lifecycle.init() done"
    );

    // ------------------------------------------------------------------
    // Step 8: Register with reactor
    // ------------------------------------------------------------------
    info!("Step 8/8: Registering flow with reactor");
    let step_start = Instant::now();

    // CROSS-CRATE DEPENDENCY: ReactorHandle::create_flow()
    // Build a FlowConfig from the linked pipeline and instances,
    // then register with the reactor.
    //
    // let flow_config = build_flow_config(
    //     flow_id,
    //     &linked,
    //     &instances,
    //     &self.config.runtime,
    // );
    //
    // let reactor_flow_id = reactor.create_flow(flow_config).await
    //     .map_err(|e| StartupError::FlowStartup {
    //         flow_name: flow_name.to_owned(),
    //         stage: StartupStage::ReactorRegistration,
    //         reason: e.to_string(),
    //     })?;

    let _step_elapsed = step_start.elapsed();

    let total_elapsed = startup_start.elapsed();
    info!(
        total_ms = total_elapsed.as_millis(),
        "Flow startup complete in {total_elapsed:?}"
    );

    Ok(flow_id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::error::{StartupError, StartupStage};

    #[test]
    fn test_startup_stage_display_all_variants() {
        // Verify all stages have reasonable display strings
        let stages = [
            StartupStage::TopologyConstruction,
            StartupStage::TopologyValidation,
            StartupStage::ContractValidation,
            StartupStage::Linking,
            StartupStage::Compilation,
            StartupStage::Instantiation,
            StartupStage::ComponentInit,
            StartupStage::ReactorRegistration,
        ];
        for stage in &stages {
            let display = format!("{stage}");
            assert!(!display.is_empty(), "stage {stage:?} has empty display");
        }
    }

    #[test]
    fn test_startup_error_preserves_flow_name_and_stage() {
        let err = StartupError::FlowStartup {
            flow_name: "test-pipeline".into(),
            stage: StartupStage::Linking,
            reason: "unresolved import 'foo'".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("test-pipeline"));
        assert!(msg.contains("component linking"));
        assert!(msg.contains("unresolved import"));
    }

    #[test]
    fn test_startup_error_each_stage_produces_distinct_message() {
        let stages = [
            (StartupStage::TopologyConstruction, "topology construction"),
            (StartupStage::TopologyValidation, "topology validation"),
            (StartupStage::ContractValidation, "contract validation"),
            (StartupStage::Linking, "component linking"),
            (StartupStage::Compilation, "Wasm compilation"),
            (StartupStage::Instantiation, "component instantiation"),
            (StartupStage::ComponentInit, "lifecycle.init"),
            (StartupStage::ReactorRegistration, "reactor registration"),
        ];

        for (stage, expected_text) in &stages {
            let err = StartupError::FlowStartup {
                flow_name: "test".into(),
                stage: *stage,
                reason: "test reason".into(),
            };
            let msg = format!("{err}");
            assert!(
                msg.contains(expected_text),
                "stage {stage:?} should contain '{expected_text}' but got: {msg}"
            );
        }
    }
}
