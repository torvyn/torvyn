//! Flow cancellation with reason tracking.
//!
//! Per Doc 04 §6.2 and DI-10: implements a cancellation token using
//! `Arc<AtomicBool>` + `tokio::sync::Notify` with an
//! `Arc<OnceLock<CancellationReason>>` to carry the reason.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;
use torvyn_types::{ComponentId, ProcessError};

// ---------------------------------------------------------------------------
// CancellationReason
// ---------------------------------------------------------------------------

/// The reason a flow was cancelled.
///
/// Per Doc 04 §6.1: cancellation sources include operator commands,
/// downstream errors, timeouts, resource exhaustion, and upstream completion.
#[derive(Clone, Debug)]
pub enum CancellationReason {
    /// Operator or API requested cancellation.
    OperatorRequest,
    /// Flow exceeded its wall-clock deadline.
    Timeout {
        /// The elapsed time when the timeout occurred.
        elapsed: Duration,
    },
    /// A component returned a fatal error.
    DownstreamError {
        /// The component that errored.
        component: ComponentId,
        /// The error that occurred.
        error: ProcessError,
    },
    /// The flow's resource budget was exhausted.
    ResourceExhaustion {
        /// Details about the exhaustion.
        detail: String,
    },
    /// The source signaled completion (graceful).
    SourceComplete,
}

impl std::fmt::Display for CancellationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CancellationReason::OperatorRequest => write!(f, "operator request"),
            CancellationReason::Timeout { elapsed } => {
                write!(f, "timeout after {elapsed:?}")
            }
            CancellationReason::DownstreamError { component, error } => {
                write!(f, "error in {component}: {error}")
            }
            CancellationReason::ResourceExhaustion { detail } => {
                write!(f, "resource exhaustion: {detail}")
            }
            CancellationReason::SourceComplete => write!(f, "source completed"),
        }
    }
}

// ---------------------------------------------------------------------------
// FlowCancellation
// ---------------------------------------------------------------------------

/// Shared inner state for the cancellation token.
struct CancellationInner {
    /// Whether the token has been cancelled.
    cancelled: AtomicBool,
    /// Notify waiters when cancellation occurs.
    notify: Notify,
    /// The reason for cancellation (set once).
    reason: OnceLock<CancellationReason>,
}

/// Cancellation token for a flow, with reason tracking.
///
/// Per Doc 04 §6.2: combines an `AtomicBool` + `Notify` with
/// `OnceLock<CancellationReason>`.
///
/// # Invariants
/// - `cancel()` can be called multiple times; only the first reason is recorded.
/// - `is_cancelled()` is safe to call from any task.
#[derive(Clone)]
pub struct FlowCancellation {
    inner: Arc<CancellationInner>,
}

impl FlowCancellation {
    /// Create a new `FlowCancellation`.
    ///
    /// # COLD PATH — called once per flow.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                reason: OnceLock::new(),
            }),
        }
    }

    /// Cancel the flow with the given reason.
    ///
    /// Only the first reason is recorded; subsequent calls are ignored.
    ///
    /// # WARM PATH — called at most once per flow (logically).
    pub fn cancel(&self, reason: CancellationReason) {
        let _ = self.inner.reason.set(reason);
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }

    /// Returns `true` if the flow has been cancelled.
    ///
    /// # HOT PATH — checked at the top of every flow driver iteration.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Returns the cancellation reason, if cancelled.
    pub fn reason(&self) -> Option<&CancellationReason> {
        self.inner.reason.get()
    }

    /// Returns a future that completes when the token is cancelled.
    /// Used in `select!` within the flow driver.
    pub async fn cancelled(&self) {
        // Fast path: already cancelled.
        if self.is_cancelled() {
            return;
        }
        // Slow path: wait for notification.
        loop {
            let notified = self.inner.notify.notified();
            // Re-check after registering the waiter to avoid races.
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

impl Default for FlowCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FlowCancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowCancellation")
            .field("is_cancelled", &self.is_cancelled())
            .field("reason", &self.inner.reason.get())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_cancellation_initial_state() {
        let cancel = FlowCancellation::new();
        assert!(!cancel.is_cancelled());
        assert!(cancel.reason().is_none());
    }

    #[test]
    fn test_flow_cancellation_cancel() {
        let cancel = FlowCancellation::new();
        cancel.cancel(CancellationReason::OperatorRequest);
        assert!(cancel.is_cancelled());
        assert!(matches!(
            cancel.reason(),
            Some(CancellationReason::OperatorRequest)
        ));
    }

    #[test]
    fn test_flow_cancellation_first_reason_wins() {
        let cancel = FlowCancellation::new();
        cancel.cancel(CancellationReason::OperatorRequest);
        cancel.cancel(CancellationReason::Timeout {
            elapsed: Duration::from_secs(30),
        });
        // First reason wins.
        assert!(matches!(
            cancel.reason(),
            Some(CancellationReason::OperatorRequest)
        ));
    }

    #[test]
    fn test_flow_cancellation_clone_shares_state() {
        let cancel1 = FlowCancellation::new();
        let cancel2 = cancel1.clone();
        cancel1.cancel(CancellationReason::OperatorRequest);
        assert!(cancel2.is_cancelled());
        assert!(cancel2.reason().is_some());
    }

    #[test]
    fn test_cancellation_reason_display() {
        assert!(format!("{}", CancellationReason::OperatorRequest).contains("operator"));
        assert!(
            format!(
                "{}",
                CancellationReason::Timeout {
                    elapsed: Duration::from_secs(5)
                }
            )
            .contains("timeout")
        );
        assert!(format!("{}", CancellationReason::SourceComplete).contains("source"));
    }

    #[tokio::test]
    async fn test_flow_cancellation_async_cancelled() {
        let cancel = FlowCancellation::new();
        let cancel2 = cancel.clone();

        let handle = tokio::spawn(async move {
            cancel2.cancelled().await;
            true
        });

        cancel.cancel(CancellationReason::OperatorRequest);
        let result = handle.await.unwrap();
        assert!(result);
    }
}
