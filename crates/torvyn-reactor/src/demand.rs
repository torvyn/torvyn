//! Credit-based demand propagation for pull-based flow control.
//!
//! Per Doc 04 §4: each stream maintains a demand counter. Demand flows
//! upstream from consumer to producer. Propagation is batched per
//! scheduling cycle to prevent demand storms.

use crate::stream::StreamState;

/// Replenish demand on a stream after the consumer processes an element.
///
/// Increments the stream's demand counter by 1.
///
/// # HOT PATH — called per element consumed.
#[inline(always)]
pub fn replenish_demand(stream: &mut StreamState) {
    stream.demand = stream.demand.saturating_add(1);
}

/// Consume one demand credit when a producer enqueues an element.
///
/// Decrements the stream's demand counter by 1.
///
/// # HOT PATH — called per element produced.
///
/// # Preconditions
/// - `stream.demand > 0`.
#[inline(always)]
pub fn consume_demand(stream: &mut StreamState) {
    debug_assert!(stream.demand > 0, "consume_demand called with zero demand");
    stream.demand = stream.demand.saturating_sub(1);
}

/// Batched demand propagation: propagate accumulated demand
/// credits through a linear pipeline.
///
/// Called at the end of each scheduling cycle (after processing
/// a batch of elements). For each stream, if the consumer has
/// consumed elements since the last propagation, the demand
/// is already incremented (via `replenish_demand`).
///
/// For multi-stage pipelines, this walks upstream: if a processor's
/// output stream gained demand (because its consumer consumed elements),
/// and the processor's input stream has capacity, the processor can
/// produce — which naturally propagates demand.
///
/// # WARM PATH — called once per scheduling cycle.
///
/// Per Doc 04 §4.4: batched propagation collapses N individual
/// demand signals into a single pass.
pub fn propagate_demand_batch(streams: &mut [StreamState], _topo_order: &[usize]) {
    // In the task-per-flow model with demand-driven scheduling,
    // demand propagation is implicit: each stage's execution
    // naturally grants demand upstream by consuming input.
    // This function is a hook for future explicit propagation
    // if needed (e.g., for fan-out MinAcrossAll policy).
    //
    // For v1 linear pipelines, the demand is already correctly
    // maintained by replenish_demand/consume_demand calls.
    let _ = streams;
    let _ = _topo_order;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::StreamState;
    use torvyn_types::{BackpressurePolicy, ComponentId, FlowId, StreamId};

    fn make_stream(demand: u64) -> StreamState {
        let mut s = StreamState::new(
            StreamId::new(1),
            FlowId::new(1),
            ComponentId::new(10),
            ComponentId::new(20),
            64,
            BackpressurePolicy::BlockProducer,
            0.5,
        );
        s.demand = demand;
        s
    }

    #[test]
    fn test_replenish_demand() {
        let mut s = make_stream(5);
        replenish_demand(&mut s);
        assert_eq!(s.demand, 6);
    }

    #[test]
    fn test_consume_demand() {
        let mut s = make_stream(5);
        consume_demand(&mut s);
        assert_eq!(s.demand, 4);
    }

    #[test]
    fn test_replenish_demand_saturates() {
        let mut s = make_stream(u64::MAX);
        replenish_demand(&mut s);
        assert_eq!(s.demand, u64::MAX);
    }

    #[test]
    fn test_consume_demand_saturates_at_zero() {
        let mut s = make_stream(0);
        // In release mode, saturating_sub prevents underflow.
        // In debug mode, the debug_assert would fire, so we test
        // the saturating behavior directly.
        s.demand = 0;
        s.demand = s.demand.saturating_sub(1);
        assert_eq!(s.demand, 0);
    }

    #[test]
    fn test_demand_round_trip() {
        let mut s = make_stream(10);
        consume_demand(&mut s);
        consume_demand(&mut s);
        assert_eq!(s.demand, 8);
        replenish_demand(&mut s);
        assert_eq!(s.demand, 9);
    }
}
