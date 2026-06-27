//! `HostBuilder`: staged construction of a [`TorvynHost`].
//!
//! The builder accepts configuration, initializes all subsystems
//! in dependency order, and produces a ready-to-run [`TorvynHost`].
//!
//! # Initialization Order (per Doc 10, Section 3.4)
//! 1. Parse and validate configuration
//! 2. Initialize Wasm engine
//! 3. Initialize observability (tracing + metrics)
//! 4. Initialize resource manager
//! 5. Initialize security manager
//! 6. Create reactor coordinator
//! 7. Return configured host

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::info;

use torvyn_config::{load_pipeline, FlowDef, ObservabilityConfig, RuntimeConfig, SecurityConfig};
use torvyn_engine::{WasmtimeEngine, WasmtimeEngineConfig, WasmtimeInvoker};
use torvyn_reactor::{
    coordinator::ReactorCoordinator,
    events::{ReactorCommand, ReactorEvent},
    handle::ReactorHandle,
};
use torvyn_types::NoopEventSink;

use crate::error::{HostError, StartupError};
use crate::host::TorvynHost;

/// Default capacity of the reactor command channel.
///
/// Sized to absorb a burst of cold-path commands (flow creation,
/// cancellation) without backpressuring the host. The reactor processes
/// commands quickly so this rarely matters in practice.
const REACTOR_COMMAND_CHANNEL_CAPACITY: usize = 256;

/// Default capacity of the reactor event channel.
///
/// Sized for observability events. Events are produced by flow drivers
/// and consumed by observability subscribers; bounded to prevent unbounded
/// memory growth if no subscriber drains.
const REACTOR_EVENT_CHANNEL_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// HostConfig
// ---------------------------------------------------------------------------

/// Aggregated configuration for the host runtime.
///
/// Collected by the builder from parsed configuration files and
/// programmatic overrides. Passed to `TorvynHost` during construction.
///
/// # Invariants
/// - All fields are validated before the host is constructed.
/// - `shutdown_timeout` must be > 0.
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Runtime configuration (threads, memory, fuel).
    pub runtime: RuntimeConfig,

    /// Observability configuration (tracing, metrics).
    pub observability: ObservabilityConfig,

    /// Security configuration (capability policies).
    pub security: SecurityConfig,

    /// Engine configuration (Wasmtime settings).
    pub engine: WasmtimeEngineConfig,

    /// Maximum time allowed for graceful shutdown.
    /// Default: 30 seconds.
    pub shutdown_timeout: Duration,

    /// Path to the pipeline configuration file.
    /// If None, the host starts with no flows (programmatic API only).
    pub pipeline_config_path: Option<PathBuf>,
}

impl Default for HostConfig {
    /// # COLD PATH
    fn default() -> Self {
        Self {
            runtime: RuntimeConfig::default(),
            observability: ObservabilityConfig::default(),
            security: SecurityConfig::default(),
            engine: WasmtimeEngineConfig::default(),
            shutdown_timeout: Duration::from_secs(30),
            pipeline_config_path: None,
        }
    }
}

impl HostConfig {
    /// Validate all configuration fields. Returns a list of problems.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.shutdown_timeout.is_zero() {
            problems.push(
                "shutdown_timeout must be > 0. \
                 Set a positive duration (recommended: 30s)."
                    .into(),
            );
        }

        // Delegate to engine config validation
        let engine_problems = self.engine.validate();
        problems.extend(engine_problems);

        // CROSS-CRATE DEPENDENCY: RuntimeConfig, ObservabilityConfig,
        // SecurityConfig each have their own validate() methods.

        problems
    }
}

// ---------------------------------------------------------------------------
// HostBuilder
// ---------------------------------------------------------------------------

/// Staged builder for constructing a [`TorvynHost`].
///
/// # Design Decision (Doc 02, Section 10.3)
/// The builder pattern ensures all subsystems are initialized in the
/// correct dependency order and validates the configuration before
/// the host becomes usable. This prevents partial-initialization bugs
/// that are common in complex multi-subsystem applications.
///
/// # Examples
/// ```no_run
/// use torvyn_host::HostBuilder;
///
/// # async fn example() -> Result<(), torvyn_host::HostError> {
/// let host = HostBuilder::new()
///     .with_config_file("Torvyn.toml")
///     .build()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct HostBuilder {
    config: HostConfig,
    config_path: Option<PathBuf>,
    /// Flow definitions registered programmatically. Merged with any flows
    /// loaded from the configuration file during [`build()`](Self::build);
    /// programmatic definitions take precedence on name conflicts.
    flow_definitions: BTreeMap<String, FlowDef>,
}

impl HostBuilder {
    /// Create a new `HostBuilder` with default configuration.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: HostConfig::default(),
            config_path: None,
            flow_definitions: BTreeMap::new(),
        }
    }

    /// Register a flow definition programmatically, keyed by name.
    ///
    /// This is the programmatic equivalent of declaring a `[flow.*]` table in
    /// the configuration file: the host can later start it by name via
    /// [`TorvynHost::start_flow`](crate::TorvynHost::start_flow).
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_flow_definition(mut self, name: impl Into<String>, flow: FlowDef) -> Self {
        self.flow_definitions.insert(name.into(), flow);
        self
    }

    /// Load configuration from a TOML file path.
    ///
    /// The file is parsed and validated during [`build()`](Self::build).
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_config_file(mut self, path: impl AsRef<Path>) -> Self {
        self.config_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Override the runtime configuration programmatically.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_runtime_config(mut self, config: RuntimeConfig) -> Self {
        self.config.runtime = config;
        self
    }

    /// Override the engine configuration.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_engine_config(mut self, config: WasmtimeEngineConfig) -> Self {
        self.config.engine = config;
        self
    }

    /// Override the observability configuration.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_observability_config(mut self, config: ObservabilityConfig) -> Self {
        self.config.observability = config;
        self
    }

    /// Override the security configuration.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_security_config(mut self, config: SecurityConfig) -> Self {
        self.config.security = config;
        self
    }

    /// Set the pipeline config path (where flow definitions live).
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_pipeline_config(mut self, path: impl AsRef<Path>) -> Self {
        self.config.pipeline_config_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set the graceful shutdown timeout.
    ///
    /// # COLD PATH
    #[must_use]
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.config.shutdown_timeout = timeout;
        self
    }

    /// Build the host, initializing all subsystems.
    ///
    /// # COLD PATH
    ///
    /// # Steps
    /// 1. If a config file was specified, parse and merge it.
    /// 2. Validate the merged configuration.
    /// 3. Initialize subsystems in dependency order.
    /// 4. Return a ready `TorvynHost`.
    ///
    /// # Errors
    /// Returns `HostError::Config` if configuration is invalid.
    /// Returns `HostError::Startup` if any subsystem fails to initialize.
    // `async` is required even though no `.await` is used: `tokio::spawn`
    // requires an active Tokio runtime context, which an async fn provides.
    #[allow(clippy::unused_async)]
    pub async fn build(mut self) -> Result<TorvynHost, HostError> {
        // Step 1: Parse config file if specified
        if let Some(ref path) = self.config_path {
            info!(path = %path.display(), "Loading configuration");

            // LLI DEVIATION: load_pipeline takes &str, not &Path.
            // Also returns Vec<ConfigParseError> on failure instead of single error.
            let path_str = path.to_string_lossy();
            let parsed = load_pipeline(&path_str).map_err(|errors| {
                let messages: Vec<String> = errors.iter().map(|e| format!("{e}")).collect();
                HostError::config(format!(
                    "Failed to load '{}': {}",
                    path.display(),
                    messages.join("; ")
                ))
            })?;

            // CROSS-CRATE DEPENDENCY: load_pipeline returns PipelineDefinition.
            // Merge parsed config fields with programmatic overrides.
            if let Some(runtime) = parsed.runtime {
                self.config.runtime = runtime;
            }
            if let Some(observability) = parsed.observability {
                self.config.observability = observability;
            }
            if let Some(security) = parsed.security {
                self.config.security = security;
            }
            // Flows from the file are the base; programmatic definitions
            // registered via `with_flow_definition` override on conflict.
            for (name, flow) in parsed.flows {
                self.flow_definitions.entry(name).or_insert(flow);
            }
        }

        // Step 2: Validate
        let problems = self.config.validate();
        if !problems.is_empty() {
            return Err(HostError::config(format!(
                "Configuration validation failed:\n  - {}",
                problems.join("\n  - ")
            )));
        }

        info!("Configuration validated successfully");

        // Step 3: Initialize subsystems in dependency order
        // (Per Doc 02, Section 8.1 / Doc 10, Section 3.4)

        // 3a: Initialize Wasm engine
        let engine = Arc::new(
            WasmtimeEngine::new(self.config.engine.clone()).map_err(|e| {
                StartupError::EngineInit {
                    reason: e.to_string(),
                }
            })?,
        );
        info!("Wasm engine initialized");

        // 3b: Initialize the component invoker (Wasmtime backend).
        let invoker = Arc::new(WasmtimeInvoker::new());
        info!("Component invoker initialized");

        // 3c: Spawn the reactor coordinator and obtain its handle.
        let (cmd_tx, cmd_rx) = mpsc::channel::<ReactorCommand>(REACTOR_COMMAND_CHANNEL_CAPACITY);
        let (event_tx, _event_rx) = mpsc::channel::<ReactorEvent>(REACTOR_EVENT_CHANNEL_CAPACITY);

        // Phase 0 uses the no-op event sink; observability wiring will
        // replace this in a later phase. NoopEventSink is `Clone`, which
        // satisfies the coordinator's `E: EventSink + Clone + 'static`
        // bound and lets each spawned flow driver receive its own copy.
        let event_sink = Arc::new(NoopEventSink);
        let coordinator = ReactorCoordinator::new(
            cmd_rx,
            event_tx,
            Arc::clone(&invoker),
            Arc::clone(&event_sink),
            engine.resource_manager(),
        );
        let coordinator_join = tokio::spawn(coordinator.run());
        let reactor = ReactorHandle::new(cmd_tx);
        info!("Reactor coordinator spawned");

        // Step 4: Construct host with all subsystem handles wired in.
        Ok(TorvynHost::new(
            self.config,
            engine,
            invoker,
            reactor,
            Some(coordinator_join),
            self.flow_definitions,
        ))
    }
}

impl Default for HostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_config_default_is_valid() {
        let config = HostConfig::default();
        let problems = config.validate();
        assert!(
            problems.is_empty(),
            "default config should be valid: {problems:?}",
        );
    }

    #[test]
    fn test_host_config_zero_shutdown_timeout() {
        let config = HostConfig {
            shutdown_timeout: Duration::ZERO,
            ..HostConfig::default()
        };
        let problems = config.validate();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("shutdown_timeout"));
    }

    #[test]
    fn test_builder_default_creates_builder() {
        let builder = HostBuilder::new();
        assert!(builder.config_path.is_none());
        assert!(builder.config.pipeline_config_path.is_none());
    }

    #[test]
    fn test_builder_with_config_file() {
        let builder = HostBuilder::new().with_config_file("Torvyn.toml");
        assert_eq!(
            builder.config_path.as_deref(),
            Some(Path::new("Torvyn.toml"))
        );
    }

    #[test]
    fn test_builder_with_shutdown_timeout() {
        // 45s chosen as an arbitrary non-default, non-minute-multiple
        // value: it exercises the setter while avoiding clippy's
        // `duration_suboptimal_units` lint that fires on multiples of 60s.
        let builder = HostBuilder::new().with_shutdown_timeout(Duration::from_secs(45));
        assert_eq!(builder.config.shutdown_timeout, Duration::from_secs(45));
    }

    #[test]
    fn test_builder_chaining() {
        let builder = HostBuilder::new()
            .with_config_file("test.toml")
            .with_shutdown_timeout(Duration::from_secs(10))
            .with_engine_config(WasmtimeEngineConfig::default());

        assert!(builder.config_path.is_some());
        assert_eq!(builder.config.shutdown_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_builder_with_pipeline_config() {
        let builder = HostBuilder::new().with_pipeline_config("pipeline.toml");
        assert_eq!(
            builder.config.pipeline_config_path.as_deref(),
            Some(Path::new("pipeline.toml"))
        );
    }

    #[tokio::test]
    async fn test_builder_rejects_invalid_config() {
        let config = HostConfig {
            shutdown_timeout: Duration::ZERO,
            ..HostConfig::default()
        };

        let builder = HostBuilder {
            config,
            config_path: None,
            flow_definitions: BTreeMap::new(),
        };

        let result = builder.build().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(format!("{err}").contains("shutdown_timeout"));
    }

    #[tokio::test]
    async fn test_builder_default_produces_valid_host() {
        let result = HostBuilder::new().build().await;
        assert!(
            result.is_ok(),
            "default builder should succeed: {:?}",
            result.err()
        );
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
}
