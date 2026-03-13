//! Trace sampling strategies.
//!
//! Per HLI Doc 05 §2.3: three sampling strategies:
//! - Head-based: configurable fraction of flows are fully traced.
//! - Error-triggered: errored flows are promoted to full trace.
//! - Tail-latency: flows exceeding a latency threshold are promoted.

use crate::config::TracingConfig;

/// Sampling decision for a flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplingDecision {
    /// Do not export spans for this flow.
    Drop,
    /// Export spans (flow was selected by head sampling or promoted).
    Sample,
}

/// Sampler that makes per-flow sampling decisions.
///
/// # Thread-safety
/// Immutable after creation. Safe to share across tasks.
pub struct Sampler {
    /// Head-based sample rate [0.0, 1.0].
    sample_rate: f64,
    /// Whether to promote errored flows.
    error_promote: bool,
    /// Latency threshold in nanoseconds for promotion.
    latency_promote_threshold_ns: u64,
}

impl Sampler {
    /// Create a new sampler from configuration.
    ///
    /// # COLD PATH
    pub fn new(config: &TracingConfig) -> Self {
        Self {
            sample_rate: config.sample_rate,
            error_promote: config.error_promote,
            latency_promote_threshold_ns: config.latency_promote_threshold_ms * 1_000_000,
        }
    }

    /// Make a head-based sampling decision for a new flow.
    ///
    /// Uses a deterministic hash of the trace ID so sampling is reproducible.
    ///
    /// # COLD PATH — called once per flow.
    pub fn should_sample_head(&self, trace_id_bytes: &[u8; 16]) -> SamplingDecision {
        if self.sample_rate >= 1.0 {
            return SamplingDecision::Sample;
        }
        if self.sample_rate <= 0.0 {
            return SamplingDecision::Drop;
        }

        // Use the last 8 bytes of trace ID as a u64, then check if it falls
        // within the sample rate range.
        let hash = u64::from_le_bytes([
            trace_id_bytes[8],
            trace_id_bytes[9],
            trace_id_bytes[10],
            trace_id_bytes[11],
            trace_id_bytes[12],
            trace_id_bytes[13],
            trace_id_bytes[14],
            trace_id_bytes[15],
        ]);

        let threshold = (self.sample_rate * u64::MAX as f64) as u64;
        if hash <= threshold {
            SamplingDecision::Sample
        } else {
            SamplingDecision::Drop
        }
    }

    /// Check if a flow should be promoted due to an error.
    ///
    /// # WARM PATH — called when an error occurs.
    #[inline]
    pub fn should_promote_error(&self) -> bool {
        self.error_promote
    }

    /// Check if a flow should be promoted due to high latency.
    ///
    /// # WARM PATH — called at flow completion.
    #[inline]
    pub fn should_promote_latency(&self, latency_ns: u64) -> bool {
        self.latency_promote_threshold_ns > 0 && latency_ns > self.latency_promote_threshold_ns
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> TracingConfig {
        TracingConfig::default()
    }

    #[test]
    fn test_sampler_rate_zero_drops_all() {
        let mut config = default_config();
        config.sample_rate = 0.0;
        let sampler = Sampler::new(&config);

        for i in 0u8..100 {
            let trace_id = [i; 16];
            assert_eq!(
                sampler.should_sample_head(&trace_id),
                SamplingDecision::Drop
            );
        }
    }

    #[test]
    fn test_sampler_rate_one_samples_all() {
        let mut config = default_config();
        config.sample_rate = 1.0;
        let sampler = Sampler::new(&config);

        for i in 0u8..100 {
            let trace_id = [i; 16];
            assert_eq!(
                sampler.should_sample_head(&trace_id),
                SamplingDecision::Sample
            );
        }
    }

    #[test]
    fn test_sampler_partial_rate() {
        let mut config = default_config();
        config.sample_rate = 0.5;
        let sampler = Sampler::new(&config);

        let mut sampled = 0;
        let total = 10000;
        for i in 0u64..total {
            // Spread values across the full u64 range using a multiplicative hash,
            // otherwise small sequential i values all fall below the threshold.
            let val = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let trace_id = val.to_le_bytes();
            let mut full = [0u8; 16];
            full[8..].copy_from_slice(&trace_id);
            if sampler.should_sample_head(&full) == SamplingDecision::Sample {
                sampled += 1;
            }
        }

        // With 50% rate, expect roughly 5000 +/- generous margin.
        assert!(
            sampled > 3000 && sampled < 7000,
            "sampled {sampled} out of {total}"
        );
    }

    #[test]
    fn test_promote_error() {
        let config = default_config(); // error_promote = true
        let sampler = Sampler::new(&config);
        assert!(sampler.should_promote_error());
    }

    #[test]
    fn test_promote_error_disabled() {
        let mut config = default_config();
        config.error_promote = false;
        let sampler = Sampler::new(&config);
        assert!(!sampler.should_promote_error());
    }

    #[test]
    fn test_promote_latency() {
        let config = default_config(); // threshold = 10ms
        let sampler = Sampler::new(&config);

        assert!(!sampler.should_promote_latency(5_000_000)); // 5ms
        assert!(sampler.should_promote_latency(15_000_000)); // 15ms
    }

    #[test]
    fn test_promote_latency_zero_threshold() {
        let mut config = default_config();
        config.latency_promote_threshold_ms = 0;
        let sampler = Sampler::new(&config);

        assert!(!sampler.should_promote_latency(100_000_000));
    }
}
