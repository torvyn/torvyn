//! Integration tests for EventSink trait implementation.

use torvyn_observability::collector::ObservabilityCollector;
use torvyn_observability::config::ObservabilityConfig;
use torvyn_types::{
    ComponentId, CopyReason, EventSink, FlowId, InvocationStatus, ObservabilityLevel,
    ProcessErrorKind, ResourceId, StreamId,
};

#[test]
fn test_event_sink_noop_from_types() {
    use torvyn_types::NoopEventSink;
    let sink = NoopEventSink;
    sink.record_invocation(
        FlowId::new(1),
        ComponentId::new(1),
        0,
        100,
        InvocationStatus::Ok,
    );
    sink.record_element_transfer(FlowId::new(1), StreamId::new(1), 0, 5);
    sink.record_backpressure(FlowId::new(1), StreamId::new(1), true, 64, 1000);
    sink.record_copy(
        FlowId::new(1),
        ResourceId::new(0, 0),
        ComponentId::new(1),
        ComponentId::new(2),
        1024,
        CopyReason::CrossComponent,
    );
    assert_eq!(sink.level(), ObservabilityLevel::Off);
}

#[test]
fn test_event_sink_collector_records_to_metrics() {
    let collector = ObservabilityCollector::new_for_testing(ObservabilityConfig::default());

    let _obs = collector
        .register_flow(
            FlowId::new(1),
            &[ComponentId::new(10), ComponentId::new(20)],
            &[StreamId::new(1)],
        )
        .unwrap();

    // Record invocations.
    for _ in 0..100 {
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(10),
            0,
            5000,
            InvocationStatus::Ok,
        );
    }

    // Record copies.
    for _ in 0..50 {
        collector.record_copy(
            FlowId::new(1),
            ResourceId::new(0, 0),
            ComponentId::new(10),
            ComponentId::new(20),
            256,
            CopyReason::CrossComponent,
        );
    }

    let snap = collector.snapshot(FlowId::new(1)).unwrap();
    assert_eq!(snap.elements_total, 100);
    assert_eq!(snap.copies_total, 50);
    assert_eq!(snap.copy_bytes_total, 50 * 256);
}

#[test]
fn test_event_sink_100k_invocations() {
    let collector = ObservabilityCollector::new_for_testing(ObservabilityConfig::default());

    let _obs = collector
        .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
        .unwrap();

    for i in 0..100_000u64 {
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            i * 1000,
            i * 1000 + 500,
            InvocationStatus::Ok,
        );
    }

    let snap = collector.snapshot(FlowId::new(1)).unwrap();
    assert_eq!(snap.elements_total, 100_000);
}

#[test]
fn test_event_sink_off_level_zero_overhead() {
    let config = ObservabilityConfig {
        level: ObservabilityLevel::Off,
        ..ObservabilityConfig::default()
    };
    let collector = ObservabilityCollector::new_for_testing(config);

    let _obs = collector
        .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
        .unwrap();

    for _ in 0..100_000 {
        collector.record_invocation(
            FlowId::new(1),
            ComponentId::new(1),
            0,
            100,
            InvocationStatus::Ok,
        );
    }

    let snap = collector.snapshot(FlowId::new(1)).unwrap();
    assert_eq!(
        snap.elements_total, 0,
        "Off level should not record metrics"
    );
}

#[test]
fn test_event_sink_error_counting() {
    let collector = ObservabilityCollector::new_for_testing(ObservabilityConfig::default());

    let _obs = collector
        .register_flow(FlowId::new(1), &[ComponentId::new(1)], &[StreamId::new(1)])
        .unwrap();

    collector.record_invocation(
        FlowId::new(1),
        ComponentId::new(1),
        0,
        100,
        InvocationStatus::Ok,
    );
    collector.record_invocation(
        FlowId::new(1),
        ComponentId::new(1),
        100,
        200,
        InvocationStatus::Error(ProcessErrorKind::Internal),
    );
    collector.record_invocation(
        FlowId::new(1),
        ComponentId::new(1),
        200,
        300,
        InvocationStatus::Timeout,
    );

    let snap = collector.snapshot(FlowId::new(1)).unwrap();
    assert_eq!(snap.elements_total, 3);
    assert_eq!(snap.errors_total, 2); // Error + Timeout
}
