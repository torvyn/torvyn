//! JSON event exporter.
//!
//! Writes newline-delimited JSON to stderr or a file.

use crate::events::DiagnosticEvent;
use crate::metrics::MetricsRegistry;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// JSON export target.
pub enum JsonTarget {
    /// Write to stderr.
    Stderr,
    /// Write to a file at the given path.
    File(PathBuf),
}

/// Run the JSON export task.
///
/// Reads events from the channel and writes them as NDJSON.
///
/// # COLD PATH — background task.
pub async fn json_export_task(mut rx: mpsc::Receiver<DiagnosticEvent>, target: JsonTarget) {
    let Some(mut writer) = open_json_writer(&target) else {
        return;
    };

    while let Some(event) = rx.recv().await {
        match serde_json::to_string(&event) {
            Ok(json) => {
                let _ = writeln!(writer, "{json}");
            }
            Err(e) => {
                eprintln!("torvyn-observability: JSON serialization error: {e}");
            }
        }
    }
}

/// Open a `JsonTarget` for writing, returning `None` if a file cannot be opened.
///
/// # COLD PATH
fn open_json_writer(target: &JsonTarget) -> Option<Box<dyn Write + Send>> {
    match target {
        JsonTarget::Stderr => Some(Box::new(std::io::stderr())),
        JsonTarget::File(path) => match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(f) => Some(Box::new(f)),
            Err(e) => {
                eprintln!("torvyn-observability: failed to open export file: {e}");
                None
            }
        },
    }
}

/// Periodically snapshot every active flow's metrics and write them as
/// newline-delimited JSON to the target.
///
/// Runs until `stop` is closed — the collector holds the paired sender and
/// drops it on teardown, so this exits when the collector is torn down. A write
/// or serialization failure is logged and skipped, so export never disrupts a
/// running pipeline.
///
/// The caller guarantees `interval` is non-zero (enforced by
/// [`ObservabilityConfig::validate`](crate::config::ObservabilityConfig::validate)).
///
/// # COLD PATH — background task: one line per active flow per interval.
pub async fn metrics_export_task(
    registry: Arc<MetricsRegistry>,
    target: JsonTarget,
    interval: Duration,
    mut stop: mpsc::Receiver<()>,
) {
    let Some(mut writer) = open_json_writer(&target) else {
        return;
    };

    let mut ticker = tokio::time::interval(interval);
    // Skip missed ticks rather than bursting after a slow write.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                for snapshot in registry.snapshot_all() {
                    match serde_json::to_string(&snapshot) {
                        Ok(json) => {
                            let _ = writeln!(writer, "{json}");
                        }
                        Err(e) => {
                            eprintln!(
                                "torvyn-observability: metrics serialization error: {e}"
                            );
                        }
                    }
                }
                let _ = writer.flush();
            }
            _ = stop.recv() => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventCategory, EventPayload};

    #[tokio::test]
    async fn test_json_export_to_stderr() {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let handle = tokio::spawn(json_export_task(rx, JsonTarget::Stderr));

        let event = DiagnosticEvent::new(
            torvyn_types::Severity::Info,
            EventCategory::Lifecycle,
            EventPayload::FlowStarted { component_count: 1 },
        );
        tx.send(event).await.unwrap();
        drop(tx);

        handle.await.unwrap();
        // If we reach here without panic, export succeeded.
    }

    #[tokio::test]
    async fn test_metrics_export_task_writes_flow_metrics_and_stops_on_drop() {
        use crate::metrics::MetricsRegistry;
        use torvyn_types::{ComponentId, FlowId, StreamId};

        // A registry with one flow that has recorded some elements.
        let registry = Arc::new(MetricsRegistry::new());
        let fm = registry
            .register_flow(
                FlowId::new(1),
                &[ComponentId::new(1)],
                &[StreamId::new(0)],
                0,
            )
            .unwrap();
        fm.elements_total.increment(42);

        let path = std::env::temp_dir().join(format!(
            "torvyn_metrics_export_{}.ndjson",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let (stop_tx, stop_rx) = tokio::sync::mpsc::channel(1);
        let handle = tokio::spawn(metrics_export_task(
            Arc::clone(&registry),
            JsonTarget::File(path.clone()),
            Duration::from_millis(20),
            stop_rx,
        ));

        // Let at least one export tick fire.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Dropping the stop sender must terminate the task promptly.
        drop(stop_tx);
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("metrics_export_task must stop when the stop channel closes")
            .unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            contents.contains("\"elements_total\":42"),
            "export file must contain the flow's recorded metrics; got: {contents}",
        );
    }
}
