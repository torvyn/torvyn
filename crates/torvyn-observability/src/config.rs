//! Observability configuration types.
//!
//! Controls observability level, tracing sample rates, export targets,
//! histogram bucket boundaries, and inspection API settings.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use torvyn_types::ObservabilityLevel;

/// Top-level observability configuration.
///
/// # Invariants
/// - `tracing.sample_rate` must be in `[0.0, 1.0]`.
/// - `tracing.ring_buffer_capacity` must be a power of two and >= 8.
/// - `metrics.histogram_buckets` must be non-empty and sorted ascending.
/// - `export.batch_size` must be >= 1.
///
/// # Examples
/// ```
/// use torvyn_observability::config::ObservabilityConfig;
///
/// let config = ObservabilityConfig::default();
/// assert_eq!(config.level, torvyn_types::ObservabilityLevel::Production);
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Active observability level.
    pub level: ObservabilityLevel,
    /// Tracing configuration.
    pub tracing: TracingConfig,
    /// Metrics configuration.
    pub metrics: MetricsConfig,
    /// Export configuration.
    pub export: ExportConfig,
    /// Inspection API configuration.
    pub inspection: InspectionConfig,
    /// Event channel capacity for diagnostic events.
    /// Default: 8192.
    pub event_channel_capacity: usize,
}

/// Tracing subsystem configuration.
///
/// Controls sampling rates, ring buffer sizes, and latency thresholds
/// that trigger trace promotion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TracingConfig {
    /// Fraction of flows that are fully traced (head-based sampling).
    /// Range: `[0.0, 1.0]`. Default: 0.01 (1%).
    pub sample_rate: f64,
    /// Whether to promote errored flows to full trace.
    /// Default: true.
    pub error_promote: bool,
    /// Latency threshold (ms) above which a flow is promoted to full trace.
    /// Default: 10 ms.
    pub latency_promote_threshold_ms: u64,
    /// Per-flow span ring buffer capacity. Must be power of two.
    /// Default: 64.
    pub ring_buffer_capacity: usize,
}

/// Metrics subsystem configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Whether metrics collection is enabled.
    /// Default: true.
    pub enabled: bool,
    /// Whether to serve Prometheus exposition format on the inspection API.
    /// Default: true.
    pub prometheus_enabled: bool,
    /// Custom histogram bucket boundaries (nanoseconds). If empty, uses defaults.
    pub histogram_buckets: Vec<u64>,
}

/// Export configuration for traces and metrics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportConfig {
    /// Export target type.
    pub target: ExportTarget,
    /// OTLP endpoint URL (for OTLP targets).
    pub endpoint: Option<String>,
    /// Batch size for export.
    /// Default: 512.
    pub batch_size: usize,
    /// Export interval.
    /// Default: 5 seconds.
    #[serde(with = "humantime_serde_compat")]
    pub interval: Duration,
}

/// Supported export targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportTarget {
    /// No export. Events stay in memory only.
    None,
    /// Structured JSON to stderr.
    Stdout,
    /// Append structured JSON to file.
    File(PathBuf),
    /// OTLP over HTTP/JSON.
    OtlpHttp,
    /// In-process channel (for `torvyn bench`).
    Channel,
}

/// Inspection API configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InspectionConfig {
    /// Whether the inspection API is enabled.
    /// Default: true.
    pub enabled: bool,
    /// Bind address. Default: Unix socket.
    pub bind: InspectionBind,
    /// Optional bearer token for authentication.
    pub auth_token: Option<String>,
}

/// Inspection API binding address.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectionBind {
    /// Unix domain socket at the given path.
    Unix(PathBuf),
    /// TCP socket on localhost at the given port.
    Tcp(u16),
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            level: ObservabilityLevel::Production,
            tracing: TracingConfig::default(),
            metrics: MetricsConfig::default(),
            export: ExportConfig::default(),
            inspection: InspectionConfig::default(),
            event_channel_capacity: 8192,
        }
    }
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            sample_rate: 0.01,
            error_promote: true,
            latency_promote_threshold_ms: 10,
            ring_buffer_capacity: 64,
        }
    }
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prometheus_enabled: true,
            histogram_buckets: Vec::new(), // empty => use LATENCY_BUCKETS_NS
        }
    }
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            target: ExportTarget::None,
            endpoint: None,
            batch_size: 512,
            interval: Duration::from_secs(5),
        }
    }
}

impl Default for InspectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: InspectionBind::Tcp(9091),
            auth_token: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Errors encountered when validating an [`ObservabilityConfig`].
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValidationError {
    /// Sample rate is outside [0.0, 1.0].
    InvalidSampleRate(f64),
    /// Ring buffer capacity is not a power of two or is less than 8.
    InvalidRingBufferCapacity(usize),
    /// Histogram buckets are not sorted ascending or are empty when custom.
    InvalidHistogramBuckets,
    /// Batch size is zero.
    ZeroBatchSize,
    /// Export interval is zero.
    ZeroExportInterval,
    /// OTLP target specified but no endpoint provided.
    MissingOtlpEndpoint,
}

impl std::fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSampleRate(r) => {
                write!(f, "sample_rate {r} is outside valid range [0.0, 1.0]")
            }
            Self::InvalidRingBufferCapacity(c) => {
                write!(
                    f,
                    "ring_buffer_capacity {c} must be a power of two and >= 8"
                )
            }
            Self::InvalidHistogramBuckets => {
                write!(
                    f,
                    "histogram_buckets must be non-empty and sorted ascending"
                )
            }
            Self::ZeroBatchSize => write!(f, "export batch_size must be >= 1"),
            Self::ZeroExportInterval => write!(f, "export interval must be > 0"),
            Self::MissingOtlpEndpoint => {
                write!(f, "OTLP export target requires an endpoint URL")
            }
        }
    }
}

impl std::error::Error for ConfigValidationError {}

impl ObservabilityConfig {
    /// Validate the configuration, returning all errors found.
    ///
    /// # COLD PATH — called once at startup.
    pub fn validate(&self) -> Result<(), Vec<ConfigValidationError>> {
        let mut errors = Vec::new();

        if !(0.0..=1.0).contains(&self.tracing.sample_rate) {
            errors.push(ConfigValidationError::InvalidSampleRate(
                self.tracing.sample_rate,
            ));
        }

        let cap = self.tracing.ring_buffer_capacity;
        if cap < 8 || !cap.is_power_of_two() {
            errors.push(ConfigValidationError::InvalidRingBufferCapacity(cap));
        }

        if !self.metrics.histogram_buckets.is_empty() {
            let sorted = self
                .metrics
                .histogram_buckets
                .windows(2)
                .all(|w| w[0] < w[1]);
            if !sorted {
                errors.push(ConfigValidationError::InvalidHistogramBuckets);
            }
        }

        if self.export.batch_size == 0 {
            errors.push(ConfigValidationError::ZeroBatchSize);
        }

        if self.export.interval.is_zero() {
            errors.push(ConfigValidationError::ZeroExportInterval);
        }

        if self.export.target == ExportTarget::OtlpHttp && self.export.endpoint.is_none() {
            errors.push(ConfigValidationError::MissingOtlpEndpoint);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Serde helper for Duration as milliseconds.
mod humantime_serde_compat {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ms = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(ms))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = ObservabilityConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_sample_rate_above_one() {
        let mut config = ObservabilityConfig::default();
        config.tracing.sample_rate = 1.5;
        let errs = config.validate().unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigValidationError::InvalidSampleRate(_))));
    }

    #[test]
    fn test_invalid_sample_rate_negative() {
        let mut config = ObservabilityConfig::default();
        config.tracing.sample_rate = -0.1;
        let errs = config.validate().unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigValidationError::InvalidSampleRate(_))));
    }

    #[test]
    fn test_invalid_ring_buffer_not_power_of_two() {
        let mut config = ObservabilityConfig::default();
        config.tracing.ring_buffer_capacity = 100;
        let errs = config.validate().unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigValidationError::InvalidRingBufferCapacity(_))));
    }

    #[test]
    fn test_invalid_ring_buffer_too_small() {
        let mut config = ObservabilityConfig::default();
        config.tracing.ring_buffer_capacity = 4;
        let errs = config.validate().unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigValidationError::InvalidRingBufferCapacity(_))));
    }

    #[test]
    fn test_invalid_histogram_buckets_unsorted() {
        let mut config = ObservabilityConfig::default();
        config.metrics.histogram_buckets = vec![100, 50, 200];
        let errs = config.validate().unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigValidationError::InvalidHistogramBuckets)));
    }

    #[test]
    fn test_zero_batch_size() {
        let mut config = ObservabilityConfig::default();
        config.export.batch_size = 0;
        let errs = config.validate().unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigValidationError::ZeroBatchSize)));
    }

    #[test]
    fn test_missing_otlp_endpoint() {
        let mut config = ObservabilityConfig::default();
        config.export.target = ExportTarget::OtlpHttp;
        config.export.endpoint = None;
        let errs = config.validate().unwrap_err();
        assert!(errs
            .iter()
            .any(|e| matches!(e, ConfigValidationError::MissingOtlpEndpoint)));
    }

    #[test]
    fn test_valid_custom_histogram_buckets() {
        let mut config = ObservabilityConfig::default();
        config.metrics.histogram_buckets = vec![100, 500, 1000, 5000];
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_export_target_serde_roundtrip() {
        let target = ExportTarget::OtlpHttp;
        let json = serde_json::to_string(&target).unwrap();
        let parsed: ExportTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, target);
    }

    #[test]
    fn test_config_validation_error_display() {
        let err = ConfigValidationError::InvalidSampleRate(2.0);
        let msg = format!("{err}");
        assert!(msg.contains("2"));
        assert!(msg.contains("sample_rate"));
    }

    #[test]
    fn test_multiple_validation_errors() {
        let mut config = ObservabilityConfig::default();
        config.tracing.sample_rate = 5.0;
        config.export.batch_size = 0;
        let errs = config.validate().unwrap_err();
        assert!(errs.len() >= 2);
    }
}
