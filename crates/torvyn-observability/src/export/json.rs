//! JSON event exporter.
//!
//! Writes newline-delimited JSON to stderr or a file.

use crate::events::DiagnosticEvent;
use std::io::Write;
use std::path::PathBuf;
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
    let mut writer: Box<dyn Write + Send> = match &target {
        JsonTarget::Stderr => Box::new(std::io::stderr()),
        JsonTarget::File(path) => {
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!("torvyn-observability: failed to open export file: {e}");
                    return;
                }
            }
        }
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
}
