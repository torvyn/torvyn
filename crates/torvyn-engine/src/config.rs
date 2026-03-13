//! Configuration types for the Torvyn Wasm engine.
//!
//! [`WasmtimeEngineConfig`] is the primary configuration struct that controls
//! how the Wasmtime engine compiles and executes WebAssembly components.

use std::path::PathBuf;

/// Compilation strategy for WebAssembly code.
///
/// # Examples
/// ```
/// use torvyn_engine::CompilationStrategy;
///
/// let strategy = CompilationStrategy::Cranelift;
/// assert_eq!(format!("{:?}", strategy), "Cranelift");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompilationStrategy {
    /// Cranelift — the default, mature optimizing compiler.
    #[default]
    Cranelift,
    /// Winch — the baseline (fast-compile, slower-execute) compiler.
    Winch,
}

/// Configuration for the Wasmtime-based Wasm engine.
///
/// Per Doc 02, Section 3.2: controls fuel, memory limits,
/// compilation strategy, SIMD, caching, and parallelism.
///
/// # Invariants
/// - `default_fuel` must be > 0 when `fuel_enabled` is true.
/// - `max_memory_bytes` must be > 0.
///
/// # Examples
/// ```
/// use torvyn_engine::WasmtimeEngineConfig;
///
/// let config = WasmtimeEngineConfig::default();
/// assert!(config.fuel_enabled);
/// assert_eq!(config.max_memory_bytes, 16 * 1024 * 1024);
/// ```
#[derive(Clone, Debug)]
pub struct WasmtimeEngineConfig {
    /// Enable fuel-based CPU budgeting.
    ///
    /// When enabled, each component invocation is given a fuel allocation.
    /// Components that exceed their budget are preempted.
    pub fuel_enabled: bool,

    /// Default fuel budget per component invocation.
    ///
    /// Per Doc 02, Section 3.2: default is 1,000,000.
    pub default_fuel: u64,

    /// Fuel yield interval for async cooperative preemption.
    ///
    /// When set, Wasmtime yields back to the async executor after
    /// consuming this many fuel units, enabling cooperative multitasking.
    /// A value of 0 disables yield-on-fuel.
    pub fuel_yield_interval: u64,

    /// Maximum linear memory per component instance (bytes).
    ///
    /// Per Doc 02, Section 3.2: default is 16 MiB.
    pub max_memory_bytes: usize,

    /// Maximum number of Wasm table elements per component instance.
    pub max_table_elements: u32,

    /// Maximum number of instances per store.
    pub max_instances: u32,

    /// Compilation strategy.
    pub strategy: CompilationStrategy,

    /// Enable Wasm SIMD instructions.
    pub simd_enabled: bool,

    /// Enable Wasm multi-memory proposal.
    pub multi_memory: bool,

    /// Compilation cache directory for serialized native code.
    ///
    /// When `Some`, compiled components are serialized to disk and
    /// deserialized on subsequent loads, avoiding recompilation.
    pub cache_dir: Option<PathBuf>,

    /// Number of parallel compilation threads.
    ///
    /// `None` uses Wasmtime's default (number of CPUs).
    pub compilation_threads: Option<usize>,

    /// Stack size for Wasm execution (bytes).
    ///
    /// Default: 1 MiB. Controls the maximum call stack depth.
    pub stack_size: usize,
}

impl Default for WasmtimeEngineConfig {
    /// Returns the default engine configuration.
    ///
    /// # COLD PATH — called once at startup.
    fn default() -> Self {
        Self {
            fuel_enabled: true,
            default_fuel: 1_000_000,
            fuel_yield_interval: 10_000,
            max_memory_bytes: 16 * 1024 * 1024, // 16 MiB
            max_table_elements: 10_000,
            max_instances: 10,
            strategy: CompilationStrategy::default(),
            simd_enabled: true,
            multi_memory: false,
            cache_dir: None,
            compilation_threads: None,
            stack_size: 1024 * 1024, // 1 MiB
        }
    }
}

impl WasmtimeEngineConfig {
    /// Validate the configuration, returning a list of problems.
    ///
    /// # COLD PATH — called once at startup.
    ///
    /// # Examples
    /// ```
    /// use torvyn_engine::WasmtimeEngineConfig;
    ///
    /// let config = WasmtimeEngineConfig::default();
    /// assert!(config.validate().is_empty());
    /// ```
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.fuel_enabled && self.default_fuel == 0 {
            problems.push(
                "default_fuel must be > 0 when fuel is enabled. \
                 Set a fuel budget (recommended: 1_000_000) or disable fuel."
                    .into(),
            );
        }

        if self.max_memory_bytes == 0 {
            problems.push("max_memory_bytes must be > 0.".into());
        }

        if self.stack_size < 64 * 1024 {
            problems.push(
                "stack_size is below 64 KiB — this will likely cause \
                 stack overflows in most components."
                    .into(),
            );
        }

        problems
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
        let config = WasmtimeEngineConfig::default();
        let problems = config.validate();
        assert!(
            problems.is_empty(),
            "default config should be valid: {:?}",
            problems
        );
    }

    #[test]
    fn test_default_config_values() {
        let config = WasmtimeEngineConfig::default();
        assert!(config.fuel_enabled);
        assert_eq!(config.default_fuel, 1_000_000);
        assert_eq!(config.max_memory_bytes, 16 * 1024 * 1024);
        assert_eq!(config.strategy, CompilationStrategy::Cranelift);
        assert!(config.simd_enabled);
    }

    #[test]
    fn test_validation_catches_zero_fuel() {
        let mut config = WasmtimeEngineConfig::default();
        config.default_fuel = 0;
        let problems = config.validate();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("default_fuel"));
    }

    #[test]
    fn test_validation_catches_zero_memory() {
        let mut config = WasmtimeEngineConfig::default();
        config.max_memory_bytes = 0;
        let problems = config.validate();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("max_memory_bytes"));
    }

    #[test]
    fn test_validation_catches_small_stack() {
        let mut config = WasmtimeEngineConfig::default();
        config.stack_size = 1024; // 1 KiB — too small
        let problems = config.validate();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("stack_size"));
    }

    #[test]
    fn test_validation_fuel_disabled_zero_ok() {
        let mut config = WasmtimeEngineConfig::default();
        config.fuel_enabled = false;
        config.default_fuel = 0;
        let problems = config.validate();
        assert!(
            problems.is_empty(),
            "zero fuel should be OK when fuel is disabled"
        );
    }

    #[test]
    fn test_compilation_strategy_default() {
        assert_eq!(CompilationStrategy::default(), CompilationStrategy::Cranelift);
    }
}
