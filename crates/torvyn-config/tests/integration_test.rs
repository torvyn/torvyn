//! Integration tests for the torvyn-config crate.
//!
//! Tests cross-module interactions: loading a manifest, validating it,
//! extracting pipeline flows, and merging configurations.

use torvyn_config::*;

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

// ---------------------------------------------------------------------------
// Golden tests with fixture files
// ---------------------------------------------------------------------------

#[test]
fn test_parse_minimal_fixture() {
    let path = format!("{FIXTURES_DIR}/minimal.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let manifest = ComponentManifest::from_toml_str(&content, &path).unwrap();
    assert_eq!(manifest.torvyn.name, "my-transform");
    assert_eq!(manifest.torvyn.version, "0.1.0");
    assert_eq!(manifest.torvyn.contract_version, "0.1.0");
    assert!(!manifest.is_workspace());
    assert!(!manifest.has_flows());
}

#[test]
fn test_parse_full_manifest_fixture() {
    let path = format!("{FIXTURES_DIR}/full_manifest.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let manifest = ComponentManifest::from_toml_str(&content, &path).unwrap();
    assert_eq!(manifest.torvyn.name, "token-pipeline");
    assert!(manifest.is_workspace());
    assert_eq!(manifest.components.len(), 3);
    assert!(manifest.has_flows());
    assert_eq!(manifest.runtime.worker_threads, 2);
    assert_eq!(manifest.security.default_capability_policy, "deny-all");
    assert_eq!(manifest.security.grants.len(), 2);
    assert!(manifest.registry.default_url.is_some());

    // Validate semantically
    let errors = validate_manifest(&manifest, &path);
    assert!(errors.is_empty(), "Unexpected errors: {errors}");
}

#[test]
fn test_parse_pipeline_fixture() {
    let path = format!("{FIXTURES_DIR}/pipeline.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let pipeline = PipelineDefinition::from_toml_str(&content, &path).unwrap();
    assert!(pipeline.flows.contains_key("ingest"));
    let flow = &pipeline.flows["ingest"];
    assert_eq!(flow.nodes.len(), 3);
    assert_eq!(flow.edges.len(), 2);
    assert_eq!(flow.nodes["reader"].fuel_budget, Some(2_000_000));
    assert_eq!(flow.nodes["transformer"].priority, Some(8));
    assert_eq!(flow.edges[0].queue_depth, Some(128));
    assert_eq!(pipeline.runtime.as_ref().unwrap().worker_threads, 4);
}

// ---------------------------------------------------------------------------
// Invalid config fixture tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_invalid_name_fixture() {
    let path = format!("{FIXTURES_DIR}/invalid_name.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let result = ComponentManifest::from_toml_str(&content, &path);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.code == "E0703"));
    assert!(errors.iter().any(|e| e.key_path.contains("torvyn.name")));
}

#[test]
fn test_parse_invalid_syntax_fixture() {
    let path = format!("{FIXTURES_DIR}/invalid_syntax.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let result = ComponentManifest::from_toml_str(&content, &path);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.code == "E0701"));
}

#[test]
fn test_parse_missing_fields_fixture() {
    let path = format!("{FIXTURES_DIR}/missing_fields.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let result = ComponentManifest::from_toml_str(&content, &path);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    // Should have at least 3 errors: empty name, bad version, empty contract_version
    assert!(
        errors.len() >= 3,
        "Expected >= 3 errors, got {}",
        errors.len()
    );
}

// ---------------------------------------------------------------------------
// Defaults verification
// ---------------------------------------------------------------------------

#[test]
fn test_defaults_all_applied_for_minimal_manifest() {
    let path = format!("{FIXTURES_DIR}/minimal.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let manifest = ComponentManifest::from_toml_str(&content, &path).unwrap();

    // Runtime defaults
    assert_eq!(manifest.runtime.worker_threads, 0);
    assert_eq!(manifest.runtime.max_memory_per_component, "16MiB");
    assert_eq!(manifest.runtime.default_fuel_per_invocation, 1_000_000);
    assert_eq!(manifest.runtime.compilation_cache_dir, ".torvyn/cache");

    // Scheduling defaults
    assert_eq!(manifest.runtime.scheduling.policy, "weighted-fair");
    assert_eq!(manifest.runtime.scheduling.default_priority, 5);

    // Backpressure defaults (per Doc 10 C02-2, C02-3)
    assert_eq!(manifest.runtime.backpressure.default_queue_depth, 64);
    assert_eq!(
        manifest.runtime.backpressure.backpressure_policy,
        "block-producer"
    );

    // Observability defaults
    assert!(manifest.observability.tracing_enabled);
    assert_eq!(manifest.observability.tracing_exporter, "stdout");
    assert_eq!(manifest.observability.tracing_sample_rate, 1.0);
    assert!(manifest.observability.metrics_enabled);
    assert_eq!(manifest.observability.metrics_exporter, "none");

    // Security defaults
    assert_eq!(manifest.security.default_capability_policy, "deny-all");
    assert!(manifest.security.grants.is_empty());

    // Build defaults
    assert!(manifest.build.release);
    assert_eq!(manifest.build.target, "wasm32-wasip2");

    // Test defaults
    assert_eq!(manifest.test.timeout_seconds, 60);
}

// ---------------------------------------------------------------------------
// Round-trip tests
// ---------------------------------------------------------------------------

#[test]
fn test_round_trip_manifest_preserves_all_fields() {
    let toml_str = r#"
[torvyn]
name = "round-trip"
version = "1.2.3"
contract_version = "0.1.0"
description = "test desc"
authors = ["Ashutosh Mishra"]
license = "MIT"
repository = "https://github.com/torvyn/torvyn"

[build]
release = false
target = "wasm32-wasip2"

[test]
timeout_seconds = 120
fixtures_dir = "my-fixtures"

[runtime]
worker_threads = 4
max_memory_per_component = "32MiB"

[runtime.scheduling]
policy = "priority"
default_priority = 8

[runtime.backpressure]
default_queue_depth = 256
backpressure_policy = "drop-oldest"

[observability]
tracing_enabled = false
tracing_exporter = "otlp-grpc"
tracing_endpoint = "http://otel:4317"
tracing_sample_rate = 0.5
metrics_enabled = false
metrics_exporter = "prometheus"
metrics_endpoint = "0.0.0.0:9999"

[security]
default_capability_policy = "deny-all"

[security.grants.my-comp]
capabilities = ["filesystem:read:*"]

[registry]
default = "https://registry.example.com"
"#;

    let original = ComponentManifest::from_toml_str(toml_str, "f").unwrap();
    let serialized = original.to_toml_string().unwrap();
    let reparsed = ComponentManifest::from_toml_str(&serialized, "f").unwrap();

    assert_eq!(original.torvyn.name, reparsed.torvyn.name);
    assert_eq!(original.torvyn.version, reparsed.torvyn.version);
    assert_eq!(original.torvyn.description, reparsed.torvyn.description);
    assert_eq!(original.torvyn.authors, reparsed.torvyn.authors);
    assert_eq!(original.torvyn.license, reparsed.torvyn.license);
    assert_eq!(original.build.release, reparsed.build.release);
    assert_eq!(original.test.timeout_seconds, reparsed.test.timeout_seconds);
    assert_eq!(
        original.runtime.worker_threads,
        reparsed.runtime.worker_threads
    );
    assert_eq!(
        original.runtime.scheduling.policy,
        reparsed.runtime.scheduling.policy
    );
    assert_eq!(
        original.runtime.backpressure.default_queue_depth,
        reparsed.runtime.backpressure.default_queue_depth
    );
    assert_eq!(
        original.observability.tracing_exporter,
        reparsed.observability.tracing_exporter
    );
    assert_eq!(
        original.security.grants.len(),
        reparsed.security.grants.len()
    );
}

#[test]
fn test_round_trip_pipeline() {
    let path = format!("{FIXTURES_DIR}/pipeline.toml");
    let content = std::fs::read_to_string(&path).unwrap();
    let original = PipelineDefinition::from_toml_str(&content, &path).unwrap();
    let serialized = toml::to_string_pretty(&original).unwrap();
    let reparsed = PipelineDefinition::from_toml_str(&serialized, &path).unwrap();
    assert_eq!(original.flows.len(), reparsed.flows.len());
    assert_eq!(
        original.flows["ingest"].nodes.len(),
        reparsed.flows["ingest"].nodes.len()
    );
    assert_eq!(
        original.flows["ingest"].edges.len(),
        reparsed.flows["ingest"].edges.len()
    );
}

// ---------------------------------------------------------------------------
// Config merging tests
// ---------------------------------------------------------------------------

#[test]
fn test_config_merge_pipeline_over_manifest() {
    let base = RuntimeConfig {
        worker_threads: 2,
        max_memory_per_component: "8MiB".to_owned(),
        ..Default::default()
    };
    let pipeline_override = RuntimeConfig {
        worker_threads: 4,
        ..Default::default()
    };

    let merged = merge_runtime_config(&base, &pipeline_override);
    assert_eq!(merged.worker_threads, 4);
    assert_eq!(merged.max_memory_per_component, "8MiB");
}

// ---------------------------------------------------------------------------
// Environment variable overlay tests
// ---------------------------------------------------------------------------

#[test]
fn test_env_interpolation_in_config_value() {
    std::env::set_var("TORVYN_IT_TEST_VAR", "hello");
    let result = interpolate_env("say ${TORVYN_IT_TEST_VAR}", "f.toml", "k").unwrap();
    assert_eq!(result, "say hello");
    std::env::remove_var("TORVYN_IT_TEST_VAR");
}

#[test]
fn test_env_interpolation_missing_var_error() {
    let result = interpolate_env("${NO_SUCH_VAR_EVER_XYZ_123}", "f.toml", "k");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, "E0705");
}

#[test]
fn test_env_overrides_collected() {
    std::env::set_var("TORVYN_IT_RUNTIME_WORKER_THREADS", "16");
    let overrides = collect_env_overrides();
    assert!(overrides.contains_key("it.runtime.worker.threads"));
    std::env::remove_var("TORVYN_IT_RUNTIME_WORKER_THREADS");
}

// ---------------------------------------------------------------------------
// Cross-module integration: full pipeline config parse + validate
// ---------------------------------------------------------------------------

#[test]
fn test_full_pipeline_config_parses_and_validates() {
    let toml_str = r#"
[torvyn]
name = "integration-test"
version = "0.1.0"
contract_version = "0.1.0"

[[component]]
name = "source"
path = "components/source"

[[component]]
name = "sink"
path = "components/sink"

[flow.main.nodes.source]
component = "file://./source.wasm"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "file://./sink.wasm"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }

[runtime]
worker_threads = 2
max_memory_per_component = "8MiB"

[runtime.backpressure]
default_queue_depth = 128
backpressure_policy = "block-producer"

[security]
default_capability_policy = "deny-all"

[security.grants.source]
capabilities = ["filesystem:read:/data/*"]
"#;

    // Parse manifest
    let manifest = ComponentManifest::from_toml_str(toml_str, "Torvyn.toml").unwrap();
    assert_eq!(manifest.torvyn.name, "integration-test");
    assert!(manifest.is_workspace());
    assert!(manifest.has_flows());

    // Validate semantically
    let errors = validate_manifest(&manifest, "Torvyn.toml");
    assert!(errors.is_empty(), "Unexpected errors: {errors}");

    // Extract inline flows
    let flows = PipelineDefinition::from_manifest_flows(&manifest.flow, "Torvyn.toml").unwrap();
    assert!(flows.contains_key("main"));
    let main_flow = &flows["main"];
    assert_eq!(main_flow.nodes.len(), 2);
    assert_eq!(main_flow.edges.len(), 1);

    // Verify runtime config
    assert_eq!(manifest.runtime.worker_threads, 2);
    assert_eq!(manifest.runtime.backpressure.default_queue_depth, 128);

    // Verify security config
    assert_eq!(manifest.security.default_capability_policy, "deny-all");
    assert!(manifest.security.grants.contains_key("source"));
}

#[test]
fn test_standalone_pipeline_file_parses() {
    let toml_str = r#"
[flow.ingest]
description = "test"

[flow.ingest.nodes.src]
component = "a.wasm"
interface = "torvyn:streaming/source"
priority = 3
fuel_budget = 500000

[flow.ingest.nodes.sink]
component = "b.wasm"
interface = "torvyn:streaming/sink"

[[flow.ingest.edges]]
from = { node = "src", port = "output" }
to = { node = "sink", port = "input" }
queue_depth = 256

[runtime]
worker_threads = 8

[security]
default_capability_policy = "deny-all"
"#;
    let pipeline = PipelineDefinition::from_toml_str(toml_str, "pipeline.toml").unwrap();
    assert!(pipeline.flows.contains_key("ingest"));
    let flow = &pipeline.flows["ingest"];
    assert_eq!(flow.nodes["src"].priority, Some(3));
    assert_eq!(flow.nodes["src"].fuel_budget, Some(500_000));
    assert_eq!(flow.edges[0].queue_depth, Some(256));
    assert_eq!(pipeline.runtime.as_ref().unwrap().worker_threads, 8);
}

// ---------------------------------------------------------------------------
// Loader tests (filesystem)
// ---------------------------------------------------------------------------

#[test]
fn test_load_config_from_fixture() {
    let path = format!("{FIXTURES_DIR}/minimal.toml");
    let manifest = load_manifest(&path).unwrap();
    assert_eq!(manifest.torvyn.name, "my-transform");
}

#[test]
fn test_load_config_file_not_found() {
    let result = load_config("/nonexistent/path/Torvyn.toml");
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.code == "E0700"));
}

#[test]
fn test_load_pipeline_from_fixture() {
    let path = format!("{FIXTURES_DIR}/pipeline.toml");
    let pipeline = load_pipeline(&path).unwrap();
    assert!(pipeline.flows.contains_key("ingest"));
}
