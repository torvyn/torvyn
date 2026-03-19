//! Diagnostic event recorder.
//!
//! Receives events from a bounded MPSC channel and stores them in a
//! circular buffer for inspection API access and export.

use super::types::DiagnosticEvent;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Sender half of the event channel, given to event producers.
pub type EventSender = mpsc::Sender<DiagnosticEvent>;

/// Receiver half of the event channel, owned by the recorder.
pub type EventReceiver = mpsc::Receiver<DiagnosticEvent>;

/// Create a bounded event channel.
///
/// # COLD PATH
pub fn event_channel(capacity: usize) -> (EventSender, EventReceiver) {
    mpsc::channel(capacity)
}

/// In-memory event buffer for the inspection API and export.
///
/// Stores the most recent N events in a ring buffer.
pub struct EventBuffer {
    events: Mutex<VecDeque<DiagnosticEvent>>,
    capacity: usize,
}

impl EventBuffer {
    /// Create a new event buffer.
    ///
    /// # COLD PATH
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Push an event into the buffer. Drops oldest if full.
    ///
    /// # WARM PATH — acquires mutex (acceptable; this runs in the
    /// recorder task, not on the hot path).
    pub fn push(&self, event: DiagnosticEvent) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if events.len() >= self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    /// Read the most recent N events.
    ///
    /// # COLD PATH
    pub fn recent(&self, count: usize) -> Vec<DiagnosticEvent> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.iter().rev().take(count).cloned().collect()
    }

    /// Read all events for a specific flow.
    ///
    /// # COLD PATH
    pub fn for_flow(&self, flow_id: torvyn_types::FlowId) -> Vec<DiagnosticEvent> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events
            .iter()
            .filter(|e| e.flow_id == Some(flow_id))
            .cloned()
            .collect()
    }

    /// Current number of buffered events.
    pub fn len(&self) -> usize {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The event recorder task. Runs in a dedicated Tokio task.
///
/// Reads events from the channel and pushes them to the buffer.
///
/// # COLD PATH — this is a background task.
pub async fn event_recorder_task(
    mut receiver: EventReceiver,
    buffer: Arc<EventBuffer>,
    export_tx: Option<mpsc::Sender<DiagnosticEvent>>,
) {
    while let Some(event) = receiver.recv().await {
        // Forward to export if configured.
        if let Some(ref tx) = export_tx {
            // Best-effort: don't block if export channel is full.
            let _ = tx.try_send(event.clone());
        }
        buffer.push(event);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::types::{EventCategory, EventPayload};
    use torvyn_types::Severity;

    fn make_event(flow_id: u64) -> DiagnosticEvent {
        DiagnosticEvent::new(
            Severity::Info,
            EventCategory::Lifecycle,
            EventPayload::FlowStarted { component_count: 1 },
        )
        .with_flow(torvyn_types::FlowId::new(flow_id))
    }

    #[test]
    fn test_event_buffer_push_and_read() {
        let buf = EventBuffer::new(100);
        buf.push(make_event(1));
        buf.push(make_event(2));

        assert_eq!(buf.len(), 2);
        let recent = buf.recent(10);
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn test_event_buffer_overflow() {
        let buf = EventBuffer::new(3);
        for i in 0..5 {
            buf.push(make_event(i));
        }

        assert_eq!(buf.len(), 3);
        // Oldest (0, 1) should be dropped.
        let recent = buf.recent(10);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_event_buffer_for_flow() {
        let buf = EventBuffer::new(100);
        buf.push(make_event(1));
        buf.push(make_event(2));
        buf.push(make_event(1));

        let flow1 = buf.for_flow(torvyn_types::FlowId::new(1));
        assert_eq!(flow1.len(), 2);
    }

    #[tokio::test]
    async fn test_event_recorder_task() {
        let (tx, rx) = event_channel(100);
        let buffer = Arc::new(EventBuffer::new(100));
        let buffer_clone = Arc::clone(&buffer);

        let handle = tokio::spawn(event_recorder_task(rx, buffer_clone, None));

        tx.send(make_event(1)).await.unwrap();
        tx.send(make_event(2)).await.unwrap();
        drop(tx); // Close channel to stop task.

        handle.await.unwrap();
        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn test_event_recorder_100k_events() {
        let buf = EventBuffer::new(100_000);
        for i in 0..100_000u64 {
            buf.push(make_event(i));
        }
        assert_eq!(buf.len(), 100_000);
    }
}
