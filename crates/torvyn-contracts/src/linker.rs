//! Multi-component composition validation (link-time checks).
//!
//! This module implements the validation that `torvyn link` performs:
//! interface compatibility, capability satisfaction, topology correctness,
//! and version range resolution.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::compatibility::{check_compatibility, CompatibilityVerdict};
use crate::errors::{DiagnosticBuilder, ErrorCode, ValidationResult};
use crate::parser::ParsedPackage;
use crate::validator::ComponentManifest;

// COLD PATH — link validation runs during `torvyn link`.

/// A component entry in the pipeline topology.
///
/// Invariants:
/// - `name` is non-empty and unique within the pipeline.
/// - `role` matches the component's exported interface.
#[derive(Debug, Clone)]
pub struct PipelineComponent {
    /// Unique name of this component in the pipeline.
    pub name: String,
    /// Component's role (inferred from exported interface).
    pub role: ComponentRole,
    /// Parsed WIT packages for this component.
    pub packages: Vec<ParsedPackage>,
    /// Component manifest.
    pub manifest: ComponentManifest,
    /// Path to the component's artifact.
    pub artifact_path: PathBuf,
    /// Configuration string to pass to lifecycle.init().
    pub config: Option<String>,
}

/// Role of a component in a pipeline.
///
/// Matches `ComponentRole` from `torvyn-types` (per I-10, C02-7).
///
/// CROSS-CRATE DEPENDENCY: requires `ComponentRole` from `torvyn-types`.
/// Verify against lli_01_torvyn_types.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentRole {
    /// A data source (producer).
    Source,
    /// A stream processor (transformer).
    Processor,
    /// A data sink (consumer).
    Sink,
    /// A filter (accept/reject).
    Filter,
    /// A router (multi-port dispatch).
    Router,
}

/// A connection between two components in the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConnection {
    /// Name of the upstream component.
    pub from: String,
    /// Name of the downstream component.
    pub to: String,
    /// Queue depth for this connection.
    pub queue_depth: u32,
    /// Port name (for routers).
    pub port: Option<String>,
}

/// Capability grant for a component in the pipeline.
#[derive(Debug, Clone)]
pub struct CapabilityGrant {
    /// Capability name.
    pub name: String,
    /// Grant details (e.g., allowed paths for filesystem access).
    pub details: String,
}

/// A complete pipeline definition for link validation.
///
/// Invariants:
/// - `components` is non-empty.
/// - `connections` references only components in `components`.
/// - Each component name is unique.
#[derive(Debug, Clone)]
pub struct PipelineDefinition {
    /// Pipeline name.
    pub name: String,
    /// Components in the pipeline.
    pub components: Vec<PipelineComponent>,
    /// Connections between components.
    pub connections: Vec<PipelineConnection>,
    /// Per-component capability grants.
    pub capability_grants: HashMap<String, Vec<CapabilityGrant>>,
}

/// Validate a complete pipeline definition.
///
/// This performs all composition-time validations from Doc 01 Section 7.4:
/// - Topology validation (DAG, connectivity)
/// - Interface compatibility
/// - Capability satisfaction
/// - Version range satisfaction
///
/// Reports ALL errors found, not just the first one.
///
/// # Preconditions
/// - All components in the pipeline have valid, parsed WIT packages.
/// - All manifests are valid.
///
/// # Postconditions
/// - Returns a `ValidationResult` with all diagnostics.
/// - `is_ok()` is true only if the entire pipeline is valid.
///
/// # COLD PATH
pub fn validate_pipeline(pipeline: &PipelineDefinition) -> ValidationResult {
    let mut result = ValidationResult::new();

    // Build lookup map
    let component_map: HashMap<&str, &PipelineComponent> = pipeline
        .components
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();

    // 1. Topology validation
    validate_topology(pipeline, &component_map, &mut result);

    // 2. Interface compatibility (version checks between connected components)
    validate_interface_compatibility(pipeline, &component_map, &mut result);

    // 3. Capability satisfaction
    validate_capabilities(pipeline, &mut result);

    // 4. Version range satisfaction
    validate_version_ranges(pipeline, &mut result);

    result.sort();
    result
}

/// Validate pipeline topology.
fn validate_topology(
    pipeline: &PipelineDefinition,
    component_map: &HashMap<&str, &PipelineComponent>,
    result: &mut ValidationResult,
) {
    // Check that all connection endpoints reference known components
    for conn in &pipeline.connections {
        if !component_map.contains_key(conn.from.as_str()) {
            result.push(
                DiagnosticBuilder::error(
                    ErrorCode::InvalidTopology,
                    format!("connection references unknown component '{}'", conn.from),
                )
                .help("check that all connection 'from' values match a defined component name")
                .build(),
            );
        }
        if !component_map.contains_key(conn.to.as_str()) {
            result.push(
                DiagnosticBuilder::error(
                    ErrorCode::InvalidTopology,
                    format!("connection references unknown component '{}'", conn.to),
                )
                .help("check that all connection 'to' values match a defined component name")
                .build(),
            );
        }
    }

    // Check that every component has at least one connection
    let connected: HashSet<&str> = pipeline
        .connections
        .iter()
        .flat_map(|c| [c.from.as_str(), c.to.as_str()])
        .collect();

    for comp in &pipeline.components {
        if !connected.contains(comp.name.as_str()) {
            result.push(
                DiagnosticBuilder::error(
                    ErrorCode::DisconnectedComponent,
                    format!("component '{}' has no connections", comp.name),
                )
                .help("connect this component to the pipeline or remove it")
                .build(),
            );
        }
    }

    // Check source/sink constraints
    let incoming: HashMap<&str, usize> = {
        let mut map: HashMap<&str, usize> = HashMap::new();
        for conn in &pipeline.connections {
            *map.entry(conn.to.as_str()).or_insert(0) += 1;
        }
        map
    };

    let outgoing: HashMap<&str, usize> = {
        let mut map: HashMap<&str, usize> = HashMap::new();
        for conn in &pipeline.connections {
            *map.entry(conn.from.as_str()).or_insert(0) += 1;
        }
        map
    };

    for comp in &pipeline.components {
        if comp.role == ComponentRole::Source
            && incoming
                .get(comp.name.as_str())
                .copied()
                .unwrap_or(0)
                > 0
        {
            result.push(
                DiagnosticBuilder::error(
                    ErrorCode::SourceHasIncoming,
                    format!("source component '{}' has incoming connections", comp.name),
                )
                .help("sources produce data and should not have incoming connections")
                .build(),
            );
        }
        if comp.role == ComponentRole::Sink
            && outgoing
                .get(comp.name.as_str())
                .copied()
                .unwrap_or(0)
                > 0
        {
            result.push(
                DiagnosticBuilder::error(
                    ErrorCode::SinkHasOutgoing,
                    format!("sink component '{}' has outgoing connections", comp.name),
                )
                .help("sinks consume data and should not have outgoing connections")
                .build(),
            );
        }
    }

    // Simple cycle detection using DFS
    detect_cycles(pipeline, result);
}

/// Detect cycles in the pipeline topology using DFS.
fn detect_cycles(pipeline: &PipelineDefinition, result: &mut ValidationResult) {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for conn in &pipeline.connections {
        adj.entry(conn.from.as_str())
            .or_default()
            .push(conn.to.as_str());
    }

    let mut visited = HashSet::new();
    let mut in_stack = HashSet::new();

    for comp in &pipeline.components {
        if !visited.contains(comp.name.as_str())
            && has_cycle_dfs(comp.name.as_str(), &adj, &mut visited, &mut in_stack)
        {
            result.push(
                DiagnosticBuilder::error(
                    ErrorCode::InvalidTopology,
                    "pipeline topology contains a cycle",
                )
                .note(format!(
                    "cycle detected starting from component '{}'",
                    comp.name
                ))
                .help("pipeline must be a directed acyclic graph (DAG)")
                .build(),
            );
            return; // One cycle error is enough
        }
    }
}

/// DFS cycle detection helper. Returns true if a cycle is found.
fn has_cycle_dfs<'a>(
    node: &'a str,
    adj: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    in_stack: &mut HashSet<&'a str>,
) -> bool {
    visited.insert(node);
    in_stack.insert(node);

    if let Some(neighbors) = adj.get(node) {
        for &neighbor in neighbors {
            if !visited.contains(neighbor) {
                if has_cycle_dfs(neighbor, adj, visited, in_stack) {
                    return true;
                }
            } else if in_stack.contains(neighbor) {
                return true;
            }
        }
    }

    in_stack.remove(node);
    false
}

/// Validate interface compatibility between connected components.
///
/// Only compares shared *interfaces* (the contract types), not worlds.
/// Different components naturally have different worlds (source vs sink),
/// so world comparison would produce false positives.
fn validate_interface_compatibility(
    pipeline: &PipelineDefinition,
    component_map: &HashMap<&str, &PipelineComponent>,
    result: &mut ValidationResult,
) {
    for conn in &pipeline.connections {
        let upstream = match component_map.get(conn.from.as_str()) {
            Some(c) => c,
            None => continue,
        };
        let downstream = match component_map.get(conn.to.as_str()) {
            Some(c) => c,
            None => continue,
        };

        // Compare only interfaces (not worlds) between connected components.
        // Different component roles have different worlds by design.
        for up_pkg in &upstream.packages {
            for down_pkg in &downstream.packages {
                if up_pkg.name == down_pkg.name {
                    // LLI DEVIATION: Build interface-only packages for comparison.
                    // check_compatibility compares worlds too, which would false-positive
                    // when a source connects to a sink (different worlds are expected).
                    let up_iface_only = ParsedPackage {
                        name: up_pkg.name.clone(),
                        version: up_pkg.version.clone(),
                        interfaces: up_pkg.interfaces.clone(),
                        worlds: HashMap::new(),
                        source_files: vec![],
                    };
                    let down_iface_only = ParsedPackage {
                        name: down_pkg.name.clone(),
                        version: down_pkg.version.clone(),
                        interfaces: down_pkg.interfaces.clone(),
                        worlds: HashMap::new(),
                        source_files: vec![],
                    };
                    let report = check_compatibility(&up_iface_only, &down_iface_only);
                    if report.verdict == CompatibilityVerdict::Breaking {
                        result.push(
                            DiagnosticBuilder::error(
                                ErrorCode::IncompatibleMajorVersion,
                                format!(
                                    "incompatible contract versions between '{}' and '{}'",
                                    conn.from, conn.to
                                ),
                            )
                            .note(format!(
                                "'{}' uses {}@{}, '{}' uses {}@{}",
                                conn.from,
                                up_pkg.name,
                                up_pkg
                                    .version
                                    .as_ref()
                                    .map_or("?".into(), |v| v.to_string()),
                                conn.to,
                                down_pkg.name,
                                down_pkg
                                    .version
                                    .as_ref()
                                    .map_or("?".into(), |v| v.to_string()),
                            ))
                            .help("ensure both components are compiled against compatible contract versions")
                            .build(),
                        );
                    }
                }
            }
        }
    }
}

/// Validate that all required capabilities are granted.
fn validate_capabilities(pipeline: &PipelineDefinition, result: &mut ValidationResult) {
    for comp in &pipeline.components {
        let grants: HashSet<&str> = pipeline
            .capability_grants
            .get(&comp.name)
            .map(|gs| gs.iter().map(|g| g.name.as_str()).collect())
            .unwrap_or_default();

        for required in &comp.manifest.required_capabilities {
            if !grants.contains(required.as_str()) {
                result.push(
                    DiagnosticBuilder::error(
                        ErrorCode::UnmetCapability,
                        format!(
                            "component '{}' requires capability '{}' which is not granted",
                            comp.name, required
                        ),
                    )
                    .help(format!(
                        "add a grant for '{}' in the pipeline configuration under component '{}'",
                        required, comp.name
                    ))
                    .build(),
                );
            }
        }
    }
}

/// Validate that contract version ranges across all components are satisfiable.
fn validate_version_ranges(pipeline: &PipelineDefinition, result: &mut ValidationResult) {
    let mut version_map: HashMap<String, Vec<(&str, semver::Version)>> = HashMap::new();

    for comp in &pipeline.components {
        for pkg in &comp.packages {
            if let Some(ref ver) = pkg.version {
                version_map
                    .entry(pkg.name.clone())
                    .or_default()
                    .push((&comp.name, ver.clone()));
            }
        }
    }

    for (pkg_name, versions) in &version_map {
        if versions.len() < 2 {
            continue;
        }

        let first_major = versions[0].1.major;
        for (comp_name, ver) in &versions[1..] {
            if ver.major != first_major {
                result.push(
                    DiagnosticBuilder::error(
                        ErrorCode::VersionRangeUnsatisfiable,
                        format!(
                            "incompatible major versions for package '{}'",
                            pkg_name
                        ),
                    )
                    .note(format!(
                        "'{}' uses version {}, '{}' uses version {}",
                        versions[0].0, versions[0].1, comp_name, ver
                    ))
                    .help(
                        "all components must use the same major version of each shared package",
                    )
                    .build(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::*;
    use std::collections::HashSet;

    fn make_simple_component(
        name: &str,
        role: ComponentRole,
        version: &str,
    ) -> PipelineComponent {
        let mut exports = HashMap::new();
        let iface_name = match role {
            ComponentRole::Source => "source",
            ComponentRole::Processor => "processor",
            ComponentRole::Sink => "sink",
            ComponentRole::Filter => "filter",
            ComponentRole::Router => "router",
        };
        exports.insert(
            iface_name.to_string(),
            WorldImportExport::Interface(iface_name.to_string()),
        );

        PipelineComponent {
            name: name.to_string(),
            role,
            packages: vec![ParsedPackage {
                name: "torvyn:streaming".into(),
                version: Some(semver::Version::parse(version).unwrap()),
                interfaces: HashMap::new(),
                worlds: HashMap::from([(
                    "main".into(),
                    ParsedWorld {
                        name: "main".into(),
                        imports: HashMap::new(),
                        exports,
                    },
                )]),
                source_files: vec![],
            }],
            manifest: ComponentManifest {
                name: name.to_string(),
                version: semver::Version::parse(version).unwrap(),
                required_capabilities: HashSet::new(),
                optional_capabilities: HashSet::new(),
                resource_limits: Default::default(),
            },
            artifact_path: PathBuf::from(format!("{}.wasm", name)),
            config: None,
        }
    }

    fn make_pipeline(
        components: Vec<PipelineComponent>,
        connections: Vec<(&str, &str)>,
    ) -> PipelineDefinition {
        PipelineDefinition {
            name: "test-pipeline".into(),
            components,
            connections: connections
                .into_iter()
                .map(|(from, to)| PipelineConnection {
                    from: from.into(),
                    to: to.into(),
                    queue_depth: 64,
                    port: None,
                })
                .collect(),
            capability_grants: HashMap::new(),
        }
    }

    #[test]
    fn test_valid_source_sink_pipeline() {
        let source = make_simple_component("my-source", ComponentRole::Source, "0.1.0");
        let sink = make_simple_component("my-sink", ComponentRole::Sink, "0.1.0");
        let pipeline = make_pipeline(vec![source, sink], vec![("my-source", "my-sink")]);

        let result = validate_pipeline(&pipeline);
        assert!(
            result.is_ok(),
            "Expected valid pipeline, got: {}",
            result.format_all()
        );
    }

    #[test]
    fn test_valid_three_stage_pipeline() {
        let source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        let proc = make_simple_component("proc", ComponentRole::Processor, "0.1.0");
        let sink = make_simple_component("snk", ComponentRole::Sink, "0.1.0");
        let pipeline = make_pipeline(
            vec![source, proc, sink],
            vec![("src", "proc"), ("proc", "snk")],
        );

        let result = validate_pipeline(&pipeline);
        assert!(
            result.is_ok(),
            "Expected valid pipeline, got: {}",
            result.format_all()
        );
    }

    #[test]
    fn test_disconnected_component() {
        let source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        let sink = make_simple_component("snk", ComponentRole::Sink, "0.1.0");
        let orphan = make_simple_component("orphan", ComponentRole::Processor, "0.1.0");
        let pipeline = make_pipeline(vec![source, sink, orphan], vec![("src", "snk")]);

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::DisconnectedComponent));
    }

    #[test]
    fn test_source_with_incoming() {
        let source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        let proc = make_simple_component("proc", ComponentRole::Processor, "0.1.0");
        let pipeline = make_pipeline(vec![source, proc], vec![("proc", "src")]);

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::SourceHasIncoming));
    }

    #[test]
    fn test_sink_with_outgoing() {
        let source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        let sink = make_simple_component("snk", ComponentRole::Sink, "0.1.0");
        let proc = make_simple_component("proc", ComponentRole::Processor, "0.1.0");
        let pipeline = make_pipeline(
            vec![source, sink, proc],
            vec![("src", "snk"), ("snk", "proc")],
        );

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::SinkHasOutgoing));
    }

    #[test]
    fn test_cycle_detection() {
        let a = make_simple_component("a", ComponentRole::Processor, "0.1.0");
        let b = make_simple_component("b", ComponentRole::Processor, "0.1.0");
        let pipeline = make_pipeline(vec![a, b], vec![("a", "b"), ("b", "a")]);

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::InvalidTopology));
    }

    #[test]
    fn test_unknown_connection_endpoint() {
        let source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        let mut pipeline = make_pipeline(vec![source], vec![]);
        pipeline.connections.push(PipelineConnection {
            from: "src".into(),
            to: "nonexistent".into(),
            queue_depth: 64,
            port: None,
        });

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
    }

    #[test]
    fn test_unmet_capability() {
        let mut source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        source
            .manifest
            .required_capabilities
            .insert("wasi-filesystem-read".into());

        let sink = make_simple_component("snk", ComponentRole::Sink, "0.1.0");
        let pipeline = make_pipeline(vec![source, sink], vec![("src", "snk")]);

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::UnmetCapability));
    }

    #[test]
    fn test_satisfied_capability() {
        let mut source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        source
            .manifest
            .required_capabilities
            .insert("wasi-filesystem-read".into());

        let sink = make_simple_component("snk", ComponentRole::Sink, "0.1.0");
        let mut pipeline = make_pipeline(vec![source, sink], vec![("src", "snk")]);
        pipeline.capability_grants.insert(
            "src".into(),
            vec![CapabilityGrant {
                name: "wasi-filesystem-read".into(),
                details: String::new(),
            }],
        );

        let result = validate_pipeline(&pipeline);
        assert!(
            result.is_ok(),
            "Expected valid pipeline, got: {}",
            result.format_all()
        );
    }

    #[test]
    fn test_incompatible_major_versions() {
        let source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        let sink = make_simple_component("snk", ComponentRole::Sink, "1.0.0");
        let pipeline = make_pipeline(vec![source, sink], vec![("src", "snk")]);

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == ErrorCode::VersionRangeUnsatisfiable));
    }

    #[test]
    fn test_compatible_minor_version_difference() {
        let source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        let sink = make_simple_component("snk", ComponentRole::Sink, "0.2.0");
        let pipeline = make_pipeline(vec![source, sink], vec![("src", "snk")]);

        let result = validate_pipeline(&pipeline);
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.code == ErrorCode::VersionRangeUnsatisfiable),
            "Same major version should not trigger VersionRangeUnsatisfiable"
        );
    }

    #[test]
    fn test_reports_all_errors() {
        let mut source = make_simple_component("src", ComponentRole::Source, "0.1.0");
        source
            .manifest
            .required_capabilities
            .insert("wasi-filesystem-read".into());

        let sink = make_simple_component("snk", ComponentRole::Sink, "1.0.0");
        let orphan = make_simple_component("orphan", ComponentRole::Processor, "0.1.0");

        let pipeline = make_pipeline(vec![source, sink, orphan], vec![("src", "snk")]);

        let result = validate_pipeline(&pipeline);
        assert!(!result.is_ok());
        assert!(
            result.diagnostics.len() >= 2,
            "Expected multiple errors, got {}",
            result.diagnostics.len()
        );
    }
}
