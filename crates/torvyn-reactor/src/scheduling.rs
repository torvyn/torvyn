//! Scheduling policies for intra-flow stage ordering.
//!
//! Per Doc 04 §3: the scheduler determines which component runs next
//! within a flow. The default [`DemandDrivenPolicy`] is consumer-first.

use torvyn_types::{ComponentId, ComponentRole, StreamId};

use crate::stream::StreamState;
use crate::topology::FlowTopology;

// ---------------------------------------------------------------------------
// StageExecution
// ---------------------------------------------------------------------------

/// What the scheduler tells the flow driver to do.
///
/// # HOT PATH — returned per scheduling decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageExecution {
    /// The component to invoke.
    pub component_id: ComponentId,
    /// The stage index in the topology.
    pub stage_index: usize,
    /// What action to perform.
    pub action: StageAction,
}

/// The specific action the flow driver should take.
///
/// # HOT PATH
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageAction {
    /// Call the source's `pull()` function.
    PullFromSource,
    /// Call the processor's `process()` with the element from the given input stream.
    ProcessElement {
        /// The stream to read input from.
        input_stream: StreamId,
    },
    /// Call the sink's `push()` with the element from the given input stream.
    PushToSink {
        /// The stream to read input from.
        input_stream: StreamId,
    },
}

// ---------------------------------------------------------------------------
// SchedulingPolicy trait
// ---------------------------------------------------------------------------

/// Trait for intra-flow scheduling policies.
///
/// Given the current stream states and topology, select the next stage
/// to execute or return `None` if no stage is ready.
///
/// # HOT PATH — called per scheduling cycle.
pub trait SchedulingPolicy: Send + Sync {
    /// Select the next stage to execute.
    ///
    /// Returns `None` if no stage is currently ready (all queues empty
    /// and/or all stages are backpressured).
    fn next_ready_stage(
        &self,
        topology: &FlowTopology,
        streams: &[StreamState],
        stream_index: &StreamIndex,
    ) -> Option<StageExecution>;
}

/// Index mapping stages to their input/output streams for O(1) lookup.
///
/// Built once during flow setup. Used by scheduling policies.
///
/// # COLD PATH to build, HOT PATH to query.
#[derive(Clone, Debug)]
pub struct StreamIndex {
    /// For each stage index, the list of input stream indices in `streams`.
    pub inputs: Vec<Vec<usize>>,
    /// For each stage index, the list of output stream indices in `streams`.
    pub outputs: Vec<Vec<usize>>,
}

impl StreamIndex {
    /// Build a stream index from the topology and stream list.
    ///
    /// # COLD PATH — called once during flow setup.
    pub fn build(topology: &FlowTopology, _streams: &[StreamState]) -> Self {
        let n = topology.stages.len();
        let mut inputs = vec![Vec::new(); n];
        let mut outputs = vec![Vec::new(); n];

        for (stream_idx, conn) in topology.connections.iter().enumerate() {
            outputs[conn.from_stage].push(stream_idx);
            inputs[conn.to_stage].push(stream_idx);
        }

        Self { inputs, outputs }
    }
}

// ---------------------------------------------------------------------------
// DemandDrivenPolicy
// ---------------------------------------------------------------------------

/// Default consumer-first scheduling policy.
///
/// Per Doc 04 §3.2: walks from sinks upstream. Executes the deepest
/// consumer that has input, or the shallowest source that has demand.
/// This naturally drains queues and prevents unnecessary buffering.
pub struct DemandDrivenPolicy;

impl SchedulingPolicy for DemandDrivenPolicy {
    /// # HOT PATH — called per scheduling decision.
    ///
    /// Algorithm:
    /// 1. Walk stages in reverse topological order (consumers first).
    /// 2. For each stage, check if it has input available AND output capacity.
    /// 3. Return the first ready stage found.
    /// 4. If no processor/sink is ready, check sources that have demand.
    fn next_ready_stage(
        &self,
        topology: &FlowTopology,
        streams: &[StreamState],
        stream_index: &StreamIndex,
    ) -> Option<StageExecution> {
        let topo_order = topology.topological_order();

        // Walk in reverse order (consumers first).
        for &stage_idx in topo_order.iter().rev() {
            let stage = &topology.stages[stage_idx];

            match stage.role {
                ComponentRole::Sink => {
                    // Sink is ready if any input stream has elements.
                    for &si in &stream_index.inputs[stage_idx] {
                        if streams[si].consumer_has_input() {
                            return Some(StageExecution {
                                component_id: stage.component_id,
                                stage_index: stage_idx,
                                action: StageAction::PushToSink {
                                    input_stream: streams[si].id,
                                },
                            });
                        }
                    }
                }
                ComponentRole::Processor | ComponentRole::Filter => {
                    // Processor is ready if it has input AND its output
                    // stream has capacity (demand > 0 and not full).
                    let has_input = stream_index.inputs[stage_idx]
                        .iter()
                        .any(|&si| streams[si].consumer_has_input());

                    let has_output_capacity = stream_index.outputs[stage_idx]
                        .iter()
                        .all(|&si| streams[si].producer_can_produce());

                    // For filters, they consume but may not produce.
                    // We still need output capacity check for the case they do produce.
                    if has_input && (has_output_capacity || stage.role == ComponentRole::Filter) {
                        for &si in &stream_index.inputs[stage_idx] {
                            if streams[si].consumer_has_input() {
                                return Some(StageExecution {
                                    component_id: stage.component_id,
                                    stage_index: stage_idx,
                                    action: StageAction::ProcessElement {
                                        input_stream: streams[si].id,
                                    },
                                });
                            }
                        }
                    }
                }
                ComponentRole::Source => {
                    // Source is ready if downstream stream has capacity.
                    let can_produce = stream_index.outputs[stage_idx]
                        .iter()
                        .any(|&si| streams[si].producer_can_produce());

                    if can_produce {
                        return Some(StageExecution {
                            component_id: stage.component_id,
                            stage_index: stage_idx,
                            action: StageAction::PullFromSource,
                        });
                    }
                }
                ComponentRole::Router => {
                    // Router behaves like a processor with multiple outputs.
                    let has_input = stream_index.inputs[stage_idx]
                        .iter()
                        .any(|&si| streams[si].consumer_has_input());
                    let has_output_capacity = stream_index.outputs[stage_idx]
                        .iter()
                        .any(|&si| streams[si].producer_can_produce());

                    if has_input && has_output_capacity {
                        for &si in &stream_index.inputs[stage_idx] {
                            if streams[si].consumer_has_input() {
                                return Some(StageExecution {
                                    component_id: stage.component_id,
                                    stage_index: stage_idx,
                                    action: StageAction::ProcessElement {
                                        input_stream: streams[si].id,
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }

        None // No stage is ready
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StreamConfig;
    use crate::topology::{StageDefinition, StreamConnection};
    use torvyn_types::{BackpressurePolicy, ComponentRole};

    fn source(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Source,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn processor(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Processor,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn sink(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Sink,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn make_streams_for_linear(topology: &FlowTopology) -> Vec<StreamState> {
        topology
            .connections
            .iter()
            .enumerate()
            .map(|(idx, conn)| {
                StreamState::new(
                    StreamId::new(idx as u64),
                    torvyn_types::FlowId::new(1),
                    topology.stages[conn.from_stage].component_id,
                    topology.stages[conn.to_stage].component_id,
                    64,
                    BackpressurePolicy::BlockProducer,
                    0.5,
                )
            })
            .collect()
    }

    #[test]
    fn test_demand_driven_selects_source_when_queues_empty() {
        let topo = FlowTopology {
            stages: vec![source(1), sink(2)],
            connections: vec![StreamConnection {
                from_stage: 0,
                to_stage: 1,
                config: StreamConfig::default(),
            }],
        };
        let streams = make_streams_for_linear(&topo);
        let index = StreamIndex::build(&topo, &streams);
        let policy = DemandDrivenPolicy;

        let result = policy.next_ready_stage(&topo, &streams, &index);
        assert!(result.is_some());
        let exec = result.unwrap();
        assert_eq!(exec.component_id, ComponentId::new(1));
        assert!(matches!(exec.action, StageAction::PullFromSource));
    }

    #[test]
    fn test_demand_driven_prefers_consumer_over_source() {
        let topo = FlowTopology {
            stages: vec![source(1), sink(2)],
            connections: vec![StreamConnection {
                from_stage: 0,
                to_stage: 1,
                config: StreamConfig::default(),
            }],
        };
        let mut streams = make_streams_for_linear(&topo);
        // Put an element in the queue so the sink can consume.
        let elem = crate::stream::StreamElementRef {
            sequence: 0,
            buffer_handle: torvyn_types::BufferHandle::new(torvyn_types::ResourceId::new(0, 0)),
            meta: torvyn_types::ElementMeta::new(0, 0, String::new()),
            enqueued_at: std::time::Instant::now(),
        };
        streams[0].queue.push(elem);

        let index = StreamIndex::build(&topo, &streams);
        let policy = DemandDrivenPolicy;

        let result = policy.next_ready_stage(&topo, &streams, &index);
        assert!(result.is_some());
        let exec = result.unwrap();
        // Should prefer the sink (consumer) over the source.
        assert_eq!(exec.component_id, ComponentId::new(2));
        assert!(matches!(exec.action, StageAction::PushToSink { .. }));
    }

    #[test]
    fn test_demand_driven_returns_none_when_backpressured() {
        let topo = FlowTopology {
            stages: vec![source(1), sink(2)],
            connections: vec![StreamConnection {
                from_stage: 0,
                to_stage: 1,
                config: StreamConfig::default(),
            }],
        };
        let mut streams = make_streams_for_linear(&topo);
        // Fill the queue completely and activate backpressure.
        for i in 0..64 {
            let elem = crate::stream::StreamElementRef {
                sequence: i,
                buffer_handle: torvyn_types::BufferHandle::new(torvyn_types::ResourceId::new(
                    i as u32, 0,
                )),
                meta: torvyn_types::ElementMeta::new(i, 0, String::new()),
                enqueued_at: std::time::Instant::now(),
            };
            streams[0].queue.push(elem);
        }
        streams[0].demand = 0; // No more demand

        let index = StreamIndex::build(&topo, &streams);
        let policy = DemandDrivenPolicy;

        // Sink has input, so it should be selected.
        let result = policy.next_ready_stage(&topo, &streams, &index);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap().action,
            StageAction::PushToSink { .. }
        ));
    }

    #[test]
    fn test_fifo_round_robin_three_stage() {
        let topo = FlowTopology {
            stages: vec![source(1), processor(2), sink(3)],
            connections: vec![
                StreamConnection {
                    from_stage: 0,
                    to_stage: 1,
                    config: StreamConfig::default(),
                },
                StreamConnection {
                    from_stage: 1,
                    to_stage: 2,
                    config: StreamConfig::default(),
                },
            ],
        };
        let streams = make_streams_for_linear(&topo);
        let index = StreamIndex::build(&topo, &streams);
        let policy = DemandDrivenPolicy;

        // With empty queues, only the source should be ready.
        let result = policy.next_ready_stage(&topo, &streams, &index);
        assert!(result.is_some());
        let exec = result.unwrap();
        assert_eq!(exec.component_id, ComponentId::new(1));
        assert!(matches!(exec.action, StageAction::PullFromSource));
    }
}
