//! Integration tests for HostBuilder.

use std::time::Duration;

use torvyn_engine::WasmtimeEngineConfig;
use torvyn_host::{HostBuilder, HostStatus};

#[tokio::test]
async fn test_builder_default_produces_valid_host() {
    // Default builder should produce a host without error
    // (no config file, no flows — pure programmatic API mode).
    let result = HostBuilder::new().build().await;
    assert!(
        result.is_ok(),
        "default builder should succeed: {:?}",
        result.err()
    );

    let host = result.unwrap();
    assert_eq!(host.status(), HostStatus::Ready);
}

#[tokio::test]
async fn test_builder_missing_config_file_returns_error() {
    let result = HostBuilder::new()
        .with_config_file("/nonexistent/Torvyn.toml")
        .build()
        .await;

    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("E0900") || msg.contains("Failed to load"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn test_builder_rejects_zero_shutdown_timeout() {
    let result = HostBuilder::new()
        .with_shutdown_timeout(Duration::ZERO)
        .build()
        .await;

    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("shutdown_timeout"),
        "expected shutdown_timeout error: {msg}"
    );
}

#[tokio::test]
async fn test_builder_accepts_custom_engine_config() {
    let engine_config = WasmtimeEngineConfig::default();
    let result = HostBuilder::new()
        .with_engine_config(engine_config)
        .build()
        .await;

    assert!(
        result.is_ok(),
        "valid engine config should succeed: {:?}",
        result.err()
    );
}
