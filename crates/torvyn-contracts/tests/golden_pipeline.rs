//! Golden path integration test: a known-good two-component pipeline
//! that passes all validation.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use torvyn_contracts::linker::*;
use torvyn_contracts::parser::*;
use torvyn_contracts::validator::*;

/// Build a realistic two-component pipeline (source → sink) and
/// verify that all validation passes.
#[test]
fn golden_source_sink_pipeline_passes_all_validation() {
    // --- Build source component ---
    let source_exports = HashMap::from([
        (
            "source".to_string(),
            WorldImportExport::Interface("source".to_string()),
        ),
        (
            "lifecycle".to_string(),
            WorldImportExport::Interface("lifecycle".to_string()),
        ),
    ]);
    let source_imports = HashMap::from([
        (
            "torvyn:streaming/types".to_string(),
            WorldImportExport::Interface("types".to_string()),
        ),
        (
            "torvyn:streaming/buffer-allocator".to_string(),
            WorldImportExport::Interface("buffer-allocator".to_string()),
        ),
    ]);

    let source_world = ParsedWorld {
        name: "data-source".to_string(),
        imports: source_imports,
        exports: source_exports,
    };

    let source_pkg = ParsedPackage {
        name: "torvyn:streaming".to_string(),
        version: Some(semver::Version::new(0, 1, 0)),
        interfaces: HashMap::new(),
        worlds: HashMap::from([("data-source".to_string(), source_world)]),
        source_files: vec![PathBuf::from("wit/torvyn-streaming/source.wit")],
    };

    let source_component = PipelineComponent {
        name: "token-source".to_string(),
        role: ComponentRole::Source,
        packages: vec![source_pkg],
        manifest: ComponentManifest {
            name: "token-source".to_string(),
            version: semver::Version::new(0, 1, 0),
            required_capabilities: HashSet::new(),
            optional_capabilities: HashSet::new(),
            resource_limits: ResourceLimits::default(),
        },
        artifact_path: PathBuf::from("target/token_source.wasm"),
        config: None,
    };

    // --- Build sink component ---
    let sink_exports = HashMap::from([
        (
            "sink".to_string(),
            WorldImportExport::Interface("sink".to_string()),
        ),
        (
            "lifecycle".to_string(),
            WorldImportExport::Interface("lifecycle".to_string()),
        ),
    ]);
    let sink_imports = HashMap::from([(
        "torvyn:streaming/types".to_string(),
        WorldImportExport::Interface("types".to_string()),
    )]);

    let sink_world = ParsedWorld {
        name: "data-sink".to_string(),
        imports: sink_imports,
        exports: sink_exports,
    };

    let sink_pkg = ParsedPackage {
        name: "torvyn:streaming".to_string(),
        version: Some(semver::Version::new(0, 1, 0)),
        interfaces: HashMap::new(),
        worlds: HashMap::from([("data-sink".to_string(), sink_world)]),
        source_files: vec![PathBuf::from("wit/torvyn-streaming/sink.wit")],
    };

    let sink_component = PipelineComponent {
        name: "output-sink".to_string(),
        role: ComponentRole::Sink,
        packages: vec![sink_pkg],
        manifest: ComponentManifest {
            name: "output-sink".to_string(),
            version: semver::Version::new(0, 1, 0),
            required_capabilities: HashSet::new(),
            optional_capabilities: HashSet::new(),
            resource_limits: ResourceLimits::default(),
        },
        artifact_path: PathBuf::from("target/output_sink.wasm"),
        config: None,
    };

    // --- Build pipeline ---
    let pipeline = PipelineDefinition {
        name: "golden-test-pipeline".to_string(),
        components: vec![source_component, sink_component],
        connections: vec![PipelineConnection {
            from: "token-source".to_string(),
            to: "output-sink".to_string(),
            queue_depth: 64,
            port: None,
        }],
        capability_grants: HashMap::new(),
    };

    // --- Validate ---
    let result = torvyn_contracts::validate_pipeline(&pipeline);

    assert!(
        result.is_ok(),
        "Golden pipeline should pass all validation.\nErrors:\n{}",
        result.format_all()
    );
}

/// Test WIT file parsing using the real wit-parser backend.
#[cfg(feature = "wit-parser-backend")]
#[test]
fn parse_bundled_streaming_wit_files() {
    let parser = torvyn_contracts::WitParserImpl::new();
    let streaming_path = torvyn_contracts::wit_streaming_path();
    let path = std::path::Path::new(&streaming_path);

    let packages = parser
        .parse_directory(path)
        .expect("bundled streaming WIT files should parse successfully");

    assert!(!packages.is_empty(), "should find at least one package");

    let pkg = &packages[0];
    assert_eq!(pkg.name, "torvyn:streaming");
    assert_eq!(pkg.version, Some(semver::Version::new(0, 1, 0)));

    // Verify key interfaces exist
    assert!(
        pkg.interfaces.contains_key("types"),
        "should have 'types' interface"
    );
    assert!(
        pkg.interfaces.contains_key("processor"),
        "should have 'processor' interface"
    );
    assert!(
        pkg.interfaces.contains_key("source"),
        "should have 'source' interface"
    );
    assert!(
        pkg.interfaces.contains_key("sink"),
        "should have 'sink' interface"
    );
    assert!(
        pkg.interfaces.contains_key("lifecycle"),
        "should have 'lifecycle' interface"
    );
    assert!(
        pkg.interfaces.contains_key("buffer-allocator"),
        "should have 'buffer-allocator' interface"
    );

    // Verify key worlds exist
    assert!(
        pkg.worlds.contains_key("transform"),
        "should have 'transform' world"
    );
    assert!(
        pkg.worlds.contains_key("managed-transform"),
        "should have 'managed-transform' world"
    );
    assert!(
        pkg.worlds.contains_key("data-source"),
        "should have 'data-source' world"
    );
    assert!(
        pkg.worlds.contains_key("data-sink"),
        "should have 'data-sink' world"
    );

    // Verify types interface has the split buffer model (I-01)
    let types_iface = &pkg.interfaces["types"];
    assert!(
        types_iface.resources.contains(&"buffer".to_string()),
        "should have 'buffer' resource"
    );
    assert!(
        types_iface.resources.contains(&"mutable-buffer".to_string()),
        "should have 'mutable-buffer' resource"
    );
    assert!(
        types_iface.resources.contains(&"flow-context".to_string()),
        "should have 'flow-context' resource"
    );
}
