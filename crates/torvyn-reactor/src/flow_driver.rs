//! Flow driver: the hot-path task for a single flow.
//!
//! Per Doc 04 §1.3: one Tokio task per flow. The flow driver owns all
//! stream state and runs the scheduling loop.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use torvyn_types::{
    ComponentId, ElementMeta, EventSink, FlowId, FlowState, InvocationStatus, ProcessError,
    StreamId,
};

use torvyn_engine::{ComponentInstance, ComponentInvoker, ProcessResult, StreamElement};

use crate::backpressure::check_backpressure_transition;
use crate::cancellation::{CancellationReason, FlowCancellation};
use crate::config::{FlowConfig, TimeoutConfig};
use crate::demand::{consume_demand, replenish_demand};
use crate::error::{ErrorPolicy, FlowError};
use crate::events::ReactorEvent;
use crate::fairness::YieldController;
use crate::metrics::{ComponentMetrics, FlowCompletionStats};
use crate::queue::PushResult;
use crate::scheduling::{
    DemandDrivenPolicy, SchedulingPolicy, StageAction, StageExecution, StreamIndex,
};
use crate::stream::{StreamElementRef, StreamState};
use crate::topology::FlowTopology;

// ---------------------------------------------------------------------------
// FlowDriverHandle
// ---------------------------------------------------------------------------

/// Handle for the coordinator to communicate with a running flow driver.
///
/// Kept by the coordinator; dropped to signal the flow driver to stop.
#[derive(Debug)]
pub struct FlowDriverHandle {
    /// The flow's unique identifier.
    pub flow_id: FlowId,
    /// Cancellation token for this flow.
    pub cancellation: FlowCancellation,
    /// Current flow state (updated by coordinator on events).
    pub state: FlowState,
}

// ---------------------------------------------------------------------------
// FlowDriver
// ---------------------------------------------------------------------------

/// The core execution unit: runs one flow as a Tokio task.
///
/// Owns the pipeline topology, stream states, component instances,
/// and metrics for a single flow. Invokes components via
/// `ComponentInvoker`.
///
/// # HOT PATH — the main loop processes one element per iteration.
pub struct FlowDriver<I: ComponentInvoker, E: EventSink> {
    /// Flow identifier.
    flow_id: FlowId,
    /// Pipeline topology.
    topology: FlowTopology,
    /// Stream states (one per connection in the topology).
    streams: Vec<StreamState>,
    /// Stream index for O(1) input/output lookup.
    stream_index: StreamIndex,
    /// Component instances (one per stage).
    instances: Vec<ComponentInstance>,
    /// Component metrics (one per stage).
    component_metrics: Vec<ComponentMetrics>,
    /// The scheduling policy.
    scheduler: DemandDrivenPolicy,
    /// The component invoker.
    invoker: I,
    /// Observability event sink.
    event_sink: E,
    /// Cancellation token.
    cancellation: FlowCancellation,
    /// Timeout configuration.
    timeouts: TimeoutConfig,
    /// Error policy.
    error_policy: ErrorPolicy,
    /// Yield controller.
    yield_ctrl: YieldController,
    /// Reactor event sender.
    event_tx: mpsc::Sender<ReactorEvent>,
    /// Current flow state.
    state: FlowState,
    /// Timestamp when the flow entered Running state.
    started_at: Option<Instant>,
    /// Sequence counter for element metadata assignment.
    /// Per C01-4: the reactor assigns sequence numbers.
    next_global_sequence: u64,
    /// Anchor pairing a monotonic `Instant` with the wall-clock nanosecond it
    /// was taken at, captured once when the flow starts running.
    ///
    /// Spans have to carry absolute timestamps to be exportable, but reading
    /// the wall clock per invocation would put a `SystemTime::now` on the hot
    /// path. Anchoring once and converting monotonic instants against it
    /// costs only arithmetic per invocation, and keeps the ordering
    /// guarantees of a monotonic clock — a wall clock can step backwards, and
    /// a span whose end precedes its start is worse than no span.
    epoch_anchor: Option<(Instant, u64)>,
    /// Sequence number of the element the stage currently executing is
    /// handling, if any.
    ///
    /// Set by each stage executor as soon as the element is known — before
    /// the guest is invoked, so that a failed invocation still produces a
    /// span attributed to the right element — and read by `execute_stage`
    /// once the invocation returns.
    current_element_sequence: Option<u64>,
}

impl<I: ComponentInvoker, E: EventSink> FlowDriver<I, E> {
    /// Create a new `FlowDriver`.
    ///
    /// # COLD PATH — called once per flow.
    ///
    /// # Preconditions
    /// - `topology` has been validated.
    /// - `instances` contains one instance per stage.
    /// - `streams` contains one stream per connection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        flow_id: FlowId,
        config: FlowConfig,
        instances: Vec<ComponentInstance>,
        streams: Vec<StreamState>,
        invoker: I,
        event_sink: E,
        cancellation: FlowCancellation,
        event_tx: mpsc::Sender<ReactorEvent>,
    ) -> Self {
        let stream_index = StreamIndex::build(&config.topology, &streams);
        let component_metrics = vec![ComponentMetrics::new(); config.topology.stages.len()];
        let yield_ctrl = YieldController::new(
            config.priority.elements_per_yield(),
            config.yield_config.time_quantum,
        );

        Self {
            flow_id,
            topology: config.topology,
            streams,
            stream_index,
            instances,
            component_metrics,
            scheduler: DemandDrivenPolicy,
            invoker,
            event_sink,
            cancellation,
            timeouts: config.timeouts,
            error_policy: config.error_policy,
            yield_ctrl,
            event_tx,
            state: FlowState::Instantiated,
            started_at: None,
            next_global_sequence: 0,
            epoch_anchor: None,
            current_element_sequence: None,
        }
    }

    /// Convert a monotonic instant to nanoseconds since the Unix epoch, using
    /// the anchor captured at flow start.
    ///
    /// Falls back to reading the wall clock if the anchor is missing, which
    /// can only happen if a stage runs before `run` sets it.
    ///
    /// # HOT PATH — arithmetic only.
    #[inline]
    fn epoch_ns(&self, at: Instant) -> u64 {
        match self.epoch_anchor {
            Some((anchor_instant, anchor_ns)) => anchor_ns
                .saturating_add(at.saturating_duration_since(anchor_instant).as_nanos() as u64),
            None => torvyn_types::current_timestamp_ns(),
        }
    }

    /// Run the flow driver main loop.
    ///
    /// This method is spawned as a Tokio task by the coordinator.
    /// It returns when the flow completes, is cancelled, or fails.
    ///
    /// # HOT PATH — the inner loop processes elements.
    pub async fn run(mut self) -> (FlowId, FlowState, FlowCompletionStats) {
        // Transition to Running.
        self.state = FlowState::Running;
        let start_instant = Instant::now();
        self.started_at = Some(start_instant);
        self.epoch_anchor = Some((start_instant, torvyn_types::current_timestamp_ns()));
        self.emit_state_change(FlowState::Instantiated, FlowState::Running);

        info!(flow_id = %self.flow_id, "flow started");

        // Set up optional flow deadline.
        let deadline_sleep = match self.timeouts.flow_deadline {
            Some(d) => Box::pin(sleep(d))
                as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
            None => Box::pin(std::future::pending())
                as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
        };
        tokio::pin!(deadline_sleep);

        // === Main processing loop ===
        loop {
            // 1. Check cancellation.
            if self.cancellation.is_cancelled() {
                debug!(flow_id = %self.flow_id, "cancellation detected");
                break;
            }

            // 2. Select next ready stage.
            match self
                .scheduler
                .next_ready_stage(&self.topology, &self.streams, &self.stream_index)
            {
                Some(execution) => {
                    // 3. Execute the stage.
                    let result = self.execute_stage(&execution).await;

                    // 4. Handle result.
                    if let Err(flow_err) = result {
                        match &self.error_policy {
                            ErrorPolicy::FailFast => {
                                error!(
                                    flow_id = %self.flow_id,
                                    error = %flow_err,
                                    "FailFast: flow terminating"
                                );
                                self.cancellation
                                    .cancel(CancellationReason::DownstreamError {
                                        component: execution.component_id,
                                        error: ProcessError::Internal(format!("{flow_err}")),
                                    });
                                break;
                            }
                            ErrorPolicy::SkipElement | ErrorPolicy::LogAndContinue => {
                                warn!(
                                    flow_id = %self.flow_id,
                                    error = %flow_err,
                                    policy = %self.error_policy,
                                    "element error handled by policy"
                                );
                            }
                            ErrorPolicy::Retry {
                                max_retries: _,
                                backoff: _,
                            } => {
                                // Simplified retry: just log for now.
                                // Full retry logic with backoff is Phase 1.
                                warn!(
                                    flow_id = %self.flow_id,
                                    error = %flow_err,
                                    "retry policy active (simplified for Phase 0)"
                                );
                            }
                        }
                    }

                    // 5. Check backpressure transitions for all streams.
                    self.check_backpressure_all();

                    // 6. Yield check.
                    self.yield_ctrl.record_element();
                    if self.yield_ctrl.should_yield() {
                        let elements = self.yield_ctrl.elements_since_yield();
                        self.yield_ctrl.reset();
                        let _ = self.event_tx.try_send(ReactorEvent::FlowYielded {
                            flow_id: self.flow_id,
                            elements_since_last_yield: elements,
                        });
                        tokio::task::yield_now().await;
                    }
                }
                None => {
                    // No stage is ready. Propagate source completion through
                    // the pipeline, then check if all streams are done.
                    self.propagate_producer_complete();

                    if self.all_streams_complete() {
                        info!(flow_id = %self.flow_id, "all streams complete");
                        self.cancellation.cancel(CancellationReason::SourceComplete);
                        break;
                    }

                    // Wait for something to happen.
                    tokio::select! {
                        _ = self.cancellation.cancelled() => {
                            debug!(flow_id = %self.flow_id, "cancelled while waiting");
                            break;
                        }
                        _ = &mut deadline_sleep => {
                            let elapsed = self.started_at.unwrap().elapsed();
                            error!(
                                flow_id = %self.flow_id,
                                elapsed = ?elapsed,
                                "flow deadline exceeded"
                            );
                            self.cancellation.cancel(CancellationReason::Timeout { elapsed });
                            break;
                        }
                        // In a real implementation, we'd wait on a Notify
                        // from the resource manager or external I/O.
                        // For v1, we yield briefly and re-check.
                        _ = tokio::time::sleep(Duration::from_micros(50)) => {
                            continue;
                        }
                    }
                }
            }
        }

        // === Draining phase ===
        let drain_reason = self.cancellation.reason().cloned();
        self.emit_state_change(FlowState::Running, FlowState::Draining);
        self.state = FlowState::Draining;

        // Mark all source outputs as complete to prevent further pulls during drain.
        for stage_idx in 0..self.topology.stages.len() {
            if self.stream_index.inputs[stage_idx].is_empty() {
                for &si in &self.stream_index.outputs[stage_idx] {
                    self.streams[si].producer_complete = true;
                }
            }
        }

        debug!(flow_id = %self.flow_id, "entering drain phase");
        self.drain_queues().await;

        // === Compute final stats ===
        let total_duration = self.started_at.unwrap().elapsed();
        let mut stats = FlowCompletionStats::new(total_duration);
        let stream_data: Vec<_> = self
            .streams
            .iter()
            .map(|s| (s.id, s.metrics.clone()))
            .collect();
        stats.aggregate_from_streams(&stream_data);
        for (idx, cm) in self.component_metrics.iter().enumerate() {
            stats
                .component_stats
                .insert(self.topology.stages[idx].component_id, cm.clone());
        }

        // === Determine terminal state ===
        let terminal_state = match &drain_reason {
            Some(CancellationReason::SourceComplete) => FlowState::Completed,
            Some(CancellationReason::OperatorRequest) => FlowState::Cancelled,
            Some(_) => FlowState::Failed,
            None => FlowState::Completed,
        };

        self.emit_state_change(FlowState::Draining, terminal_state);
        self.state = terminal_state;

        let _ = self.event_tx.try_send(ReactorEvent::FlowCompleted {
            flow_id: self.flow_id,
            stats: stats.clone(),
        });

        info!(
            flow_id = %self.flow_id,
            state = %terminal_state,
            duration = ?total_duration,
            total_elements = stats.total_elements,
            "flow finished"
        );

        (self.flow_id, terminal_state, stats)
    }

    /// Execute a single stage action.
    ///
    /// # HOT PATH — called per element.
    async fn execute_stage(&mut self, execution: &StageExecution) -> Result<(), FlowError> {
        let start = Instant::now();
        let stage_idx = execution.stage_index;
        let component_id = execution.component_id;
        self.current_element_sequence = None;

        let result = match &execution.action {
            StageAction::PullFromSource => self.execute_pull(stage_idx, component_id).await,
            StageAction::ProcessElement { input_stream } => {
                self.execute_process(stage_idx, component_id, *input_stream)
                    .await
            }
            StageAction::PushToSink { input_stream } => {
                self.execute_push(stage_idx, component_id, *input_stream)
                    .await
            }
        };

        let end = Instant::now();
        let duration = end.saturating_duration_since(start);
        let is_error = result.is_err();
        self.component_metrics[stage_idx].record_invocation(duration, is_error);

        let status = if is_error {
            InvocationStatus::Error(torvyn_types::ProcessErrorKind::Internal)
        } else {
            InvocationStatus::Ok
        };

        // `record_invocation` takes absolute epoch timestamps, and the sink
        // derives the invocation's duration from `end_ns - start_ns`. Both
        // instants are converted through the flow's epoch anchor, so the
        // difference is exactly `duration` and the values are meaningful as
        // points in time — which is what makes them exportable as spans.
        let start_ns = self.epoch_ns(start);
        let end_ns = self.epoch_ns(end);
        self.event_sink
            .record_invocation(self.flow_id, component_id, start_ns, end_ns, status);

        if let Some(element_sequence) = self.current_element_sequence {
            self.event_sink.record_element_span(
                self.flow_id,
                component_id,
                element_sequence,
                start_ns,
                end_ns,
                status,
            );
        }

        result
    }

    /// Execute a source pull.
    ///
    /// # HOT PATH
    async fn execute_pull(
        &mut self,
        stage_idx: usize,
        component_id: ComponentId,
    ) -> Result<(), FlowError> {
        let instance = &mut self.instances[stage_idx];
        let output = tokio::time::timeout(
            self.timeouts.component_invocation_timeout,
            self.invoker.invoke_pull(instance, component_id),
        )
        .await
        .map_err(|_| FlowError::ComponentTimeout {
            component: component_id,
            timeout: self.timeouts.component_invocation_timeout,
        })?
        .map_err(|e| FlowError::ComponentError {
            component: component_id,
            error: e,
        })?;

        match output {
            Some(element) => {
                // Enqueue into the output stream.
                let output_streams = &self.stream_index.outputs[stage_idx];
                if let Some(&si) = output_streams.first() {
                    let seq = self.next_global_sequence;
                    self.next_global_sequence += 1;
                    // The source is where an element's identity begins, so
                    // this sequence is both the stream sequence and the
                    // origin sequence every later stage carries forward.
                    self.current_element_sequence = Some(seq);
                    let elem_ref = StreamElementRef {
                        sequence: seq,
                        origin_sequence: seq,
                        buffer_handle: element.payload,
                        meta: ElementMeta::new(
                            seq,
                            torvyn_types::current_timestamp_ns(),
                            element.meta.content_type,
                        ),
                        enqueued_at: Instant::now(),
                    };
                    let stream = &mut self.streams[si];
                    consume_demand(stream);
                    let push_result = stream.queue.push(elem_ref);
                    match push_result {
                        PushResult::Ok => {
                            stream.metrics.record_enqueue(stream.queue.len() as u32);
                            self.event_sink.record_element_transfer(
                                self.flow_id,
                                stream.id,
                                seq,
                                stream.queue.len() as u32,
                            );
                        }
                        PushResult::Full(_) => {
                            // Should not happen if demand is correctly managed.
                            warn!(
                                flow_id = %self.flow_id,
                                stream = %stream.id,
                                "queue full despite demand check"
                            );
                        }
                        _ => {}
                    }
                }
            }
            None => {
                // Source complete.
                let output_streams = &self.stream_index.outputs[stage_idx];
                for &si in output_streams {
                    self.streams[si].producer_complete = true;
                }
                debug!(
                    flow_id = %self.flow_id,
                    component = %component_id,
                    "source completed"
                );
            }
        }
        Ok(())
    }

    /// Execute a processor.
    ///
    /// # HOT PATH
    async fn execute_process(
        &mut self,
        stage_idx: usize,
        component_id: ComponentId,
        input_stream_id: StreamId,
    ) -> Result<(), FlowError> {
        let instance = &mut self.instances[stage_idx];
        // Find the input stream and pop the element.
        let input_si = self
            .streams
            .iter()
            .position(|s| s.id == input_stream_id)
            .expect("input stream not found");

        let elem_ref = self.streams[input_si]
            .queue
            .pop()
            .expect("consumer_has_input was true but queue is empty");

        replenish_demand(&mut self.streams[input_si]);

        // Carry the element's pipeline-entry timestamp forward so end-to-end
        // latency is measured from the source, not re-stamped at each stage.
        let origin_timestamp_ns = elem_ref.meta.timestamp_ns;
        // Likewise its origin sequence, so every span this invocation
        // produces is attributed to the element that entered at the source.
        // Set before the invoke so a failing invocation still yields a span.
        let origin_sequence = elem_ref.origin_sequence;
        self.current_element_sequence = Some(origin_sequence);

        let stream_element = StreamElement {
            meta: elem_ref.meta,
            payload: elem_ref.buffer_handle,
        };

        let result = tokio::time::timeout(
            self.timeouts.component_invocation_timeout,
            self.invoker
                .invoke_process(instance, component_id, stream_element),
        )
        .await
        .map_err(|_| FlowError::ComponentTimeout {
            component: component_id,
            timeout: self.timeouts.component_invocation_timeout,
        })?
        .map_err(|e| FlowError::ComponentError {
            component: component_id,
            error: e,
        })?;

        // Enqueue output into downstream stream(s).
        match result {
            ProcessResult::Output(output) => {
                let output_streams = &self.stream_index.outputs[stage_idx];
                if let Some(&si) = output_streams.first() {
                    let seq = self.next_global_sequence;
                    self.next_global_sequence += 1;
                    let out_ref = StreamElementRef {
                        sequence: seq,
                        origin_sequence,
                        buffer_handle: output.payload,
                        meta: ElementMeta::new(seq, origin_timestamp_ns, output.meta.content_type),
                        enqueued_at: Instant::now(),
                    };
                    let stream = &mut self.streams[si];
                    consume_demand(stream);
                    let _ = stream.queue.push(out_ref);
                    stream.metrics.record_enqueue(stream.queue.len() as u32);
                }
            }
            ProcessResult::Filtered => {
                // Element filtered out; no output to enqueue.
                // Buffer should be returned to pool by resource manager.
            }
            ProcessResult::Multiple(outputs) => {
                // Fan-out: enqueue into multiple output streams.
                let output_streams = self.stream_index.outputs[stage_idx].clone();
                for (i, output) in outputs.into_iter().enumerate() {
                    if let Some(&si) = output_streams.get(i) {
                        let seq = self.next_global_sequence;
                        self.next_global_sequence += 1;
                        let out_ref = StreamElementRef {
                            sequence: seq,
                            origin_sequence,
                            buffer_handle: output.payload,
                            meta: ElementMeta::new(
                                seq,
                                origin_timestamp_ns,
                                output.meta.content_type,
                            ),
                            enqueued_at: Instant::now(),
                        };
                        let stream = &mut self.streams[si];
                        consume_demand(stream);
                        let _ = stream.queue.push(out_ref);
                        stream.metrics.record_enqueue(stream.queue.len() as u32);
                    }
                }
            }
        }
        Ok(())
    }

    /// Execute a sink push.
    ///
    /// # HOT PATH
    async fn execute_push(
        &mut self,
        stage_idx: usize,
        component_id: ComponentId,
        input_stream_id: StreamId,
    ) -> Result<(), FlowError> {
        let instance = &mut self.instances[stage_idx];
        let input_si = self
            .streams
            .iter()
            .position(|s| s.id == input_stream_id)
            .expect("input stream not found");

        let elem_ref = self.streams[input_si]
            .queue
            .pop()
            .expect("consumer_has_input was true but queue is empty");

        replenish_demand(&mut self.streams[input_si]);

        // The element's pipeline-entry timestamp, for end-to-end latency below.
        let origin_timestamp_ns = elem_ref.meta.timestamp_ns;
        // Set before the invoke so a failing push still yields a span
        // attributed to the right element.
        self.current_element_sequence = Some(elem_ref.origin_sequence);

        let stream_element = StreamElement {
            meta: elem_ref.meta,
            payload: elem_ref.buffer_handle,
        };

        let _signal = tokio::time::timeout(
            self.timeouts.component_invocation_timeout,
            self.invoker
                .invoke_push(instance, component_id, stream_element),
        )
        .await
        .map_err(|_| FlowError::ComponentTimeout {
            component: component_id,
            timeout: self.timeouts.component_invocation_timeout,
        })?
        .map_err(|e| FlowError::ComponentError {
            component: component_id,
            error: e,
        })?;

        // The element has now completed its full journey (source → sink):
        // record its end-to-end latency from the pipeline-entry timestamp.
        let latency_ns = torvyn_types::current_timestamp_ns().saturating_sub(origin_timestamp_ns);
        self.event_sink
            .record_flow_latency(self.flow_id, latency_ns);

        // Backpressure signal from sink is handled by the check_backpressure_all
        // method which runs after every stage execution.
        Ok(())
    }

    /// Check all streams for backpressure transitions.
    ///
    /// # HOT PATH
    fn check_backpressure_all(&mut self) {
        for stream in &mut self.streams {
            let queue_len = stream.queue.len();
            let capacity = stream.queue.capacity();
            let low_wm = stream.low_watermark_depth();
            let currently_active = stream.backpressure.is_active();

            if let Some(activate) =
                check_backpressure_transition(queue_len, capacity, low_wm, currently_active)
            {
                if activate {
                    if stream.backpressure.try_activate() {
                        stream.metrics.record_backpressure_event();
                        self.event_sink.record_backpressure(
                            self.flow_id,
                            stream.id,
                            true,
                            queue_len as u32,
                            torvyn_types::current_timestamp_ns(),
                        );
                        debug!(
                            flow_id = %self.flow_id,
                            stream = %stream.id,
                            queue_depth = queue_len,
                            "backpressure activated"
                        );
                    }
                } else if let Some(duration) = stream.backpressure.try_deactivate() {
                    stream.metrics.add_backpressure_duration(duration);
                    self.event_sink.record_backpressure(
                        self.flow_id,
                        stream.id,
                        false,
                        queue_len as u32,
                        torvyn_types::current_timestamp_ns(),
                    );
                    debug!(
                        flow_id = %self.flow_id,
                        stream = %stream.id,
                        duration = ?duration,
                        "backpressure deactivated"
                    );
                }
            }
        }
    }

    /// Returns `true` if all streams are complete (producer done + queue empty).
    fn all_streams_complete(&self) -> bool {
        self.streams.iter().all(|s| s.is_complete())
    }

    /// Propagate `producer_complete` through the pipeline.
    ///
    /// For each non-source stage, if all input streams are complete
    /// (producer done + queue drained), mark all output streams as
    /// `producer_complete`. This cascades completion from sources
    /// through processors to sinks.
    fn propagate_producer_complete(&mut self) {
        for stage_idx in 0..self.topology.stages.len() {
            // Skip sources — they set their own completion in execute_pull.
            if self.stream_index.inputs[stage_idx].is_empty() {
                continue;
            }
            let all_inputs_complete = self.stream_index.inputs[stage_idx]
                .iter()
                .all(|&si| self.streams[si].is_complete());
            if all_inputs_complete {
                for &si in &self.stream_index.outputs[stage_idx] {
                    self.streams[si].producer_complete = true;
                }
            }
        }
    }

    /// Drain remaining elements from all queues.
    ///
    /// Called during the draining phase. Processes remaining elements
    /// with a bounded timeout.
    async fn drain_queues(&mut self) {
        let drain_deadline = Instant::now() + self.timeouts.drain_timeout;

        while Instant::now() < drain_deadline {
            // Try to process one more element.
            match self
                .scheduler
                .next_ready_stage(&self.topology, &self.streams, &self.stream_index)
            {
                Some(execution) => {
                    let _ = self.execute_stage(&execution).await;
                    tokio::task::yield_now().await;
                }
                None => break, // Nothing left to process
            }
        }

        // Force-drain any remaining elements (discard).
        for stream in &mut self.streams {
            let remaining = stream.queue.drain_all();
            if !remaining.is_empty() {
                warn!(
                    flow_id = %self.flow_id,
                    stream = %stream.id,
                    discarded = remaining.len(),
                    "discarded remaining elements during drain"
                );
            }
        }
    }

    /// Emit a flow state change event.
    fn emit_state_change(&self, old: FlowState, new: FlowState) {
        let _ = self.event_tx.try_send(ReactorEvent::FlowStateChanged {
            flow_id: self.flow_id,
            old_state: old,
            new_state: new,
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FlowConfig, StreamConfig};
    use crate::topology::{StageDefinition, StreamConnection};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use torvyn_engine::{ComponentInstance, OutputElement};
    use torvyn_types::{
        BackpressurePolicy, BackpressureSignal, BufferHandle, ComponentRole, NoopEventSink,
        ObservabilityLevel, ResourceId,
    };

    // -- Test invoker that doesn't need private fields --

    /// A test invoker that produces a finite number of elements from the
    /// source, does passthrough processing, and accepts all sink pushes.
    struct TestInvoker {
        pull_count: AtomicU64,
        max_pulls: u64,
        should_error: AtomicBool,
    }

    impl TestInvoker {
        fn new(max_pulls: u64) -> Self {
            Self {
                pull_count: AtomicU64::new(0),
                max_pulls,
                should_error: AtomicBool::new(false),
            }
        }

        fn erroring() -> Self {
            let invoker = Self::new(u64::MAX);
            invoker.should_error.store(true, Ordering::Release);
            invoker
        }
    }

    #[async_trait]
    impl ComponentInvoker for TestInvoker {
        async fn invoke_pull(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
        ) -> Result<Option<OutputElement>, ProcessError> {
            if self.should_error.load(Ordering::Acquire) {
                return Err(ProcessError::Fatal("test error".into()));
            }
            let count = self.pull_count.fetch_add(1, Ordering::Relaxed);
            if count >= self.max_pulls {
                Ok(None) // Stream complete
            } else {
                Ok(Some(OutputElement {
                    meta: ElementMeta::new(count, count * 1000, String::new()),
                    payload: BufferHandle::new(ResourceId::new(count as u32, 0)),
                }))
            }
        }

        async fn invoke_process(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
            element: StreamElement,
        ) -> Result<ProcessResult, ProcessError> {
            if self.should_error.load(Ordering::Acquire) {
                return Err(ProcessError::Fatal("test error".into()));
            }
            Ok(ProcessResult::Output(OutputElement {
                meta: element.meta,
                payload: element.payload,
            }))
        }

        async fn invoke_push(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
            _element: StreamElement,
        ) -> Result<BackpressureSignal, ProcessError> {
            if self.should_error.load(Ordering::Acquire) {
                return Err(ProcessError::Fatal("test error".into()));
            }
            Ok(BackpressureSignal::Ready)
        }

        async fn invoke_init(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
            _config: &str,
        ) -> Result<(), ProcessError> {
            Ok(())
        }

        async fn invoke_teardown(
            &self,
            _instance: &mut ComponentInstance,
            _component_id: ComponentId,
        ) {
        }
    }

    // -- Helper functions --

    fn source_stage(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Source,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn processor_stage(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Processor,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn sink_stage(id: u64) -> StageDefinition {
        StageDefinition {
            component_id: ComponentId::new(id),
            role: ComponentRole::Sink,
            fuel_budget: None,
            config: String::new(),
        }
    }

    fn conn(from: usize, to: usize) -> StreamConnection {
        StreamConnection {
            from_stage: from,
            to_stage: to,
            config: StreamConfig::default(),
        }
    }

    fn make_streams(topology: &FlowTopology, flow_id: FlowId) -> Vec<StreamState> {
        topology
            .connections
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                StreamState::new(
                    StreamId::new(idx as u64),
                    flow_id,
                    topology.stages[c.from_stage].component_id,
                    topology.stages[c.to_stage].component_id,
                    64,
                    BackpressurePolicy::BlockProducer,
                    0.5,
                )
            })
            .collect()
    }

    /// Create placeholder component instances for testing.
    /// The TestInvoker ignores the instance, so contents don't matter.
    async fn make_instances(topology: &FlowTopology) -> Vec<ComponentInstance> {
        use torvyn_engine::mock::MockEngine;
        use torvyn_engine::WasmEngine;
        let engine = MockEngine::new();
        let mut instances = Vec::new();
        for stage in &topology.stages {
            let compiled = engine.compile_component(b"test").unwrap();
            let imports = MockEngine::mock_imports();
            let instance = engine
                .instantiate(
                    &compiled,
                    imports,
                    stage.component_id,
                    &torvyn_engine::WasiConfiguration::deny_all(),
                )
                .await
                .unwrap();
            instances.push(instance);
        }
        instances
    }

    // -- Tests --

    #[tokio::test]
    async fn test_flow_driver_source_to_sink_100_elements() {
        let invoker = TestInvoker::new(100);
        let flow_id = FlowId::new(1);

        let topology = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();

        let streams = make_streams(&topology, flow_id);
        let instances = make_instances(&topology).await;

        let config = FlowConfig::default_with_topology(topology.clone());
        let cancellation = FlowCancellation::new();
        let (event_tx, mut event_rx) = mpsc::channel(256);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            invoker,
            NoopEventSink,
            cancellation,
            event_tx,
        );

        let (id, state, stats) = driver.run().await;

        assert_eq!(id, flow_id);
        assert_eq!(state, FlowState::Completed);
        // 100 elements through source->sink = 100 in the stream.
        assert_eq!(stats.total_elements, 100);

        // Verify state change events.
        let mut saw_running = false;
        let mut saw_draining = false;
        while let Ok(event) = event_rx.try_recv() {
            if let ReactorEvent::FlowStateChanged {
                new_state: FlowState::Running,
                ..
            } = event
            {
                saw_running = true;
            }
            if let ReactorEvent::FlowStateChanged {
                new_state: FlowState::Draining,
                ..
            } = event
            {
                saw_draining = true;
            }
        }
        assert!(saw_running, "expected FlowStateChanged to Running");
        assert!(saw_draining, "expected FlowStateChanged to Draining");
    }

    #[tokio::test]
    async fn test_flow_driver_cancellation_within_100ms() {
        let invoker = TestInvoker::new(u64::MAX); // infinite
        let flow_id = FlowId::new(2);

        let topology = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();

        let streams = make_streams(&topology, flow_id);
        let instances = make_instances(&topology).await;

        let config = FlowConfig::default_with_topology(topology.clone());
        let cancellation = FlowCancellation::new();
        let cancel_handle = cancellation.clone();
        let (event_tx, _event_rx) = mpsc::channel(256);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            invoker,
            NoopEventSink,
            cancellation,
            event_tx,
        );

        let start = Instant::now();
        // Cancel immediately.
        cancel_handle.cancel(CancellationReason::OperatorRequest);

        let (_, state, _) = driver.run().await;
        let elapsed = start.elapsed();

        assert_eq!(state, FlowState::Cancelled);
        assert!(
            elapsed < Duration::from_millis(100),
            "cancellation took {elapsed:?}, expected < 100ms"
        );
    }

    #[tokio::test]
    async fn test_flow_driver_timeout() {
        let invoker = TestInvoker::new(u64::MAX);
        let flow_id = FlowId::new(3);

        let topology = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();

        // Make streams with zero demand so the driver blocks waiting.
        let mut streams = make_streams(&topology, flow_id);
        streams[0].demand = 0;

        let instances = make_instances(&topology).await;

        let mut config = FlowConfig::default_with_topology(topology.clone());
        config.timeouts.flow_deadline = Some(Duration::from_millis(50));

        let cancellation = FlowCancellation::new();
        let (event_tx, _event_rx) = mpsc::channel(256);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            invoker,
            NoopEventSink,
            cancellation,
            event_tx,
        );

        let (_, state, _) = driver.run().await;
        assert_eq!(state, FlowState::Failed);
    }

    #[tokio::test]
    async fn test_flow_driver_cooperative_yield() {
        let invoker = TestInvoker::new(u64::MAX);
        let flow_id = FlowId::new(4);

        let topology = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();

        let streams = make_streams(&topology, flow_id);
        let instances = make_instances(&topology).await;

        let mut config = FlowConfig::default_with_topology(topology.clone());
        // Yield every 4 elements for easy testing.
        config.yield_config.elements_per_yield = 4;

        let cancellation = FlowCancellation::new();
        let cancel_handle = cancellation.clone();
        let (event_tx, mut event_rx) = mpsc::channel(256);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            invoker,
            NoopEventSink,
            cancellation,
            event_tx,
        );

        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel_handle.cancel(CancellationReason::OperatorRequest);
        });

        let _ = driver.run().await;
        cancel_task.await.unwrap();

        // Should have emitted FlowYielded events.
        let mut yield_count = 0;
        while let Ok(event) = event_rx.try_recv() {
            if matches!(event, ReactorEvent::FlowYielded { .. }) {
                yield_count += 1;
            }
        }
        assert!(yield_count > 0, "expected at least one FlowYielded event");
    }

    #[tokio::test]
    async fn test_flow_driver_source_processor_sink() {
        let invoker = TestInvoker::new(50);
        let flow_id = FlowId::new(5);

        let topology = FlowTopology {
            stages: vec![source_stage(1), processor_stage(2), sink_stage(3)],
            connections: vec![conn(0, 1), conn(1, 2)],
        };
        topology.validate().unwrap();

        let streams = make_streams(&topology, flow_id);
        let instances = make_instances(&topology).await;

        let config = FlowConfig::default_with_topology(topology.clone());
        let cancellation = FlowCancellation::new();
        let (event_tx, _event_rx) = mpsc::channel(256);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            invoker,
            NoopEventSink,
            cancellation,
            event_tx,
        );

        let (_, state, stats) = driver.run().await;

        assert_eq!(state, FlowState::Completed);
        // 50 elements through 2 streams (source->proc, proc->sink).
        assert_eq!(stats.total_elements, 100);
    }

    #[tokio::test]
    async fn test_flow_driver_component_error_fail_fast() {
        let invoker = TestInvoker::erroring();
        let flow_id = FlowId::new(6);

        let topology = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();

        let streams = make_streams(&topology, flow_id);
        let instances = make_instances(&topology).await;

        let config = FlowConfig::default_with_topology(topology.clone());
        let cancellation = FlowCancellation::new();
        let (event_tx, _event_rx) = mpsc::channel(256);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            invoker,
            NoopEventSink,
            cancellation,
            event_tx,
        );

        let (_, state, _) = driver.run().await;
        assert_eq!(state, FlowState::Failed);
    }

    #[tokio::test]
    async fn test_flow_driver_lifecycle_state_transitions() {
        let invoker = TestInvoker::new(10);
        let flow_id = FlowId::new(7);

        let topology = FlowTopology {
            stages: vec![source_stage(1), sink_stage(2)],
            connections: vec![conn(0, 1)],
        };
        topology.validate().unwrap();

        let streams = make_streams(&topology, flow_id);
        let instances = make_instances(&topology).await;

        let config = FlowConfig::default_with_topology(topology.clone());
        let cancellation = FlowCancellation::new();
        let (event_tx, mut event_rx) = mpsc::channel(256);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            invoker,
            NoopEventSink,
            cancellation,
            event_tx,
        );

        let (_, state, _) = driver.run().await;

        // Collect all state transitions.
        let mut transitions = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            if let ReactorEvent::FlowStateChanged {
                old_state,
                new_state,
                ..
            } = event
            {
                transitions.push((old_state, new_state));
            }
        }

        // Verify: Instantiated -> Running -> Draining -> Completed
        assert!(transitions.contains(&(FlowState::Instantiated, FlowState::Running)));
        assert!(transitions.contains(&(FlowState::Running, FlowState::Draining)));
        assert_eq!(state, FlowState::Completed);
    }

    // -- Observability recording --

    /// An `EventSink` that keeps every invocation and span it is handed, so a
    /// test can assert on what the driver actually reported.
    #[derive(Default)]
    struct RecordingSink {
        invocations: std::sync::Mutex<Vec<(ComponentId, u64, u64)>>,
        spans: std::sync::Mutex<Vec<(ComponentId, u64, u64, u64)>>,
        level: ObservabilityLevel,
    }

    impl RecordingSink {
        fn at(level: ObservabilityLevel) -> Arc<Self> {
            Arc::new(Self {
                level,
                ..Default::default()
            })
        }
    }

    impl EventSink for RecordingSink {
        fn record_invocation(
            &self,
            _flow_id: FlowId,
            component_id: ComponentId,
            start_ns: u64,
            end_ns: u64,
            _status: InvocationStatus,
        ) {
            self.invocations.lock().expect("invocations mutex").push((
                component_id,
                start_ns,
                end_ns,
            ));
        }

        fn record_element_transfer(&self, _: FlowId, _: StreamId, _: u64, _: u32) {}

        fn record_backpressure(&self, _: FlowId, _: StreamId, _: bool, _: u32, _: u64) {}

        fn record_copy(
            &self,
            _: FlowId,
            _: torvyn_types::ResourceId,
            _: ComponentId,
            _: ComponentId,
            _: u64,
            _: torvyn_types::CopyReason,
        ) {
        }

        fn record_element_span(
            &self,
            _flow_id: FlowId,
            component_id: ComponentId,
            element_sequence: u64,
            start_ns: u64,
            end_ns: u64,
            _status: InvocationStatus,
        ) {
            if self.level != ObservabilityLevel::Diagnostic {
                return;
            }
            self.spans.lock().expect("spans mutex").push((
                component_id,
                element_sequence,
                start_ns,
                end_ns,
            ));
        }

        fn level(&self) -> ObservabilityLevel {
            self.level
        }
    }

    async fn run_recorded(
        element_count: u64,
        three_stage: bool,
        level: ObservabilityLevel,
    ) -> Arc<RecordingSink> {
        let sink = RecordingSink::at(level);
        let flow_id = FlowId::new(1);
        let topology = if three_stage {
            FlowTopology {
                stages: vec![source_stage(1), processor_stage(2), sink_stage(3)],
                connections: vec![conn(0, 1), conn(1, 2)],
            }
        } else {
            FlowTopology {
                stages: vec![source_stage(1), sink_stage(2)],
                connections: vec![conn(0, 1)],
            }
        };
        topology.validate().unwrap();

        let streams = make_streams(&topology, flow_id);
        let instances = make_instances(&topology).await;
        let config = FlowConfig::default_with_topology(topology.clone());
        let (event_tx, _event_rx) = mpsc::channel(1024);

        let driver = FlowDriver::new(
            flow_id,
            config,
            instances,
            streams,
            TestInvoker::new(element_count),
            Arc::clone(&sink),
            FlowCancellation::new(),
            event_tx,
        );
        let (_, state, _) = driver.run().await;
        assert_eq!(state, FlowState::Completed);
        sink
    }

    /// Regression test. `record_invocation` is documented to take absolute
    /// epoch timestamps, and every sink derives the invocation's duration
    /// from `end_ns - start_ns`.
    ///
    /// The driver used to pass `start.elapsed()` and
    /// `(start + duration).elapsed()` — both of which measure time *since* an
    /// instant, not a point in time. That made `start_ns` roughly the
    /// invocation's duration and `end_ns` roughly zero, so every sink
    /// computing `end_ns - start_ns` saturated to 0 and the per-component
    /// processing-time histogram recorded nothing but zeros.
    #[tokio::test]
    async fn test_record_invocation_reports_ordered_epoch_timestamps() {
        let before = torvyn_types::current_timestamp_ns();
        let sink = run_recorded(50, false, ObservabilityLevel::Production).await;
        let after = torvyn_types::current_timestamp_ns();

        let invocations = sink.invocations.lock().expect("invocations mutex");
        assert!(
            !invocations.is_empty(),
            "the driver must report its invocations"
        );

        for (component_id, start_ns, end_ns) in invocations.iter().copied() {
            assert!(
                end_ns >= start_ns,
                "component {component_id}: end_ns ({end_ns}) precedes start_ns ({start_ns});                  a sink computing end - start would saturate to zero"
            );
            assert!(
                start_ns >= before && end_ns <= after,
                "component {component_id}: [{start_ns}, {end_ns}] falls outside the run's                  wall-clock window [{before}, {after}] — these are not epoch timestamps"
            );
            assert!(
                end_ns.saturating_sub(start_ns) < 10_000_000_000,
                "component {component_id}: implausible duration {} ns",
                end_ns.saturating_sub(start_ns),
            );
        }
    }

    /// The mock invoker returns immediately, so most invocations are shorter
    /// than a nanosecond and legitimately round to zero. The point of this
    /// test is that the reported durations are not *uniformly* zero, which is
    /// what the old arithmetic guaranteed.
    #[tokio::test]
    async fn test_reported_durations_are_not_uniformly_zero() {
        let sink = run_recorded(200, true, ObservabilityLevel::Production).await;
        let invocations = sink.invocations.lock().expect("invocations mutex");
        let nonzero = invocations
            .iter()
            .filter(|(_, start, end)| end.saturating_sub(*start) > 0)
            .count();
        assert!(
            nonzero > 0,
            "every one of {} invocations reported a zero duration",
            invocations.len(),
        );
    }

    #[tokio::test]
    async fn test_spans_are_recorded_at_diagnostic_level() {
        const ELEMENTS: u64 = 20;
        let sink = run_recorded(ELEMENTS, true, ObservabilityLevel::Diagnostic).await;
        let spans = sink.spans.lock().expect("spans mutex");

        // Source, processor, and sink each handle every element once.
        assert_eq!(
            spans.len() as u64,
            ELEMENTS * 3,
            "expected one span per stage per element"
        );

        for (component_id, _sequence, start_ns, end_ns) in spans.iter().copied() {
            assert!(
                end_ns >= start_ns,
                "component {component_id}: span ends before it starts"
            );
        }

        // Every element must be traceable end to end. Origin sequences are
        // drawn from a counter shared by every emit site, so in a three-stage
        // flow the source's elements are numbered 0, 2, 4, ... rather than
        // 0, 1, 2, ... — what matters is that there are exactly `ELEMENTS` of
        // them and that each is seen by all three stages, which is what lets
        // a trace group three spans into one element's journey.
        let mut by_element: std::collections::BTreeMap<u64, std::collections::HashSet<u64>> =
            std::collections::BTreeMap::new();
        for (component_id, sequence, _, _) in spans.iter().copied() {
            by_element
                .entry(sequence)
                .or_default()
                .insert(component_id.as_u64());
        }
        assert_eq!(
            by_element.len() as u64,
            ELEMENTS,
            "expected {ELEMENTS} distinct traced elements, got {}",
            by_element.len()
        );
        for (sequence, stages) in &by_element {
            assert_eq!(
                stages.len(),
                3,
                "element {sequence} was seen by components {stages:?}, expected all three stages"
            );
        }
    }

    #[tokio::test]
    async fn test_no_spans_below_diagnostic_level() {
        let sink = run_recorded(20, true, ObservabilityLevel::Production).await;
        assert!(
            sink.spans.lock().expect("spans mutex").is_empty(),
            "Production level must not retain per-element spans"
        );
    }
}
