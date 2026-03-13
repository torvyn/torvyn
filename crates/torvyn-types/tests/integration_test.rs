//! Integration tests verifying cross-module consistency of torvyn-types.

use torvyn_types::*;

#[test]
fn test_buffer_handle_wraps_resource_id_correctly() {
    let rid = ResourceId::new(42, 7);
    let handle = BufferHandle::new(rid);
    assert_eq!(handle.resource_id().index(), 42);
    assert_eq!(handle.resource_id().generation(), 7);
}

#[test]
fn test_process_error_converts_to_torvyn_error() {
    let process_err = ProcessError::Fatal("disk full".into());
    let torvyn_err: TorvynError = process_err.into();
    let display = format!("{torvyn_err}");
    assert!(display.contains("FATAL"));
    assert!(display.contains("disk full"));
}

#[test]
fn test_all_error_types_convert_to_torvyn_error() {
    let _: TorvynError = ProcessError::DeadlineExceeded.into();
    let _: TorvynError = ContractError::PackageNotFound { package_name: "x".into() }.into();
    let _: TorvynError = LinkError::CyclicDependency { cycle: vec![] }.into();
    let _: TorvynError = ResourceError::StaleHandle { handle: ResourceId::new(0, 0) }.into();
    let _: TorvynError = ReactorError::ShuttingDown.into();
    let _: TorvynError = ConfigError::FileNotFound { path: "x".into() }.into();
    let _: TorvynError = SecurityError::CapabilityDenied {
        component: ComponentId::new(0),
        capability: "x".into(),
    }.into();
    let _: TorvynError = PackagingError::InvalidArtifact {
        path: "x".into(),
        reason: "y".into(),
    }.into();
}

#[test]
fn test_flow_state_full_happy_path() {
    let state = FlowState::Created;
    let state = state.transition_to(FlowState::Validated).unwrap();
    let state = state.transition_to(FlowState::Instantiated).unwrap();
    let state = state.transition_to(FlowState::Running).unwrap();
    let state = state.transition_to(FlowState::Draining).unwrap();
    let state = state.transition_to(FlowState::Completed).unwrap();
    assert!(state.is_terminal());
}

#[test]
fn test_flow_state_error_path() {
    let state = FlowState::Created;
    let state = state.transition_to(FlowState::Failed).unwrap();
    assert!(state.is_terminal());
}

#[test]
fn test_resource_state_full_lifecycle() {
    let state = ResourceState::Pooled;
    let state = state.transition_to(ResourceState::Owned).unwrap();
    let state = state.transition_to(ResourceState::Borrowed).unwrap();
    let state = state.transition_to(ResourceState::Owned).unwrap();
    let state = state.transition_to(ResourceState::Pooled).unwrap();
    assert!(state.is_available());
}

#[test]
fn test_resource_state_transit_lifecycle() {
    let state = ResourceState::Owned;
    let state = state.transition_to(ResourceState::Transit).unwrap();
    let state = state.transition_to(ResourceState::Owned).unwrap();
    assert_eq!(state, ResourceState::Owned);
}

#[test]
fn test_element_meta_with_current_timestamp() {
    let ts = current_timestamp_ns();
    let meta = ElementMeta::new(0, ts, "application/json".into());
    assert!(meta.timestamp_ns > 0);
}

#[test]
fn test_trace_context_roundtrip() {
    let ctx = TraceContext::new(
        TraceId::new([0xab; 16]),
        SpanId::new([0xcd; 8]),
    );
    assert!(ctx.is_valid());
    let display = format!("{ctx}");
    assert!(display.contains("abababab"));
}

#[test]
fn test_noop_event_sink_satisfies_trait() {
    fn use_sink(sink: &dyn EventSink) {
        assert_eq!(sink.level(), ObservabilityLevel::Off);
    }
    use_sink(&NoopEventSink);
}

#[test]
fn test_component_id_is_alias_for_component_instance_id() {
    let instance_id = ComponentInstanceId::new(99);
    let component_id: ComponentId = instance_id;
    assert_eq!(instance_id, component_id);
    assert_eq!(component_id.as_u64(), 99);
}

#[test]
fn test_identity_types_are_copy() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<ComponentTypeId>();
    assert_copy::<ComponentInstanceId>();
    assert_copy::<FlowId>();
    assert_copy::<StreamId>();
    assert_copy::<ResourceId>();
    assert_copy::<BufferHandle>();
    assert_copy::<TraceId>();
    assert_copy::<SpanId>();
}

#[test]
fn test_identity_types_are_hash() {
    fn assert_hash<T: std::hash::Hash>() {}
    assert_hash::<ComponentTypeId>();
    assert_hash::<ComponentInstanceId>();
    assert_hash::<FlowId>();
    assert_hash::<StreamId>();
    assert_hash::<ResourceId>();
    assert_hash::<BufferHandle>();
    assert_hash::<TraceId>();
    assert_hash::<SpanId>();
}

#[test]
fn test_enum_types_are_copy() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<ComponentRole>();
    assert_copy::<BackpressureSignal>();
    assert_copy::<BackpressurePolicy>();
    assert_copy::<ObservabilityLevel>();
    assert_copy::<Severity>();
    assert_copy::<CopyReason>();
    assert_copy::<FlowState>();
    assert_copy::<ResourceState>();
}
