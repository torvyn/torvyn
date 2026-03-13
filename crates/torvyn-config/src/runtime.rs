//! Runtime configuration types.
//!
//! Shared between the component manifest and pipeline definition.
//! Default values are drawn from Doc 02 Section 6.4 with corrections
//! from Doc 10: C02-2 (queue depth 1024→64), C02-3 (`overflow_policy→backpressure_policy`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// SchedulingConfig
// ---------------------------------------------------------------------------

/// Scheduling policy configuration.
///
/// Default values per Doc 02 Section 6.4:
/// - `policy`: `"weighted-fair"`
/// - `default_priority`: `5`
///
/// # Examples
/// ```
/// use torvyn_config::SchedulingConfig;
///
/// let cfg = SchedulingConfig::default();
/// assert_eq!(cfg.policy, "weighted-fair");
/// assert_eq!(cfg.default_priority, 5);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulingConfig {
    /// Scheduling policy.
    /// Valid values: `"round-robin"`, `"weighted-fair"`, `"priority"`.
    /// Default: `"weighted-fair"`.
    #[serde(default = "default_scheduling_policy")]
    pub policy: String,

    /// Default priority for components that do not specify one.
    /// Range: 1 (lowest) to 10 (highest).
    /// Default: `5`.
    #[serde(default = "default_priority")]
    pub default_priority: u32,
}

fn default_scheduling_policy() -> String {
    "weighted-fair".to_owned()
}

fn default_priority() -> u32 {
    5
}

impl Default for SchedulingConfig {
    fn default() -> Self {
        Self {
            policy: default_scheduling_policy(),
            default_priority: default_priority(),
        }
    }
}

// ---------------------------------------------------------------------------
// BackpressureConfig
// ---------------------------------------------------------------------------

/// Backpressure configuration.
///
/// Default values per Doc 02 Section 6.4, with Doc 10 corrections:
/// - `default_queue_depth`: `64` (was 1024, changed per C02-2)
/// - `backpressure_policy`: `"block-producer"` (renamed from `overflow_policy` per C02-3)
///
/// # Examples
/// ```
/// use torvyn_config::BackpressureConfig;
///
/// let cfg = BackpressureConfig::default();
/// assert_eq!(cfg.default_queue_depth, 64);
/// assert_eq!(cfg.backpressure_policy, "block-producer");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackpressureConfig {
    /// Maximum elements in inter-component stream queue.
    /// Default: `64` (per Doc 10, C02-2).
    #[serde(default = "default_queue_depth")]
    pub default_queue_depth: usize,

    /// What happens when a queue is full.
    /// Valid values: `"block-producer"`, `"drop-oldest"`, `"drop-newest"`, `"error"`.
    /// Default: `"block-producer"` (renamed from `overflow_policy` per Doc 10, C02-3).
    #[serde(default = "default_backpressure_policy")]
    pub backpressure_policy: String,
}

fn default_queue_depth() -> usize {
    64 // Per Doc 10, C02-2: changed from 1024 to 64
}

fn default_backpressure_policy() -> String {
    "block-producer".to_owned()
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            default_queue_depth: default_queue_depth(),
            backpressure_policy: default_backpressure_policy(),
        }
    }
}

// ---------------------------------------------------------------------------
// RuntimeConfig
// ---------------------------------------------------------------------------

/// Runtime configuration from the `[runtime]` table.
///
/// Default values per Doc 02 Section 6.4.
///
/// # Examples
/// ```
/// use torvyn_config::RuntimeConfig;
///
/// let cfg = RuntimeConfig::default();
/// assert_eq!(cfg.max_memory_per_component, "16MiB");
/// assert_eq!(cfg.default_fuel_per_invocation, 1_000_000);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Number of Tokio worker threads.
    /// Default: number of physical CPU cores (represented as `0` = auto-detect).
    #[serde(default)]
    pub worker_threads: usize,

    /// Per-component linear memory cap.
    /// Accepts human-readable sizes: `"16MiB"`, `"64MiB"`, etc.
    /// Default: `"16MiB"`.
    #[serde(default = "default_max_memory")]
    pub max_memory_per_component: String,

    /// Fuel budget per component invocation.
    /// Default: `1_000_000`.
    #[serde(default = "default_fuel")]
    pub default_fuel_per_invocation: u64,

    /// Compilation cache directory.
    /// Default: `".torvyn/cache"`.
    #[serde(default = "default_cache_dir")]
    pub compilation_cache_dir: String,

    /// Scheduling configuration.
    #[serde(default)]
    pub scheduling: SchedulingConfig,

    /// Backpressure configuration.
    #[serde(default)]
    pub backpressure: BackpressureConfig,
}

fn default_max_memory() -> String {
    "16MiB".to_owned()
}

fn default_fuel() -> u64 {
    1_000_000
}

fn default_cache_dir() -> String {
    ".torvyn/cache".to_owned()
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            worker_threads: 0, // 0 means auto-detect
            max_memory_per_component: default_max_memory(),
            default_fuel_per_invocation: default_fuel(),
            compilation_cache_dir: default_cache_dir(),
            scheduling: SchedulingConfig::default(),
            backpressure: BackpressureConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// ObservabilityConfig
// ---------------------------------------------------------------------------

/// Observability configuration from the `[observability]` table.
///
/// Default values per Doc 02 Section 6.2.
///
/// # Examples
/// ```
/// use torvyn_config::ObservabilityConfig;
///
/// let cfg = ObservabilityConfig::default();
/// assert!(cfg.tracing_enabled);
/// assert_eq!(cfg.tracing_exporter, "stdout");
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Whether tracing is enabled.
    /// Default: `true`.
    #[serde(default = "default_true_obs")]
    pub tracing_enabled: bool,

    /// Tracing exporter backend.
    /// Valid values: `"otlp-grpc"`, `"otlp-http"`, `"stdout"`, `"none"`.
    /// Default: `"stdout"` (safe default for development).
    #[serde(default = "default_tracing_exporter")]
    pub tracing_exporter: String,

    /// OTLP endpoint URL (used when exporter is `"otlp-grpc"` or `"otlp-http"`).
    /// Default: `"http://localhost:4317"`.
    #[serde(default = "default_tracing_endpoint")]
    pub tracing_endpoint: String,

    /// Tracing sample rate (0.0 to 1.0).
    /// Default: `1.0` (full sampling in development).
    #[serde(default = "default_sample_rate")]
    pub tracing_sample_rate: f64,

    /// Whether metrics collection is enabled.
    /// Default: `true`.
    #[serde(default = "default_true_obs")]
    pub metrics_enabled: bool,

    /// Metrics exporter backend.
    /// Valid values: `"prometheus"`, `"otlp"`, `"none"`.
    /// Default: `"none"`.
    #[serde(default = "default_metrics_exporter")]
    pub metrics_exporter: String,

    /// Metrics endpoint (for prometheus scrape or OTLP push).
    /// Default: `"0.0.0.0:9090"`.
    #[serde(default = "default_metrics_endpoint")]
    pub metrics_endpoint: String,
}

fn default_true_obs() -> bool {
    true
}

fn default_tracing_exporter() -> String {
    "stdout".to_owned()
}

fn default_tracing_endpoint() -> String {
    "http://localhost:4317".to_owned()
}

fn default_sample_rate() -> f64 {
    1.0
}

fn default_metrics_exporter() -> String {
    "none".to_owned()
}

fn default_metrics_endpoint() -> String {
    "0.0.0.0:9090".to_owned()
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            tracing_enabled: true,
            tracing_exporter: default_tracing_exporter(),
            tracing_endpoint: default_tracing_endpoint(),
            tracing_sample_rate: default_sample_rate(),
            metrics_enabled: true,
            metrics_exporter: default_metrics_exporter(),
            metrics_endpoint: default_metrics_endpoint(),
        }
    }
}

impl Eq for ObservabilityConfig {}

// ---------------------------------------------------------------------------
// SecurityConfig
// ---------------------------------------------------------------------------

/// Security configuration from the `[security]` table.
///
/// Default: deny-all capability policy per Doc 02 Section 6.2 and
/// vision document Section 5.4.
///
/// # Examples
/// ```
/// use torvyn_config::SecurityConfig;
///
/// let cfg = SecurityConfig::default();
/// assert_eq!(cfg.default_capability_policy, "deny-all");
/// assert!(cfg.grants.is_empty());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Default capability policy.
    /// Valid values: `"deny-all"`, `"allow-all"` (dangerous, dev only).
    /// Default: `"deny-all"`.
    #[serde(default = "default_capability_policy")]
    pub default_capability_policy: String,

    /// Per-component capability grants.
    /// Key: component name. Value: grant specification.
    #[serde(default)]
    pub grants: BTreeMap<String, CapabilityGrant>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            default_capability_policy: default_capability_policy(),
            grants: BTreeMap::new(),
        }
    }
}

fn default_capability_policy() -> String {
    "deny-all".to_owned()
}

/// Capability grant for a specific component.
///
/// # Examples
/// ```
/// use torvyn_config::CapabilityGrant;
///
/// let grant = CapabilityGrant {
///     capabilities: vec!["filesystem:read:/data/*".into()],
/// };
/// assert_eq!(grant.capabilities.len(), 1);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    /// List of capability strings granted to this component.
    /// Format: `"<category>:<action>:<scope>"`.
    /// E.g., `"filesystem:read:/data/*"`, `"network:egress:*"`.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

// ---------------------------------------------------------------------------
// Utility: parse_memory_size
// ---------------------------------------------------------------------------

/// Parse a human-readable memory size string into bytes.
///
/// Supported suffixes: `B`, `KiB`, `MiB`, `GiB`.
/// Case-insensitive suffix matching.
///
/// # COLD PATH — called during config validation.
///
/// # Errors
///
/// Returns `Err` if the string is empty, has no numeric prefix, uses an
/// unknown suffix, or overflows `u64`.
///
/// # Examples
/// ```
/// use torvyn_config::parse_memory_size;
///
/// assert_eq!(parse_memory_size("16MiB"), Ok(16 * 1024 * 1024));
/// assert_eq!(parse_memory_size("1GiB"), Ok(1024 * 1024 * 1024));
/// assert_eq!(parse_memory_size("512KiB"), Ok(512 * 1024));
/// assert_eq!(parse_memory_size("1024B"), Ok(1024));
/// assert!(parse_memory_size("invalid").is_err());
/// ```
pub fn parse_memory_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty memory size string".to_owned());
    }

    // Find where the numeric part ends
    let num_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());

    if num_end == 0 {
        return Err(format!("no numeric value in '{s}'"));
    }

    let num: u64 = s[..num_end]
        .parse()
        .map_err(|e| format!("invalid number in '{s}': {e}"))?;

    let suffix = s[num_end..].trim();
    let multiplier = match suffix.to_ascii_lowercase().as_str() {
        "" | "b" => 1u64,
        "kib" | "k" => 1024,
        "mib" | "m" => 1024 * 1024,
        "gib" | "g" => 1024 * 1024 * 1024,
        other => {
            return Err(format!(
                "unknown memory size suffix '{other}'. Use B, KiB, MiB, or GiB."
            ))
        }
    };

    num.checked_mul(multiplier)
        .ok_or_else(|| format!("memory size overflow: {num} * {multiplier}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- SchedulingConfig ---

    #[test]
    fn test_scheduling_config_defaults() {
        let cfg = SchedulingConfig::default();
        assert_eq!(cfg.policy, "weighted-fair");
        assert_eq!(cfg.default_priority, 5);
    }

    // --- BackpressureConfig ---

    #[test]
    fn test_backpressure_config_defaults_match_doc10() {
        let cfg = BackpressureConfig::default();
        assert_eq!(cfg.default_queue_depth, 64);
        assert_eq!(cfg.backpressure_policy, "block-producer");
    }

    // --- RuntimeConfig ---

    #[test]
    fn test_runtime_config_defaults() {
        let cfg = RuntimeConfig::default();
        assert_eq!(cfg.worker_threads, 0); // auto-detect
        assert_eq!(cfg.max_memory_per_component, "16MiB");
        assert_eq!(cfg.default_fuel_per_invocation, 1_000_000);
        assert_eq!(cfg.compilation_cache_dir, ".torvyn/cache");
    }

    #[test]
    fn test_runtime_config_serde_round_trip() {
        let cfg = RuntimeConfig::default();
        let toml_str = toml::to_string(&cfg).unwrap();
        let reparsed: RuntimeConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg, reparsed);
    }

    // --- ObservabilityConfig ---

    #[test]
    fn test_observability_config_defaults() {
        let cfg = ObservabilityConfig::default();
        assert!(cfg.tracing_enabled);
        assert_eq!(cfg.tracing_exporter, "stdout");
        assert_eq!(cfg.tracing_sample_rate, 1.0);
        assert!(cfg.metrics_enabled);
        assert_eq!(cfg.metrics_exporter, "none");
    }

    // --- SecurityConfig ---

    #[test]
    fn test_security_config_defaults() {
        let cfg = SecurityConfig::default();
        assert_eq!(cfg.default_capability_policy, "deny-all");
        assert!(cfg.grants.is_empty());
    }

    #[test]
    fn test_security_config_with_grants() {
        let toml_str = r#"
default_capability_policy = "deny-all"

[grants.source-1]
capabilities = ["filesystem:read:/data/*"]

[grants.sink-1]
capabilities = ["network:egress:*"]
"#;
        let cfg: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.grants.len(), 2);
        assert_eq!(cfg.grants["source-1"].capabilities.len(), 1);
    }

    // --- parse_memory_size ---

    #[test]
    fn test_parse_memory_size_bytes() {
        assert_eq!(parse_memory_size("1024B"), Ok(1024));
        assert_eq!(parse_memory_size("1024"), Ok(1024));
    }

    #[test]
    fn test_parse_memory_size_kib() {
        assert_eq!(parse_memory_size("512KiB"), Ok(512 * 1024));
        assert_eq!(parse_memory_size("1K"), Ok(1024));
    }

    #[test]
    fn test_parse_memory_size_mib() {
        assert_eq!(parse_memory_size("16MiB"), Ok(16 * 1024 * 1024));
        assert_eq!(parse_memory_size("1M"), Ok(1024 * 1024));
    }

    #[test]
    fn test_parse_memory_size_gib() {
        assert_eq!(parse_memory_size("1GiB"), Ok(1024 * 1024 * 1024));
    }

    #[test]
    fn test_parse_memory_size_empty_is_error() {
        assert!(parse_memory_size("").is_err());
    }

    #[test]
    fn test_parse_memory_size_no_number_is_error() {
        assert!(parse_memory_size("MiB").is_err());
    }

    #[test]
    fn test_parse_memory_size_unknown_suffix_is_error() {
        assert!(parse_memory_size("100TiB").is_err());
    }

    #[test]
    fn test_parse_memory_size_case_insensitive() {
        assert_eq!(parse_memory_size("16mib"), Ok(16 * 1024 * 1024));
        assert_eq!(parse_memory_size("16MIB"), Ok(16 * 1024 * 1024));
    }
}
