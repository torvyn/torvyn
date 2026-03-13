//! Configuration merging across multiple layers.
//!
//! Precedence order (highest first):
//! 1. CLI flags
//! 2. Environment variables (`TORVYN_*`)
//! 3. Project manifest (`Torvyn.toml`)
//! 4. Global user config
//! 5. Built-in defaults
//!
//! Per Doc 07 Section 4.4.

use crate::runtime::{
    BackpressureConfig, ObservabilityConfig, RuntimeConfig, SchedulingConfig, SecurityConfig,
};

/// Merge two `RuntimeConfig` values, where `overrides` takes precedence.
///
/// Fields in `overrides` that are at their default value are NOT applied,
/// to avoid clobbering explicit base values with defaults. A field in
/// `overrides` is considered "set" if it differs from `RuntimeConfig::default()`.
///
/// # COLD PATH — called once during config resolution.
///
/// # Examples
/// ```
/// use torvyn_config::{RuntimeConfig, merge_runtime_config};
///
/// let base = RuntimeConfig {
///     worker_threads: 4,
///     ..Default::default()
/// };
/// let overrides = RuntimeConfig {
///     worker_threads: 8,
///     ..Default::default()
/// };
/// let merged = merge_runtime_config(&base, &overrides);
/// assert_eq!(merged.worker_threads, 8);
/// ```
pub fn merge_runtime_config(base: &RuntimeConfig, overrides: &RuntimeConfig) -> RuntimeConfig {
    let defaults = RuntimeConfig::default();

    RuntimeConfig {
        worker_threads: if overrides.worker_threads != defaults.worker_threads {
            overrides.worker_threads
        } else {
            base.worker_threads
        },
        max_memory_per_component: if overrides.max_memory_per_component
            != defaults.max_memory_per_component
        {
            overrides.max_memory_per_component.clone()
        } else {
            base.max_memory_per_component.clone()
        },
        default_fuel_per_invocation: if overrides.default_fuel_per_invocation
            != defaults.default_fuel_per_invocation
        {
            overrides.default_fuel_per_invocation
        } else {
            base.default_fuel_per_invocation
        },
        compilation_cache_dir: if overrides.compilation_cache_dir
            != defaults.compilation_cache_dir
        {
            overrides.compilation_cache_dir.clone()
        } else {
            base.compilation_cache_dir.clone()
        },
        scheduling: merge_scheduling_config(&base.scheduling, &overrides.scheduling),
        backpressure: merge_backpressure_config(&base.backpressure, &overrides.backpressure),
    }
}

/// Merge two `SchedulingConfig` values.
///
/// # COLD PATH.
pub fn merge_scheduling_config(
    base: &SchedulingConfig,
    overrides: &SchedulingConfig,
) -> SchedulingConfig {
    let defaults = SchedulingConfig::default();

    SchedulingConfig {
        policy: if overrides.policy != defaults.policy {
            overrides.policy.clone()
        } else {
            base.policy.clone()
        },
        default_priority: if overrides.default_priority != defaults.default_priority {
            overrides.default_priority
        } else {
            base.default_priority
        },
    }
}

/// Merge two `BackpressureConfig` values.
///
/// # COLD PATH.
pub fn merge_backpressure_config(
    base: &BackpressureConfig,
    overrides: &BackpressureConfig,
) -> BackpressureConfig {
    let defaults = BackpressureConfig::default();

    BackpressureConfig {
        default_queue_depth: if overrides.default_queue_depth != defaults.default_queue_depth {
            overrides.default_queue_depth
        } else {
            base.default_queue_depth
        },
        backpressure_policy: if overrides.backpressure_policy != defaults.backpressure_policy {
            overrides.backpressure_policy.clone()
        } else {
            base.backpressure_policy.clone()
        },
    }
}

/// Merge two `ObservabilityConfig` values.
///
/// # COLD PATH.
pub fn merge_observability_config(
    base: &ObservabilityConfig,
    overrides: &ObservabilityConfig,
) -> ObservabilityConfig {
    let defaults = ObservabilityConfig::default();

    ObservabilityConfig {
        tracing_enabled: if overrides.tracing_enabled != defaults.tracing_enabled {
            overrides.tracing_enabled
        } else {
            base.tracing_enabled
        },
        tracing_exporter: if overrides.tracing_exporter != defaults.tracing_exporter {
            overrides.tracing_exporter.clone()
        } else {
            base.tracing_exporter.clone()
        },
        tracing_endpoint: if overrides.tracing_endpoint != defaults.tracing_endpoint {
            overrides.tracing_endpoint.clone()
        } else {
            base.tracing_endpoint.clone()
        },
        tracing_sample_rate: if (overrides.tracing_sample_rate - defaults.tracing_sample_rate)
            .abs()
            > f64::EPSILON
        {
            overrides.tracing_sample_rate
        } else {
            base.tracing_sample_rate
        },
        metrics_enabled: if overrides.metrics_enabled != defaults.metrics_enabled {
            overrides.metrics_enabled
        } else {
            base.metrics_enabled
        },
        metrics_exporter: if overrides.metrics_exporter != defaults.metrics_exporter {
            overrides.metrics_exporter.clone()
        } else {
            base.metrics_exporter.clone()
        },
        metrics_endpoint: if overrides.metrics_endpoint != defaults.metrics_endpoint {
            overrides.metrics_endpoint.clone()
        } else {
            base.metrics_endpoint.clone()
        },
    }
}

/// Merge two `SecurityConfig` values.
///
/// Grants are merged additively: the override's grants are added to or
/// replace the base's grants on a per-component basis.
///
/// # COLD PATH.
pub fn merge_security_config(
    base: &SecurityConfig,
    overrides: &SecurityConfig,
) -> SecurityConfig {
    let defaults = SecurityConfig::default();

    let mut grants = base.grants.clone();
    for (k, v) in &overrides.grants {
        grants.insert(k.clone(), v.clone());
    }

    SecurityConfig {
        default_capability_policy: if overrides.default_capability_policy
            != defaults.default_capability_policy
        {
            overrides.default_capability_policy.clone()
        } else {
            base.default_capability_policy.clone()
        },
        grants,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::CapabilityGrant;

    #[test]
    fn test_merge_runtime_override_takes_precedence() {
        let base = RuntimeConfig {
            worker_threads: 4,
            ..Default::default()
        };
        let overrides = RuntimeConfig {
            worker_threads: 8,
            ..Default::default()
        };
        let merged = merge_runtime_config(&base, &overrides);
        assert_eq!(merged.worker_threads, 8);
    }

    #[test]
    fn test_merge_runtime_base_preserved_when_override_is_default() {
        let base = RuntimeConfig {
            worker_threads: 4,
            default_fuel_per_invocation: 500_000,
            ..Default::default()
        };
        let overrides = RuntimeConfig::default();
        let merged = merge_runtime_config(&base, &overrides);
        assert_eq!(merged.worker_threads, 4);
        assert_eq!(merged.default_fuel_per_invocation, 500_000);
    }

    #[test]
    fn test_merge_scheduling_override() {
        let base = SchedulingConfig::default();
        let overrides = SchedulingConfig {
            policy: "priority".to_owned(),
            ..Default::default()
        };
        let merged = merge_scheduling_config(&base, &overrides);
        assert_eq!(merged.policy, "priority");
        assert_eq!(merged.default_priority, 5); // base preserved
    }

    #[test]
    fn test_merge_backpressure_override() {
        let base = BackpressureConfig {
            default_queue_depth: 128,
            ..Default::default()
        };
        let overrides = BackpressureConfig {
            backpressure_policy: "drop-newest".to_owned(),
            ..Default::default()
        };
        let merged = merge_backpressure_config(&base, &overrides);
        assert_eq!(merged.default_queue_depth, 128); // base preserved
        assert_eq!(merged.backpressure_policy, "drop-newest"); // override applied
    }

    #[test]
    fn test_merge_security_grants_additive() {
        let mut base_grants = std::collections::BTreeMap::new();
        base_grants.insert(
            "source".to_owned(),
            CapabilityGrant {
                capabilities: vec!["filesystem:read:*".into()],
            },
        );
        let base = SecurityConfig {
            grants: base_grants,
            ..Default::default()
        };

        let mut override_grants = std::collections::BTreeMap::new();
        override_grants.insert(
            "sink".to_owned(),
            CapabilityGrant {
                capabilities: vec!["network:egress:*".into()],
            },
        );
        let overrides = SecurityConfig {
            grants: override_grants,
            ..Default::default()
        };

        let merged = merge_security_config(&base, &overrides);
        assert_eq!(merged.grants.len(), 2);
        assert!(merged.grants.contains_key("source"));
        assert!(merged.grants.contains_key("sink"));
    }

    #[test]
    fn test_merge_security_grants_override_replaces_per_component() {
        let mut base_grants = std::collections::BTreeMap::new();
        base_grants.insert(
            "comp".to_owned(),
            CapabilityGrant {
                capabilities: vec!["a".into()],
            },
        );
        let base = SecurityConfig {
            grants: base_grants,
            ..Default::default()
        };

        let mut override_grants = std::collections::BTreeMap::new();
        override_grants.insert(
            "comp".to_owned(),
            CapabilityGrant {
                capabilities: vec!["b".into(), "c".into()],
            },
        );
        let overrides = SecurityConfig {
            grants: override_grants,
            ..Default::default()
        };

        let merged = merge_security_config(&base, &overrides);
        assert_eq!(merged.grants["comp"].capabilities, vec!["b", "c"]);
    }

    #[test]
    fn test_merge_observability_override() {
        let base = ObservabilityConfig::default();
        let overrides = ObservabilityConfig {
            tracing_exporter: "otlp-grpc".to_owned(),
            tracing_sample_rate: 0.01,
            ..Default::default()
        };
        let merged = merge_observability_config(&base, &overrides);
        assert_eq!(merged.tracing_exporter, "otlp-grpc");
        assert!((merged.tracing_sample_rate - 0.01).abs() < f64::EPSILON);
    }
}
