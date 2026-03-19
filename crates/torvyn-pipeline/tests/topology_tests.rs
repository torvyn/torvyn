//! Integration tests for topology construction and validation end-to-end.

use torvyn_pipeline::*;
use torvyn_types::ComponentRole;

#[test]
fn test_three_stage_linear_pipeline_end_to_end() {
    let topo = PipelineTopologyBuilder::new("linear-3")
        .description("Source → Processor → Sink")
        .add_node(
            "source",
            ComponentRole::Source,
            "file://source.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "transform",
            ComponentRole::Processor,
            "file://transform.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "sink",
            ComponentRole::Sink,
            "file://sink.wasm",
            NodeConfig::default(),
        )
        .add_edge("source", "output", "transform", "input")
        .add_edge("transform", "output", "sink", "input")
        .build()
        .expect("valid topology");

    assert_eq!(topo.node_count(), 3);
    assert_eq!(topo.edge_count(), 2);
    assert_eq!(topo.name(), "linear-3");

    // Execution order: source must come before transform, transform before sink
    let order = topo.execution_order();
    assert_eq!(order.len(), 3);
    let src_pos = order
        .iter()
        .position(|&i| i == topo.node_index_by_name("source").unwrap())
        .unwrap();
    let xfm_pos = order
        .iter()
        .position(|&i| i == topo.node_index_by_name("transform").unwrap())
        .unwrap();
    let snk_pos = order
        .iter()
        .position(|&i| i == topo.node_index_by_name("sink").unwrap())
        .unwrap();
    assert!(src_pos < xfm_pos);
    assert!(xfm_pos < snk_pos);
}

#[test]
fn test_five_stage_diamond_pipeline() {
    //     ┌─ proc1 ─┐
    // src ─┤         ├─ sink
    //     └─ proc2 ─┘
    let topo = PipelineTopologyBuilder::new("diamond")
        .add_node(
            "src",
            ComponentRole::Source,
            "file://s.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "proc1",
            ComponentRole::Processor,
            "file://p1.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "proc2",
            ComponentRole::Processor,
            "file://p2.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "sink",
            ComponentRole::Sink,
            "file://k.wasm",
            NodeConfig::default(),
        )
        .add_edge("src", "out", "proc1", "in")
        .add_edge("src", "out", "proc2", "in")
        .add_edge("proc1", "out", "sink", "in")
        .add_edge("proc2", "out", "sink", "in")
        .build()
        .expect("valid diamond topology");

    assert_eq!(topo.node_count(), 4);
    assert_eq!(topo.edge_count(), 4);

    let order = topo.execution_order();
    // Source must be first, sink must be last
    assert_eq!(order[0], topo.node_index_by_name("src").unwrap());
    assert_eq!(
        *order.last().unwrap(),
        topo.node_index_by_name("sink").unwrap()
    );
}

#[test]
fn test_multi_source_multi_sink() {
    let topo = PipelineTopologyBuilder::new("multi")
        .add_node(
            "s1",
            ComponentRole::Source,
            "file://s1.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "s2",
            ComponentRole::Source,
            "file://s2.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "proc",
            ComponentRole::Processor,
            "file://p.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "k1",
            ComponentRole::Sink,
            "file://k1.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "k2",
            ComponentRole::Sink,
            "file://k2.wasm",
            NodeConfig::default(),
        )
        .add_edge("s1", "out", "proc", "in1")
        .add_edge("s2", "out", "proc", "in2")
        .add_edge("proc", "out1", "k1", "in")
        .add_edge("proc", "out2", "k2", "in")
        .build()
        .expect("valid multi-source multi-sink");

    assert_eq!(topo.source_nodes().len(), 2);
    assert_eq!(topo.sink_nodes().len(), 2);
}

#[test]
fn test_cycle_detection_three_node_cycle() {
    // a → b → c → a (cycle)
    let result = PipelineTopologyBuilder::new("cycle3")
        .add_node(
            "s",
            ComponentRole::Source,
            "file://s.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "a",
            ComponentRole::Processor,
            "file://a.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "b",
            ComponentRole::Processor,
            "file://b.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "c",
            ComponentRole::Processor,
            "file://c.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "k",
            ComponentRole::Sink,
            "file://k.wasm",
            NodeConfig::default(),
        )
        .add_edge("s", "out", "a", "in")
        .add_edge("a", "out", "b", "in")
        .add_edge("b", "out", "c", "in")
        .add_edge("c", "out", "a", "in2") // cycle
        .add_edge("c", "out2", "k", "in")
        .build();

    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| matches!(e, PipelineError::CycleDetected { .. })));
}

#[test]
fn test_validation_collects_multiple_errors() {
    // Multiple issues: no source, no sink
    let result = PipelineTopologyBuilder::new("bad")
        .add_node(
            "p1",
            ComponentRole::Processor,
            "file://p.wasm",
            NodeConfig::default(),
        )
        .add_node(
            "p2",
            ComponentRole::Processor,
            "file://p.wasm",
            NodeConfig::default(),
        )
        .add_edge("p1", "out", "p2", "in")
        .build();

    assert!(result.is_err());
    let errors = result.unwrap_err();
    // Should have at least: NoSourceNodes and NoSinkNodes
    assert!(
        errors.len() >= 2,
        "expected multiple errors, got {}",
        errors.len()
    );
}
