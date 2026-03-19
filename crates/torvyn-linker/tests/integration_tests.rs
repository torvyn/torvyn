//! Integration tests for torvyn-linker.
//!
//! These tests verify cross-module interactions within the crate.

use std::collections::HashMap;
use torvyn_linker::*;
use torvyn_types::{BackpressurePolicy, ComponentRole};

fn source_node(name: &str) -> TopologyNode {
    TopologyNode {
        name: name.into(),
        role: ComponentRole::Source,
        artifact_path: format!("{name}.wasm").into(),
        config: None,
        capability_grants: vec![],
    }
}

fn processor_node(name: &str) -> TopologyNode {
    TopologyNode {
        name: name.into(),
        role: ComponentRole::Processor,
        artifact_path: format!("{name}.wasm").into(),
        config: None,
        capability_grants: vec![],
    }
}

fn sink_node(name: &str) -> TopologyNode {
    TopologyNode {
        name: name.into(),
        role: ComponentRole::Sink,
        artifact_path: format!("{name}.wasm").into(),
        config: None,
        capability_grants: vec![],
    }
}

fn edge(from: &str, to: &str) -> TopologyEdge {
    TopologyEdge {
        from_node: from.into(),
        from_port: "output".into(),
        to_node: to.into(),
        to_port: "input".into(),
        queue_depth: 64,
        backpressure_policy: BackpressurePolicy::default(),
    }
}

#[test]
fn integration_full_three_stage_pipeline() {
    let mut topo = PipelineTopology::new("full-test".into());
    topo.add_node(source_node("source-1"));
    topo.add_node(processor_node("transform-1"));
    topo.add_node(sink_node("sink-1"));
    topo.add_edge(edge("source-1", "transform-1"));
    topo.add_edge(edge("transform-1", "sink-1"));

    let mut imports = HashMap::new();
    imports.insert(
        "source-1".into(),
        vec![
            "wasi:io/streams".into(),
            "torvyn:resources/buffer-ops".into(),
        ],
    );
    imports.insert(
        "transform-1".into(),
        vec![
            "wasi:io/streams".into(),
            "torvyn:resources/buffer-ops".into(),
        ],
    );
    imports.insert("sink-1".into(), vec!["wasi:io/streams".into()]);

    let mut exports = HashMap::new();
    exports.insert("source-1".into(), vec![]);
    exports.insert("transform-1".into(), vec![]);
    exports.insert("sink-1".into(), vec![]);

    let mut linker = PipelineLinker::new();
    let result = linker.link(&topo, &imports, &exports);
    assert!(
        result.is_ok(),
        "Full pipeline link should succeed: {:?}",
        result.err()
    );

    let linked = result.unwrap();
    assert_eq!(linked.name, "full-test");
    assert_eq!(linked.component_count(), 3);
    assert_eq!(linked.connection_count(), 2);

    // Verify topological order
    let order = &linked.topological_order;
    let src_pos = order.iter().position(|n| n == "source-1").unwrap();
    let proc_pos = order.iter().position(|n| n == "transform-1").unwrap();
    let snk_pos = order.iter().position(|n| n == "sink-1").unwrap();
    assert!(src_pos < proc_pos, "source must come before processor");
    assert!(proc_pos < snk_pos, "processor must come before sink");
}

#[test]
fn integration_diamond_topology() {
    // src → proc1 → snk
    // src → proc2 → snk
    let mut topo = PipelineTopology::new("diamond".into());
    topo.add_node(source_node("src"));
    topo.add_node(processor_node("proc1"));
    topo.add_node(processor_node("proc2"));
    topo.add_node(sink_node("snk"));
    topo.add_edge(edge("src", "proc1"));
    topo.add_edge(edge("src", "proc2"));
    topo.add_edge(TopologyEdge {
        from_node: "proc1".into(),
        from_port: "output".into(),
        to_node: "snk".into(),
        to_port: "input1".into(),
        queue_depth: 64,
        backpressure_policy: Default::default(),
    });
    topo.add_edge(TopologyEdge {
        from_node: "proc2".into(),
        from_port: "output".into(),
        to_node: "snk".into(),
        to_port: "input2".into(),
        queue_depth: 64,
        backpressure_policy: Default::default(),
    });

    let mut linker = PipelineLinker::new();
    let result = linker.link_topology_only(&topo);
    assert!(
        result.is_ok(),
        "Diamond topology should link: {:?}",
        result.err()
    );

    let linked = result.unwrap();
    assert_eq!(linked.component_count(), 4);
    assert_eq!(linked.connection_count(), 4);

    // snk must come after both proc1 and proc2
    let order = &linked.topological_order;
    let snk_pos = order.iter().position(|n| n == "snk").unwrap();
    let proc1_pos = order.iter().position(|n| n == "proc1").unwrap();
    let proc2_pos = order.iter().position(|n| n == "proc2").unwrap();
    assert!(proc1_pos < snk_pos);
    assert!(proc2_pos < snk_pos);
}

#[test]
fn integration_error_report_is_complete() {
    let mut topo = PipelineTopology::new("bad-pipeline".into());
    topo.add_node(TopologyNode {
        name: "src".into(),
        role: ComponentRole::Source,
        artifact_path: "src.wasm".into(),
        config: None,
        capability_grants: vec![],
    });
    topo.add_node(sink_node("snk"));
    topo.add_edge(edge("src", "snk"));

    // src imports a capability-gated interface without a grant
    // AND an unknown interface
    let mut imports = HashMap::new();
    imports.insert(
        "src".into(),
        vec![
            "wasi:filesystem/preopens".into(),
            "totally:unknown/interface".into(),
        ],
    );
    let exports = HashMap::new();

    let mut linker = PipelineLinker::new();
    let result = linker.link(&topo, &imports, &exports);
    assert!(result.is_err());

    if let Err(LinkerError::LinkFailed(report)) = result {
        // Must report ALL errors, not just first
        let has_unresolved = report
            .errors
            .iter()
            .any(|e| e.category == LinkDiagnosticCategory::UnresolvedImport);
        let has_capability = report
            .errors
            .iter()
            .any(|e| e.category == LinkDiagnosticCategory::CapabilityDenied);
        assert!(
            has_unresolved,
            "should report unresolved import: {}",
            report.format_all()
        );
        assert!(
            has_capability,
            "should report capability denied: {}",
            report.format_all()
        );
    }
}

#[test]
fn integration_linked_pipeline_connections_have_correct_metadata() {
    let mut topo = PipelineTopology::new("meta-test".into());
    topo.add_node(source_node("src"));
    topo.add_node(sink_node("snk"));
    topo.add_edge(TopologyEdge {
        from_node: "src".into(),
        from_port: "my-output".into(),
        to_node: "snk".into(),
        to_port: "my-input".into(),
        queue_depth: 128,
        backpressure_policy: BackpressurePolicy::DropOldest,
    });

    let mut linker = PipelineLinker::new();
    let linked = linker.link_topology_only(&topo).unwrap();

    let conn = &linked.connections[0];
    assert_eq!(conn.from_component, "src");
    assert_eq!(conn.from_port, "my-output");
    assert_eq!(conn.to_component, "snk");
    assert_eq!(conn.to_port, "my-input");
    assert_eq!(conn.queue_depth, 128);
    assert_eq!(conn.backpressure_policy, BackpressurePolicy::DropOldest);
}

#[test]
fn integration_component_config_propagated() {
    let mut topo = PipelineTopology::new("config-test".into());
    topo.add_node(TopologyNode {
        name: "src".into(),
        role: ComponentRole::Source,
        artifact_path: "src.wasm".into(),
        config: Some("{\"path\":\"/data/input.csv\"}".into()),
        capability_grants: vec![],
    });
    topo.add_node(sink_node("snk"));
    topo.add_edge(edge("src", "snk"));

    let mut linker = PipelineLinker::new();
    let linked = linker.link_topology_only(&topo).unwrap();

    let src = linked.get_component("src").unwrap();
    assert_eq!(
        src.config.as_deref(),
        Some("{\"path\":\"/data/input.csv\"}")
    );
}
