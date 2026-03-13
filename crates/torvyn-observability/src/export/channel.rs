//! In-process channel export for benchmark and test integration.

use crate::events::DiagnosticEvent;
use crate::metrics::snapshot::FlowMetricsSnapshot;
use tokio::sync::mpsc;

/// Export channel receiver for benchmarks.
pub struct ChannelExportReceiver {
    /// Receiver for diagnostic events.
    pub events: mpsc::Receiver<DiagnosticEvent>,
    /// Receiver for flow metrics snapshots.
    pub snapshots: mpsc::Receiver<FlowMetricsSnapshot>,
}

/// Export channel senders (held by the exporter).
pub struct ChannelExportSender {
    /// Sender for diagnostic events.
    pub events: mpsc::Sender<DiagnosticEvent>,
    /// Sender for flow metrics snapshots.
    pub snapshots: mpsc::Sender<FlowMetricsSnapshot>,
}

/// Create an in-process export channel pair.
///
/// # COLD PATH
pub fn channel_export(capacity: usize) -> (ChannelExportSender, ChannelExportReceiver) {
    let (event_tx, event_rx) = mpsc::channel(capacity);
    let (snap_tx, snap_rx) = mpsc::channel(capacity);
    (
        ChannelExportSender {
            events: event_tx,
            snapshots: snap_tx,
        },
        ChannelExportReceiver {
            events: event_rx,
            snapshots: snap_rx,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_channel_export_roundtrip() {
        let (sender, mut receiver) = channel_export(16);

        let event = DiagnosticEvent::new(
            torvyn_types::Severity::Info,
            crate::events::EventCategory::Lifecycle,
            crate::events::EventPayload::FlowStarted {
                component_count: 1,
            },
        );

        sender.events.send(event).await.unwrap();
        let received = receiver.events.recv().await.unwrap();
        assert_eq!(received.event_name(), "flow.started");
    }
}
