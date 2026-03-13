//! Diagnostic event subsystem.

pub mod recorder;
pub mod types;

pub use recorder::{EventBuffer, EventReceiver, EventSender, event_channel};
pub use types::{DiagnosticEvent, EventCategory, EventPayload, current_time_ns};
