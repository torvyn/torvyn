//! Pipeline definition schema.
//!
//! This is the second of two configuration contexts in Torvyn's model:
//!
//! 1. **Component Manifest** (`manifest` module) — project metadata.
//! 2. **Pipeline Definition** (this module) — flow topology and runtime
//!    overrides.
//!
//! A pipeline definition may appear:
//! - Inline in a project-level `Torvyn.toml` as `[flow.*]` tables.
//! - As a standalone `pipeline.toml` file.
//!
//! Per Doc 02 Section 5.2 and Doc 10, C01-1: uses `[flow.*.nodes.*]`
//! format with `[[flow.*.edges]]` arrays.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{ConfigErrors, ConfigParseError};
use crate::runtime::{BackpressureConfig, ObservabilityConfig, RuntimeConfig, SecurityConfig};

// ---------------------------------------------------------------------------
// EdgeEndpoint
// ---------------------------------------------------------------------------

/// One end of a stream edge (source or destination).
///
/// # Examples
/// ```
/// use torvyn_config::EdgeEndpoint;
///
/// let ep = EdgeEndpoint {
///     node: "source-1".into(),
///     port: "output".into(),
/// };
/// assert_eq!(ep.node, "source-1");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeEndpoint {
    /// Node name within the flow.
    pub node: String,

    /// Port name on the node.
    pub port: String,
}

// ---------------------------------------------------------------------------
// EdgeDef
// ---------------------------------------------------------------------------

/// A stream edge connecting two nodes in a flow.
///
/// # Invariants
/// - `from.node` and `to.node` must reference nodes defined in the same flow.
/// - `from.node` must differ from `to.node` (no self-loops).
///
/// # Examples
/// ```
/// use torvyn_config::{EdgeDef, EdgeEndpoint};
///
/// let edge = EdgeDef {
///     from: EdgeEndpoint { node: "a".into(), port: "output".into() },
///     to: EdgeEndpoint { node: "b".into(), port: "input".into() },
///     queue_depth: None,
///     backpressure: None,
/// };
/// assert_eq!(edge.from.node, "a");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeDef {
    /// Upstream endpoint (producer).
    pub from: EdgeEndpoint,

    /// Downstream endpoint (consumer).
    pub to: EdgeEndpoint,

    /// Per-edge queue depth override.
    /// Default: uses the flow-level or global default (64 per C02-2).
    #[serde(default)]
    pub queue_depth: Option<usize>,

    /// Per-edge backpressure policy override.
    /// Default: uses the flow-level or global default.
    #[serde(default)]
    pub backpressure: Option<BackpressureConfig>,
}

// ---------------------------------------------------------------------------
// NodeDef
// ---------------------------------------------------------------------------

/// A component instance within a flow.
///
/// # Invariants
/// - `component` is a valid component reference (file path, OCI reference,
///   or local component name).
/// - `interface` is a valid WIT interface path.
///
/// # Examples
/// ```
/// use torvyn_config::NodeDef;
///
/// let node = NodeDef {
///     component: "file://./my-source.wasm".into(),
///     interface: "torvyn:streaming/source".into(),
///     ..Default::default()
/// };
/// assert_eq!(node.interface, "torvyn:streaming/source");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDef {
    /// Component reference. Can be:
    /// - `"file://./path/to/component.wasm"` — local file.
    /// - `"oci://registry.example.com/name:tag"` — OCI artifact.
    /// - A bare name referencing a `[[component]]` declaration in the manifest.
    #[serde(default)]
    pub component: String,

    /// WIT interface this node implements.
    /// E.g., `"torvyn:streaming/source"`, `"torvyn:streaming/processor"`.
    #[serde(default)]
    pub interface: String,

    /// Per-component configuration override (JSON string).
    /// Passed to `lifecycle.init()`. Overrides the component-level `config`.
    #[serde(default)]
    pub config: Option<String>,

    /// Per-component fuel budget override.
    /// Default: uses the global `default_fuel_per_invocation`.
    #[serde(default)]
    pub fuel_budget: Option<u64>,

    /// Per-component memory budget override (in bytes).
    /// Default: uses the global `max_memory_per_component`.
    #[serde(default)]
    pub max_memory: Option<String>,

    /// Per-component priority for scheduling.
    /// Default: uses the global `default_priority`.
    #[serde(default)]
    pub priority: Option<u32>,
}

// ---------------------------------------------------------------------------
// FlowDef
// ---------------------------------------------------------------------------

/// A complete flow (pipeline) definition.
///
/// A flow is a DAG of component nodes connected by typed stream edges.
///
/// # Invariants
/// - At least two nodes (a source and a sink).
/// - At least one edge connecting them.
/// - The graph is a DAG (no cycles).
/// - Every edge references nodes that exist in this flow.
///
/// # Examples
/// ```
/// use torvyn_config::FlowDef;
///
/// let toml_str = r#"
/// description = "test flow"
///
/// [nodes.source]
/// component = "file://./source.wasm"
/// interface = "torvyn:streaming/source"
///
/// [nodes.sink]
/// component = "file://./sink.wasm"
/// interface = "torvyn:streaming/sink"
///
/// [[edges]]
/// from = { node = "source", port = "output" }
/// to = { node = "sink", port = "input" }
/// "#;
///
/// let flow: FlowDef = toml::from_str(toml_str).unwrap();
/// assert_eq!(flow.nodes.len(), 2);
/// assert_eq!(flow.edges.len(), 1);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowDef {
    /// Human-readable description of this flow.
    #[serde(default)]
    pub description: String,

    /// Component nodes in this flow, keyed by node name.
    #[serde(default)]
    pub nodes: BTreeMap<String, NodeDef>,

    /// Stream edges connecting nodes.
    #[serde(default)]
    pub edges: Vec<EdgeDef>,

    /// Per-flow scheduling policy override.
    #[serde(default)]
    pub scheduling_policy: Option<String>,

    /// Per-flow default queue depth override.
    #[serde(default)]
    pub default_queue_depth: Option<usize>,

    /// Per-flow backpressure config override.
    #[serde(default)]
    pub backpressure: Option<BackpressureConfig>,
}

// ---------------------------------------------------------------------------
// PipelineDefinition
// ---------------------------------------------------------------------------

/// Top-level pipeline definition, containing one or more flows.
///
/// Can be loaded from:
/// - Inline `[flow.*]` tables in a `Torvyn.toml`.
/// - A standalone `pipeline.toml` file.
///
/// # Examples
/// ```
/// use torvyn_config::PipelineDefinition;
///
/// let toml_str = r#"
/// [flow.main]
/// description = "Main pipeline"
///
/// [flow.main.nodes.source]
/// component = "file://./source.wasm"
/// interface = "torvyn:streaming/source"
///
/// [flow.main.nodes.sink]
/// component = "file://./sink.wasm"
/// interface = "torvyn:streaming/sink"
///
/// [[flow.main.edges]]
/// from = { node = "source", port = "output" }
/// to = { node = "sink", port = "input" }
/// "#;
///
/// let pipeline = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml").unwrap();
/// assert!(pipeline.flows.contains_key("main"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PipelineDefinition {
    /// Named flows in this pipeline definition.
    #[serde(default, rename = "flow")]
    pub flows: BTreeMap<String, FlowDef>,

    /// Runtime configuration overrides for this pipeline.
    #[serde(default)]
    pub runtime: Option<RuntimeConfig>,

    /// Observability overrides for this pipeline.
    #[serde(default)]
    pub observability: Option<ObservabilityConfig>,

    /// Security overrides for this pipeline.
    #[serde(default)]
    pub security: Option<SecurityConfig>,
}

impl PipelineDefinition {
    /// Parse a `PipelineDefinition` from a TOML string.
    ///
    /// # COLD PATH — called during pipeline loading.
    ///
    /// # Errors
    /// Returns `Err(Vec<ConfigParseError>)` if parsing or validation fails.
    pub fn from_toml_str(
        toml_str: &str,
        file_path: &str,
    ) -> Result<Self, Vec<ConfigParseError>> {
        let pipeline: Self = toml::from_str(toml_str).map_err(|e| {
            vec![ConfigParseError::toml_syntax(file_path, &e)]
        })?;

        let mut errors = ConfigErrors::new();
        pipeline.validate(file_path, &mut errors);

        errors.into_result()?;
        Ok(pipeline)
    }

    /// Extract a `PipelineDefinition` from inline `[flow.*]` tables
    /// in a `ComponentManifest`.
    ///
    /// # COLD PATH — called when flows are defined inline in `Torvyn.toml`.
    ///
    /// # Errors
    /// Returns `Err(Vec<ConfigParseError>)` if any flow definition is invalid.
    pub fn from_manifest_flows(
        flow_map: &BTreeMap<String, toml::Value>,
        file_path: &str,
    ) -> Result<BTreeMap<String, FlowDef>, Vec<ConfigParseError>> {
        let mut flows = BTreeMap::new();
        let mut all_errors = ConfigErrors::new();

        for (name, value) in flow_map {
            match value.clone().try_into::<FlowDef>() {
                Ok(flow_def) => {
                    validate_flow(file_path, name, &flow_def, &mut all_errors);
                    flows.insert(name.clone(), flow_def);
                }
                Err(e) => {
                    all_errors.push(ConfigParseError::invalid_value(
                        file_path,
                        &format!("flow.{name}"),
                        &format!("{e}"),
                        "a valid flow definition",
                        "Check the flow table structure: [flow.<name>.nodes.*] and [[flow.<name>.edges]].",
                    ));
                }
            }
        }

        all_errors.into_result()?;
        Ok(flows)
    }

    /// Validate the entire pipeline definition.
    ///
    /// # COLD PATH — called during pipeline loading.
    fn validate(&self, file: &str, errors: &mut ConfigErrors) {
        if self.flows.is_empty() {
            errors.push(ConfigParseError::missing_field(
                file,
                "flow",
                "pipeline definition",
            ));
            return;
        }

        for (name, flow) in &self.flows {
            validate_flow(file, name, flow, errors);
        }
    }
}

/// Validate a single flow definition.
///
/// # COLD PATH — called during pipeline validation.
fn validate_flow(file: &str, name: &str, flow: &FlowDef, errors: &mut ConfigErrors) {
    let prefix = format!("flow.{name}");

    // Must have at least two nodes
    if flow.nodes.len() < 2 {
        errors.push(ConfigParseError::semantic(
            file,
            &prefix,
            &format!(
                "Flow '{name}' has {} node(s), but at least 2 are required (a source and a sink)",
                flow.nodes.len()
            ),
            "Add at least a source and a sink node.",
        ));
    }

    // Must have at least one edge
    if flow.edges.is_empty() {
        errors.push(ConfigParseError::semantic(
            file,
            &format!("{prefix}.edges"),
            &format!("Flow '{name}' has no edges"),
            "Add at least one edge connecting a source to a sink.",
        ));
    }

    // Validate node definitions
    for (node_name, node_def) in &flow.nodes {
        if node_def.component.is_empty() {
            errors.push(ConfigParseError::missing_field(
                file,
                &format!("{prefix}.nodes.{node_name}.component"),
                &format!("{prefix}.nodes.{node_name}"),
            ));
        }
        if node_def.interface.is_empty() {
            errors.push(ConfigParseError::missing_field(
                file,
                &format!("{prefix}.nodes.{node_name}.interface"),
                &format!("{prefix}.nodes.{node_name}"),
            ));
        }
        if let Some(priority) = node_def.priority {
            if !(1..=10).contains(&priority) {
                errors.push(ConfigParseError::invalid_value(
                    file,
                    &format!("{prefix}.nodes.{node_name}.priority"),
                    &priority.to_string(),
                    "an integer between 1 and 10",
                    "Set priority to a value from 1 (lowest) to 10 (highest).",
                ));
            }
        }
    }

    // Validate edge endpoints reference existing nodes
    for (i, edge) in flow.edges.iter().enumerate() {
        if !flow.nodes.contains_key(&edge.from.node) {
            errors.push(ConfigParseError::semantic(
                file,
                &format!("{prefix}.edges[{i}].from.node"),
                &format!("Edge references non-existent node '{}'", edge.from.node),
                &format!(
                    "Ensure '{}' is defined in [{prefix}.nodes].",
                    edge.from.node
                ),
            ));
        }
        if !flow.nodes.contains_key(&edge.to.node) {
            errors.push(ConfigParseError::semantic(
                file,
                &format!("{prefix}.edges[{i}].to.node"),
                &format!("Edge references non-existent node '{}'", edge.to.node),
                &format!(
                    "Ensure '{}' is defined in [{prefix}.nodes].",
                    edge.to.node
                ),
            ));
        }
        if edge.from.node == edge.to.node {
            errors.push(ConfigParseError::semantic(
                file,
                &format!("{prefix}.edges[{i}]"),
                &format!("Self-loop: edge from '{}' to itself", edge.from.node),
                "Remove the self-loop or use different nodes.",
            ));
        }
        if let Some(qd) = edge.queue_depth {
            if qd == 0 {
                errors.push(ConfigParseError::invalid_value(
                    file,
                    &format!("{prefix}.edges[{i}].queue_depth"),
                    "0",
                    "a positive integer",
                    "Queue depth must be at least 1.",
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_PIPELINE: &str = r#"
[flow.main]
description = "test"

[flow.main.nodes.source]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }
"#;

    #[test]
    fn test_pipeline_parse_minimal() {
        let pipeline =
            PipelineDefinition::from_toml_str(MINIMAL_PIPELINE, "pipeline.toml").unwrap();
        assert!(pipeline.flows.contains_key("main"));
        let flow = &pipeline.flows["main"];
        assert_eq!(flow.nodes.len(), 2);
        assert_eq!(flow.edges.len(), 1);
        assert_eq!(flow.edges[0].from.node, "source");
        assert_eq!(flow.edges[0].to.node, "sink");
    }

    #[test]
    fn test_pipeline_parse_three_stage() {
        let toml_str = r#"
[flow.main]
description = "Three-stage pipeline"

[flow.main.nodes.source-1]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.transform-1]
component = "file://./transform.wasm"
interface = "torvyn:streaming/processor"
priority = 8

[flow.main.nodes.sink-1]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source-1", port = "output" }
to = { node = "transform-1", port = "input" }
queue_depth = 128

[[flow.main.edges]]
from = { node = "transform-1", port = "output" }
to = { node = "sink-1", port = "input" }
"#;
        let pipeline =
            PipelineDefinition::from_toml_str(toml_str, "pipeline.toml").unwrap();
        let flow = &pipeline.flows["main"];
        assert_eq!(flow.nodes.len(), 3);
        assert_eq!(flow.edges.len(), 2);
        assert_eq!(flow.nodes["transform-1"].priority, Some(8));
        assert_eq!(flow.edges[0].queue_depth, Some(128));
    }

    #[test]
    fn test_pipeline_no_flows_returns_error() {
        let toml_str = "";
        let result = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_flow_with_one_node_returns_error() {
        let toml_str = r#"
[flow.main.nodes.source]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"
"#;
        let result = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_edge_references_nonexistent_node() {
        let toml_str = r#"
[flow.main.nodes.source]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "ghost", port = "input" }
"#;
        let result = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.message.contains("ghost")));
    }

    #[test]
    fn test_pipeline_self_loop_returns_error() {
        let toml_str = r#"
[flow.main.nodes.source]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "source", port = "input" }
"#;
        let result = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml");
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.message.contains("Self-loop")));
    }

    #[test]
    fn test_pipeline_invalid_priority_returns_error() {
        let toml_str = r#"
[flow.main.nodes.source]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"
priority = 99

[flow.main.nodes.sink]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }
"#;
        let result = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_zero_queue_depth_returns_error() {
        let toml_str = r#"
[flow.main.nodes.source]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }
queue_depth = 0
"#;
        let result = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_missing_component_field() {
        let toml_str = r#"
[flow.main.nodes.source]
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }
"#;
        let result = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_round_trip() {
        let original =
            PipelineDefinition::from_toml_str(MINIMAL_PIPELINE, "p.toml").unwrap();
        let serialized = toml::to_string_pretty(&original).unwrap();
        let reparsed = PipelineDefinition::from_toml_str(&serialized, "p.toml").unwrap();
        assert_eq!(original.flows.len(), reparsed.flows.len());
        assert_eq!(
            original.flows["main"].nodes.len(),
            reparsed.flows["main"].nodes.len()
        );
    }

    #[test]
    fn test_pipeline_multiple_flows() {
        let toml_str = r#"
[flow.ingest]
[flow.ingest.nodes.src]
component = "a.wasm"
interface = "torvyn:streaming/source"
[flow.ingest.nodes.sink]
component = "b.wasm"
interface = "torvyn:streaming/sink"
[[flow.ingest.edges]]
from = { node = "src", port = "output" }
to = { node = "sink", port = "input" }

[flow.process]
[flow.process.nodes.src]
component = "c.wasm"
interface = "torvyn:streaming/source"
[flow.process.nodes.sink]
component = "d.wasm"
interface = "torvyn:streaming/sink"
[[flow.process.edges]]
from = { node = "src", port = "output" }
to = { node = "sink", port = "input" }
"#;
        let pipeline = PipelineDefinition::from_toml_str(toml_str, "p.toml").unwrap();
        assert_eq!(pipeline.flows.len(), 2);
        assert!(pipeline.flows.contains_key("ingest"));
        assert!(pipeline.flows.contains_key("process"));
    }
}
