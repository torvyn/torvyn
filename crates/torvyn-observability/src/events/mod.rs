//! Diagnostic event subsystem.

pub mod recorder;
pub mod types;

pub use recorder::{event_channel, EventBuffer, EventReceiver, EventSender};
pub use types::{current_time_ns, DiagnosticEvent, EventCategory, EventPayload};
