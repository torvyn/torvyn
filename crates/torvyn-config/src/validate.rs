//! Semantic validation for parsed configuration.
//!
//! Runs after TOML deserialization to check cross-field constraints,
//! value ranges, and structural rules that serde cannot enforce.

use crate::error::{ConfigErrors, ConfigParseError};
use crate::manifest::ComponentManifest;
use crate::pipeline::PipelineDefinition;
use crate::runtime::parse_memory_size;

/// Valid scheduling policy names.
const VALID_SCHEDULING_POLICIES: &[&str] = &["round-robin", "weighted-fair", "priority"];

/// Valid backpressure policy names.
const VALID_BACKPRESSURE_POLICIES: &[&str] =
    &["block-producer", "drop-oldest", "drop-newest", "error"];

/// Valid tracing exporter names.
const VALID_TRACING_EXPORTERS: &[&str] = &["otlp-grpc", "otlp-http", "stdout", "none"];

/// Valid metrics exporter names.
const VALID_METRICS_EXPORTERS: &[&str] = &["prometheus", "otlp", "none"];

/// Valid capability policy names.
const VALID_CAPABILITY_POLICIES: &[&str] = &["deny-all", "allow-all"];

/// Validate a `ComponentManifest` semantically.
///
/// This checks value ranges, enum validity, and cross-field consistency
/// beyond what the structural parsing in `manifest.rs` checks.
///
/// # COLD PATH — called once after parsing.
///
/// # Examples
/// ```
/// use torvyn_config::{ComponentManifest, validate_manifest};
///
/// let toml_str = r#"
/// [torvyn]
/// name = "test"
/// version = "0.1.0"
/// contract_version = "0.1.0"
/// "#;
/// let manifest = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml").unwrap();
/// let errors = validate_manifest(&manifest, "Torvyn.toml");
/// assert!(errors.is_empty());
/// ```
pub fn validate_manifest(manifest: &ComponentManifest, file: &str) -> ConfigErrors {
    let mut errors = ConfigErrors::new();

    validate_runtime_config(&manifest.runtime, file, &mut errors);
    validate_observability_config(&manifest.observability, file, &mut errors);
    validate_security_config(&manifest.security, file, &mut errors);

    errors
}

/// Validate a `PipelineDefinition` semantically.
///
/// # COLD PATH — called once after parsing.
pub fn validate_pipeline(pipeline: &PipelineDefinition, file: &str) -> ConfigErrors {
    let mut errors = ConfigErrors::new();

    if let Some(ref rt) = pipeline.runtime {
        validate_runtime_config(rt, file, &mut errors);
    }
    if let Some(ref obs) = pipeline.observability {
        validate_observability_config(obs, file, &mut errors);
    }
    if let Some(ref sec) = pipeline.security {
        validate_security_config(sec, file, &mut errors);
    }

    // Validate per-flow scheduling policies
    for (name, flow) in &pipeline.flows {
        if let Some(ref policy) = flow.scheduling_policy {
            if !VALID_SCHEDULING_POLICIES.contains(&policy.as_str()) {
                errors.push(ConfigParseError::invalid_value(
                    file,
                    &format!("flow.{name}.scheduling_policy"),
                    policy,
                    &format!("one of: {}", VALID_SCHEDULING_POLICIES.join(", ")),
                    "Use a supported scheduling policy.",
                ));
            }
        }
    }

    errors
}

/// Validate runtime configuration fields.
///
/// # COLD PATH.
fn validate_runtime_config(
    config: &crate::runtime::RuntimeConfig,
    file: &str,
    errors: &mut ConfigErrors,
) {
    // Validate scheduling policy
    if !VALID_SCHEDULING_POLICIES.contains(&config.scheduling.policy.as_str()) {
        errors.push(ConfigParseError::invalid_value(
            file,
            "runtime.scheduling.policy",
            &config.scheduling.policy,
            &format!("one of: {}", VALID_SCHEDULING_POLICIES.join(", ")),
            "Use a supported scheduling policy.",
        ));
    }

    // Validate priority range
    if config.scheduling.default_priority == 0 || config.scheduling.default_priority > 10 {
        errors.push(ConfigParseError::invalid_value(
            file,
            "runtime.scheduling.default_priority",
            &config.scheduling.default_priority.to_string(),
            "an integer between 1 and 10",
            "Set priority to a value from 1 (lowest) to 10 (highest).",
        ));
    }

    // Validate backpressure policy
    if !VALID_BACKPRESSURE_POLICIES.contains(&config.backpressure.backpressure_policy.as_str()) {
        errors.push(ConfigParseError::invalid_value(
            file,
            "runtime.backpressure.backpressure_policy",
            &config.backpressure.backpressure_policy,
            &format!("one of: {}", VALID_BACKPRESSURE_POLICIES.join(", ")),
            "Use a supported backpressure policy.",
        ));
    }

    // Validate queue depth
    if config.backpressure.default_queue_depth == 0 {
        errors.push(ConfigParseError::invalid_value(
            file,
            "runtime.backpressure.default_queue_depth",
            "0",
            "a positive integer",
            "Queue depth must be at least 1.",
        ));
    }

    // Validate memory size is parseable
    if let Err(reason) = parse_memory_size(&config.max_memory_per_component) {
        errors.push(ConfigParseError::invalid_value(
            file,
            "runtime.max_memory_per_component",
            &config.max_memory_per_component,
            "a valid memory size (e.g., \"16MiB\", \"64MiB\")",
            &format!("{reason}. Use B, KiB, MiB, or GiB suffixes."),
        ));
    }

    // Validate fuel > 0
    if config.default_fuel_per_invocation == 0 {
        errors.push(ConfigParseError::invalid_value(
            file,
            "runtime.default_fuel_per_invocation",
            "0",
            "a positive integer",
            "Fuel budget must be at least 1.",
        ));
    }
}

/// Validate observability configuration fields.
///
/// # COLD PATH.
fn validate_observability_config(
    config: &crate::runtime::ObservabilityConfig,
    file: &str,
    errors: &mut ConfigErrors,
) {
    if !VALID_TRACING_EXPORTERS.contains(&config.tracing_exporter.as_str()) {
        errors.push(ConfigParseError::invalid_value(
            file,
            "observability.tracing_exporter",
            &config.tracing_exporter,
            &format!("one of: {}", VALID_TRACING_EXPORTERS.join(", ")),
            "Use a supported tracing exporter.",
        ));
    }

    if !(0.0..=1.0).contains(&config.tracing_sample_rate) {
        errors.push(ConfigParseError::invalid_value(
            file,
            "observability.tracing_sample_rate",
            &config.tracing_sample_rate.to_string(),
            "a float between 0.0 and 1.0",
            "Set sample rate to a value from 0.0 (none) to 1.0 (all).",
        ));
    }

    if !VALID_METRICS_EXPORTERS.contains(&config.metrics_exporter.as_str()) {
        errors.push(ConfigParseError::invalid_value(
            file,
            "observability.metrics_exporter",
            &config.metrics_exporter,
            &format!("one of: {}", VALID_METRICS_EXPORTERS.join(", ")),
            "Use a supported metrics exporter.",
        ));
    }
}

/// Validate security configuration fields.
///
/// # COLD PATH.
fn validate_security_config(
    config: &crate::runtime::SecurityConfig,
    file: &str,
    errors: &mut ConfigErrors,
) {
    if !VALID_CAPABILITY_POLICIES.contains(&config.default_capability_policy.as_str()) {
        errors.push(ConfigParseError::invalid_value(
            file,
            "security.default_capability_policy",
            &config.default_capability_policy,
            &format!("one of: {}", VALID_CAPABILITY_POLICIES.join(", ")),
            "Use 'deny-all' (recommended) or 'allow-all' (development only).",
        ));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ComponentManifest;

    #[test]
    fn test_validate_manifest_valid_defaults() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(errors.is_empty(), "expected no errors, got: {errors}");
    }

    #[test]
    fn test_validate_invalid_scheduling_policy() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[runtime.scheduling]
policy = "random"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
        assert!(errors
            .iter()
            .any(|e| e.key_path.contains("scheduling.policy")));
    }

    #[test]
    fn test_validate_invalid_backpressure_policy() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[runtime.backpressure]
backpressure_policy = "explode"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_invalid_memory_size() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[runtime]
max_memory_per_component = "not-a-size"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_zero_fuel_is_error() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[runtime]
default_fuel_per_invocation = 0
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_invalid_sample_rate() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[observability]
tracing_sample_rate = 2.0
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_invalid_capability_policy() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[security]
default_capability_policy = "yolo"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_zero_queue_depth_is_error() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[runtime.backpressure]
default_queue_depth = 0
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_priority_out_of_range() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[runtime.scheduling]
default_priority = 0
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_pipeline_with_invalid_flow_scheduling_policy() {
        let toml_str = r#"
[flow.main]
scheduling_policy = "invalid"

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
        let pipeline = PipelineDefinition::from_toml_str(toml_str, "p.toml").unwrap();
        let errors = validate_pipeline(&pipeline, "p.toml");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_invalid_tracing_exporter() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[observability]
tracing_exporter = "magic"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_invalid_metrics_exporter() {
        let toml_str = r#"
[torvyn]
name = "test"
version = "0.1.0"
contract_version = "0.1.0"

[observability]
metrics_exporter = "magic"
"#;
        let manifest = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
        let errors = validate_manifest(&manifest, "f");
        assert!(!errors.is_empty());
    }
}
