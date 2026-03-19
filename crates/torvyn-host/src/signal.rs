//! OS signal handling for graceful shutdown.
//!
//! Listens for SIGINT (Ctrl-C) and SIGTERM and triggers the
//! host's graceful shutdown sequence.
//!
//! Behind the `signal` feature flag. Disabled for embedded/library use.

use tracing::info;

/// Wait for a shutdown signal (SIGINT or SIGTERM).
///
/// Returns when the first signal is received. The caller should
/// initiate graceful shutdown.
///
/// # COLD PATH — blocks until signal is received.
///
/// # Platform Support
/// - Unix: listens for SIGINT and SIGTERM.
/// - Non-Unix: listens for Ctrl-C only (SIGTERM is not available).
///
/// # Panics
/// Panics if the OS signal handler cannot be registered.
pub async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigint =
            signal(SignalKind::interrupt()).expect("failed to register SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");

        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl-C)");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to register Ctrl-C handler");
        info!("Received Ctrl-C");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Signal tests are inherently platform-specific and hard to unit-test
    // without actually sending signals. We test that the function compiles
    // and can be called, but we don't actually trigger a signal in CI.

    #[test]
    fn test_signal_module_compiles() {
        // Compile-time check: the function exists and has the right signature.
        let _: fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            || Box::pin(super::wait_for_shutdown_signal());
    }
}
