//! Audit logging for security-relevant actions.
//!
//! Every capability exercise, denial, lifecycle event, and security violation
//! is recorded as a structured `AuditEvent`. Events are emitted to an `AuditSink`
//! trait, allowing pluggable backends.
//!
//! Per Doc 06 §8: audit events serve security teams (incident investigation,
//! compliance) and operators (capability governance).

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use torvyn_types::{ComponentId, FlowId};

use crate::tenant::TenantId;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AuditSeverity
// ---------------------------------------------------------------------------

/// Severity level for audit events.
///
/// Per Doc 06 §8.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum AuditSeverity {
    /// Normal operation, recorded for audit trail.
    Info,
    /// Potential issue (e.g., optional capability not granted, unused grant).
    Warning,
    /// Security-relevant failure (e.g., capability denial, sandbox violation).
    Critical,
}

impl fmt::Display for AuditSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditSeverity::Info => write!(f, "INFO"),
            AuditSeverity::Warning => write!(f, "WARN"),
            AuditSeverity::Critical => write!(f, "CRIT"),
        }
    }
}

// ---------------------------------------------------------------------------
// AuditEventKind
// ---------------------------------------------------------------------------

/// The specific payload of an audit event.
///
/// Per Doc 06 §8.1, events are categorized into lifecycle, capability, and
/// security events.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum AuditEventKind {
    // --- Lifecycle Events ---
    /// A component instance was created with its sandbox configuration.
    ComponentInstantiated {
        /// The resolved capabilities for the component.
        resolved_capabilities: Vec<String>,
    },
    /// A component instance was terminated.
    ComponentTerminated {
        /// The reason for termination.
        reason: String,
    },
    /// A pipeline flow began execution.
    FlowStarted {
        /// The component IDs in the flow.
        component_ids: Vec<u64>,
    },
    /// A pipeline flow ended.
    FlowTerminated {
        /// The reason for termination.
        reason: String,
        /// Number of elements processed.
        elements_processed: u64,
    },

    // --- Capability Events ---
    /// A capability grant was resolved during linking.
    CapabilityGrantResolved {
        /// The requested capability.
        requested: String,
        /// The granted capability.
        granted: String,
        /// The effective (intersected) capability.
        effective: String,
    },
    /// A required capability was missing or incompatible at link time.
    CapabilityDeniedAtLink {
        /// The denied capability.
        capability: String,
        /// The reason for denial.
        reason: String,
    },
    /// A cold-path capability was exercised at runtime (individual event).
    CapabilityExercised {
        /// The exercised capability.
        capability: String,
        /// Optional detail about the exercise.
        detail: Option<String>,
    },
    /// Hot-path capability usage (aggregated counter).
    CapabilityExercisedAggregate {
        /// The capability being aggregated.
        capability: String,
        /// The count of exercises in the interval.
        count: u64,
        /// The interval in milliseconds.
        interval_ms: u64,
    },
    /// A runtime capability check failed.
    CapabilityDeniedAtRuntime {
        /// The denied capability.
        capability: String,
        /// Details about the denial.
        detail: String,
    },
    /// A warning during capability resolution.
    CapabilityResolutionWarning {
        /// Details about the warning.
        detail: String,
    },

    // --- Security Events ---
    /// A sandbox limit was hit.
    SandboxViolation {
        /// The type of violation.
        violation_type: String,
        /// Details about the violation.
        detail: String,
    },
}

impl fmt::Display for AuditEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditEventKind::ComponentInstantiated {
                resolved_capabilities,
            } => {
                write!(
                    f,
                    "component_instantiated: caps=[{}]",
                    resolved_capabilities.join(", ")
                )
            }
            AuditEventKind::ComponentTerminated { reason } => {
                write!(f, "component_terminated: {reason}")
            }
            AuditEventKind::FlowStarted { component_ids } => {
                write!(f, "flow_started: components={component_ids:?}")
            }
            AuditEventKind::FlowTerminated {
                reason,
                elements_processed,
            } => {
                write!(
                    f,
                    "flow_terminated: {reason} ({elements_processed} elements)"
                )
            }
            AuditEventKind::CapabilityGrantResolved {
                requested,
                granted,
                effective,
            } => {
                write!(
                    f,
                    "cap_resolved: req={requested}, grant={granted}, eff={effective}"
                )
            }
            AuditEventKind::CapabilityDeniedAtLink { capability, reason } => {
                write!(f, "cap_denied_link: {capability}: {reason}")
            }
            AuditEventKind::CapabilityExercised { capability, detail } => {
                write!(f, "cap_exercised: {capability}")?;
                if let Some(d) = detail {
                    write!(f, " ({d})")?;
                }
                Ok(())
            }
            AuditEventKind::CapabilityExercisedAggregate {
                capability,
                count,
                interval_ms,
            } => {
                write!(
                    f,
                    "cap_exercised_agg: {capability} x{count} in {interval_ms}ms"
                )
            }
            AuditEventKind::CapabilityDeniedAtRuntime { capability, detail } => {
                write!(f, "cap_denied_runtime: {capability}: {detail}")
            }
            AuditEventKind::CapabilityResolutionWarning { detail } => {
                write!(f, "cap_resolution_warn: {detail}")
            }
            AuditEventKind::SandboxViolation {
                violation_type,
                detail,
            } => {
                write!(f, "sandbox_violation: {violation_type}: {detail}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AuditEvent
// ---------------------------------------------------------------------------

/// A structured audit event.
///
/// Per Doc 06 §8.2, events carry a monotonic sequence number, timestamp,
/// severity, optional identifiers, and the event payload.
///
/// # Invariants
/// - `sequence` is monotonically increasing within a process.
/// - `timestamp_ns` is wall-clock nanoseconds since Unix epoch.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AuditEvent {
    /// Monotonic event sequence number.
    pub sequence: u64,
    /// Wall-clock timestamp (nanoseconds since Unix epoch).
    pub timestamp_ns: u64,
    /// Event severity.
    pub severity: AuditSeverity,
    /// The tenant associated with this event (if applicable).
    pub tenant_id: Option<TenantId>,
    /// The component associated with this event (if applicable).
    pub component_id: Option<ComponentId>,
    /// The flow associated with this event (if applicable).
    pub flow_id: Option<FlowId>,
    /// The specific event payload.
    pub event: AuditEventKind,
}

/// Global sequence counter for audit events.
static AUDIT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl AuditEvent {
    /// Create a new audit event with auto-assigned sequence number and current timestamp.
    ///
    /// # WARM PATH — called per audit event emission.
    pub fn new(
        severity: AuditSeverity,
        component_id: Option<ComponentId>,
        flow_id: Option<FlowId>,
        tenant_id: Option<TenantId>,
        event: AuditEventKind,
    ) -> Self {
        Self {
            sequence: AUDIT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            timestamp_ns: torvyn_types::current_timestamp_ns(),
            severity,
            tenant_id,
            component_id,
            flow_id,
            event,
        }
    }

    /// Serialize to JSON.
    ///
    /// # WARM PATH — called per audit event by sinks that write JSON.
    #[cfg(feature = "serde")]
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl fmt::Display for AuditEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] seq={} ts={} ",
            self.severity, self.sequence, self.timestamp_ns
        )?;
        if let Some(tid) = &self.tenant_id {
            write!(f, "tenant={} ", tid)?;
        }
        if let Some(cid) = &self.component_id {
            write!(f, "component={} ", cid)?;
        }
        if let Some(fid) = &self.flow_id {
            write!(f, "flow={} ", fid)?;
        }
        write!(f, "{}", self.event)
    }
}

// ---------------------------------------------------------------------------
// AuditSink trait
// ---------------------------------------------------------------------------

/// Trait for consuming audit events.
///
/// Per Doc 06 §8.3. Implementations must be `Send + Sync` since audit events
/// may be emitted from any thread.
pub trait AuditSink: Send + Sync {
    /// Record a single audit event synchronously.
    ///
    /// Implementations should be non-blocking and return quickly.
    /// If buffering is needed, implement internal buffering with periodic flush.
    fn record(&self, event: AuditEvent);

    /// Flush any buffered events. Called during graceful shutdown.
    fn flush(&self);
}

// ---------------------------------------------------------------------------
// AuditSinkHandle
// ---------------------------------------------------------------------------

/// A handle to an `AuditSink` that can be cheaply cloned and shared.
///
/// Wraps an `Arc<dyn AuditSink>` for ergonomic use across subsystems.
#[derive(Clone)]
pub struct AuditSinkHandle {
    inner: std::sync::Arc<dyn AuditSink>,
}

impl AuditSinkHandle {
    /// Create a new handle from a sink implementation.
    pub fn new(sink: impl AuditSink + 'static) -> Self {
        Self {
            inner: std::sync::Arc::new(sink),
        }
    }

    /// Create a no-op handle that discards all events.
    pub fn noop() -> Self {
        Self::new(NoopAuditSink)
    }

    /// Record an event synchronously.
    #[inline]
    pub fn record_sync(&self, event: AuditEvent) {
        self.inner.record(event);
    }

    /// Flush buffered events.
    pub fn flush(&self) {
        self.inner.flush();
    }
}

impl fmt::Debug for AuditSinkHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AuditSinkHandle(..)")
    }
}

// ---------------------------------------------------------------------------
// NoopAuditSink
// ---------------------------------------------------------------------------

/// An audit sink that discards all events. Used in tests and when audit is disabled.
pub struct NoopAuditSink;

impl AuditSink for NoopAuditSink {
    fn record(&self, _event: AuditEvent) {}
    fn flush(&self) {}
}

// ---------------------------------------------------------------------------
// StdoutAuditSink
// ---------------------------------------------------------------------------

/// An audit sink that writes JSON-formatted events to stdout.
///
/// Suitable for development and piping to log aggregators.
/// Per Doc 06 §8.3.
pub struct StdoutAuditSink;

#[cfg(feature = "serde")]
impl AuditSink for StdoutAuditSink {
    fn record(&self, event: AuditEvent) {
        if let Ok(json) = event.to_json() {
            // Best-effort: ignore write errors to stdout
            let _ = writeln!(std::io::stdout().lock(), "{json}");
        }
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
    }
}

#[cfg(not(feature = "serde"))]
impl AuditSink for StdoutAuditSink {
    fn record(&self, event: AuditEvent) {
        let _ = writeln!(std::io::stdout().lock(), "{event}");
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
    }
}

// ---------------------------------------------------------------------------
// FileAuditSink
// ---------------------------------------------------------------------------

/// An audit sink that writes to a rotating audit log file.
///
/// Per DI-16 and Doc 06 §8.3:
/// - Size-based rotation with configurable max file size.
/// - Rotated files are renamed with a numeric suffix (e.g., `audit.log.1`).
/// - Maximum number of rotated files is configurable.
///
/// # Invariants
/// - The current log file is always writable.
/// - Rotation is atomic within the lock.
///
/// # Examples
/// ```no_run
/// use torvyn_security::FileAuditSink;
///
/// let sink = FileAuditSink::new(
///     "/var/log/torvyn/audit.log",
///     10 * 1024 * 1024, // 10 MiB max
///     5, // keep 5 rotated files
/// ).unwrap();
/// ```
#[cfg(feature = "audit-file")]
pub struct FileAuditSink {
    state: Mutex<FileAuditState>,
    path: PathBuf,
    max_bytes: u64,
    max_files: u32,
}

#[cfg(feature = "audit-file")]
struct FileAuditState {
    writer: std::io::BufWriter<std::fs::File>,
    current_bytes: u64,
}

#[cfg(feature = "audit-file")]
impl FileAuditSink {
    /// Create a new `FileAuditSink`.
    ///
    /// Creates the log file (and parent directories) if they don't exist.
    ///
    /// # COLD PATH — called during runtime initialization.
    ///
    /// # Errors
    /// Returns `std::io::Error` if the file cannot be created.
    pub fn new(
        path: impl AsRef<Path>,
        max_bytes: u64,
        max_files: u32,
    ) -> Result<Self, std::io::Error> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let current_bytes = file.metadata()?.len();

        Ok(Self {
            state: Mutex::new(FileAuditState {
                writer: std::io::BufWriter::new(file),
                current_bytes,
            }),
            path,
            max_bytes,
            max_files,
        })
    }

    /// Perform log rotation.
    ///
    /// Renames `audit.log` -> `audit.log.1`, `audit.log.1` -> `audit.log.2`, etc.
    /// Deletes the oldest file if `max_files` would be exceeded.
    ///
    /// # COLD PATH — called when file size exceeds `max_bytes`.
    fn rotate(&self, state: &mut FileAuditState) -> Result<(), std::io::Error> {
        // Flush and drop current writer
        state.writer.flush()?;

        // Rotate existing files
        for i in (1..self.max_files).rev() {
            let from = self.rotated_path(i);
            let to = self.rotated_path(i + 1);
            if from.exists() {
                if i + 1 > self.max_files {
                    std::fs::remove_file(&from)?;
                } else {
                    std::fs::rename(&from, &to)?;
                }
            }
        }

        // Rename current file to .1
        let rotated = self.rotated_path(1);
        std::fs::rename(&self.path, &rotated)?;

        // Delete oldest if exceeds max
        let oldest = self.rotated_path(self.max_files + 1);
        if oldest.exists() {
            let _ = std::fs::remove_file(&oldest);
        }

        // Open a fresh file
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        state.writer = std::io::BufWriter::new(file);
        state.current_bytes = 0;

        Ok(())
    }

    fn rotated_path(&self, n: u32) -> PathBuf {
        let mut p = self.path.as_os_str().to_os_string();
        p.push(format!(".{n}"));
        PathBuf::from(p)
    }
}

#[cfg(all(feature = "audit-file", feature = "serde"))]
impl AuditSink for FileAuditSink {
    fn record(&self, event: AuditEvent) {
        let json = match event.to_json() {
            Ok(j) => j,
            Err(_) => return, // Best effort
        };

        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return, // Poisoned lock — best effort
        };

        let line = format!("{json}\n");
        let line_bytes = line.len() as u64;

        // Check if rotation is needed
        if state.current_bytes + line_bytes > self.max_bytes && self.rotate(&mut state).is_err() {
            return; // Best effort
        }

        if state.writer.write_all(line.as_bytes()).is_ok() {
            state.current_bytes += line_bytes;
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.lock() {
            let _ = state.writer.flush();
        }
    }
}

#[cfg(all(feature = "audit-file", not(feature = "serde")))]
impl AuditSink for FileAuditSink {
    fn record(&self, event: AuditEvent) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };

        let line = format!("{event}\n");
        let line_bytes = line.len() as u64;

        if state.current_bytes + line_bytes > self.max_bytes {
            if self.rotate(&mut state).is_err() {
                return;
            }
        }

        if state.writer.write_all(line.as_bytes()).is_ok() {
            state.current_bytes += line_bytes;
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.lock() {
            let _ = state.writer.flush();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent::new(
            AuditSeverity::Info,
            Some(ComponentId::new(1)),
            Some(FlowId::new(1)),
            None,
            AuditEventKind::CapabilityExercised {
                capability: "clock:wall".to_owned(),
                detail: None,
            },
        );
        assert_eq!(event.severity, AuditSeverity::Info);
        assert_eq!(event.component_id, Some(ComponentId::new(1)));
    }

    #[test]
    fn test_audit_event_sequence_monotonic() {
        let e1 = AuditEvent::new(
            AuditSeverity::Info,
            None,
            None,
            None,
            AuditEventKind::ComponentTerminated {
                reason: "test".to_owned(),
            },
        );
        let e2 = AuditEvent::new(
            AuditSeverity::Info,
            None,
            None,
            None,
            AuditEventKind::ComponentTerminated {
                reason: "test".to_owned(),
            },
        );
        assert!(e2.sequence > e1.sequence);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_audit_event_json_serialization() {
        let event = AuditEvent::new(
            AuditSeverity::Critical,
            Some(ComponentId::new(42)),
            None,
            None,
            AuditEventKind::CapabilityDeniedAtRuntime {
                capability: "torvyn:custom-metrics".to_owned(),
                detail: "not in resolved set".to_owned(),
            },
        );
        let json = event.to_json().unwrap();
        assert!(json.contains("CRIT") || json.contains("Critical"));
        assert!(json.contains("custom-metrics"));
    }

    #[test]
    fn test_noop_sink() {
        let sink = NoopAuditSink;
        sink.record(AuditEvent::new(
            AuditSeverity::Info,
            None,
            None,
            None,
            AuditEventKind::ComponentTerminated {
                reason: "test".to_owned(),
            },
        ));
        sink.flush();
        // No panic = success
    }

    #[test]
    fn test_audit_sink_handle_noop() {
        let handle = AuditSinkHandle::noop();
        handle.record_sync(AuditEvent::new(
            AuditSeverity::Info,
            None,
            None,
            None,
            AuditEventKind::ComponentTerminated {
                reason: "test".to_owned(),
            },
        ));
        handle.flush();
    }

    #[test]
    fn test_severity_ordering() {
        assert!(AuditSeverity::Info < AuditSeverity::Warning);
        assert!(AuditSeverity::Warning < AuditSeverity::Critical);
    }

    #[cfg(feature = "audit-file")]
    #[test]
    fn test_file_audit_sink_write_and_rotate() {
        let dir = std::env::temp_dir().join("torvyn_audit_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("audit.log");

        // Small max_bytes to trigger rotation quickly
        let sink = FileAuditSink::new(&log_path, 200, 3).unwrap();

        for i in 0..20 {
            sink.record(AuditEvent::new(
                AuditSeverity::Info,
                Some(ComponentId::new(i)),
                None,
                None,
                AuditEventKind::CapabilityExercised {
                    capability: "clock:wall".to_owned(),
                    detail: None,
                },
            ));
        }
        sink.flush();

        // Current file should exist
        assert!(log_path.exists());

        // At least one rotated file should exist
        let rotated = dir.join("audit.log.1");
        assert!(rotated.exists());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_audit_event_display() {
        let event = AuditEvent::new(
            AuditSeverity::Warning,
            Some(ComponentId::new(5)),
            Some(FlowId::new(3)),
            Some(TenantId::default_tenant()),
            AuditEventKind::CapabilityResolutionWarning {
                detail: "optional stderr not granted".to_owned(),
            },
        );
        let display = format!("{event}");
        assert!(display.contains("WARN"));
        assert!(display.contains("component-5"));
        assert!(display.contains("flow-3"));
    }
}
