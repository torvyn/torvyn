//! Sandbox configuration generation.
//!
//! Produces a complete `SandboxConfig` for each component instance by combining:
//! - Resolved capabilities -> WASI configuration + Torvyn capability guard
//! - Resource budget -> memory and handle limits
//! - CPU budget -> fuel and timeout limits
//!
//! The `SandboxConfigurator` trait is the public API consumed by the host runtime.

use crate::audit::{AuditEvent, AuditEventKind, AuditSeverity, AuditSinkHandle};
use crate::capability::Capability;
use crate::resolver::ResolvedCapabilitySet;
use crate::tenant::TenantId;
use std::sync::Arc;
use std::time::Duration;
use torvyn_types::ComponentId;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// CpuBudget
// ---------------------------------------------------------------------------

/// CPU budget configuration per component.
///
/// Per Doc 06 §5.1.2, fuel provides instruction-level metering and the timeout
/// acts as a wall-clock backstop.
///
/// # Invariants
/// - `fuel_per_invocation > 0`.
/// - `timeout` is a reasonable duration (not zero).
///
/// # Examples
/// ```
/// use torvyn_security::CpuBudget;
/// use std::time::Duration;
///
/// let budget = CpuBudget::new(1_000_000, Duration::from_millis(100), true);
/// assert_eq!(budget.fuel_per_invocation(), 1_000_000);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CpuBudget {
    /// Fuel units allocated per invocation.
    fuel_per_invocation: u64,
    /// Maximum wall-clock time per invocation (backstop).
    timeout: Duration,
    /// Whether to refuel between invocations automatically.
    auto_refuel: bool,
}

impl CpuBudget {
    /// Create a new `CpuBudget`.
    ///
    /// # COLD PATH — called during config parsing.
    pub fn new(fuel_per_invocation: u64, timeout: Duration, auto_refuel: bool) -> Self {
        Self {
            fuel_per_invocation,
            timeout,
            auto_refuel,
        }
    }

    /// Returns the fuel units per invocation.
    #[inline]
    pub fn fuel_per_invocation(&self) -> u64 {
        self.fuel_per_invocation
    }

    /// Returns the wall-clock timeout per invocation.
    #[inline]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Returns whether auto-refueling is enabled.
    #[inline]
    pub fn auto_refuel(&self) -> bool {
        self.auto_refuel
    }
}

impl Default for CpuBudget {
    fn default() -> Self {
        Self {
            fuel_per_invocation: 10_000_000, // 10M instructions
            timeout: Duration::from_secs(5),
            auto_refuel: true,
        }
    }
}

// ---------------------------------------------------------------------------
// ResourceBudget
// ---------------------------------------------------------------------------

/// Resource budget configuration per component.
///
/// Per Doc 06 §5.1.3, limits buffer memory, handle count, and allocation rate.
///
/// # Invariants
/// - `max_buffer_bytes > 0`.
/// - `max_handles > 0`.
///
/// # Examples
/// ```
/// use torvyn_security::ResourceBudget;
///
/// let budget = ResourceBudget::new(16 * 1024 * 1024, 256, Some(1000));
/// assert_eq!(budget.max_buffer_bytes(), 16 * 1024 * 1024);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceBudget {
    /// Maximum total bytes of buffer memory at any time.
    max_buffer_bytes: u64,
    /// Maximum number of resource handles simultaneously.
    max_handles: u32,
    /// Maximum allocations per second (rate limiting).
    max_allocations_per_second: Option<u32>,
}

impl ResourceBudget {
    /// Create a new `ResourceBudget`.
    ///
    /// # COLD PATH — called during config parsing.
    pub fn new(
        max_buffer_bytes: u64,
        max_handles: u32,
        max_allocations_per_second: Option<u32>,
    ) -> Self {
        Self {
            max_buffer_bytes,
            max_handles,
            max_allocations_per_second,
        }
    }

    /// Returns the max buffer bytes.
    #[inline]
    pub fn max_buffer_bytes(&self) -> u64 {
        self.max_buffer_bytes
    }

    /// Returns the max handle count.
    #[inline]
    pub fn max_handles(&self) -> u32 {
        self.max_handles
    }

    /// Returns the max allocations per second, if set.
    #[inline]
    pub fn max_allocations_per_second(&self) -> Option<u32> {
        self.max_allocations_per_second
    }
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_buffer_bytes: 64 * 1024 * 1024, // 64 MiB
            max_handles: 1024,
            max_allocations_per_second: None,
        }
    }
}

// ---------------------------------------------------------------------------
// WasiConfiguration
// ---------------------------------------------------------------------------

/// WASI-layer sandbox configuration derived from resolved capabilities.
///
/// This struct is consumed by the host runtime to configure Wasmtime's `WasiCtx`.
/// It is a Torvyn-side abstraction that insulates the security model from
/// Wasmtime API changes (Doc 06 §2.5).
///
/// # Invariants
/// - `preopened_dirs` only contains paths from granted filesystem capabilities.
/// - Boolean flags default to `false` (deny) unless explicitly granted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct WasiConfiguration {
    /// Directories to preopen for the component, with read/write flags.
    pub preopened_dirs: Vec<PreopenedDir>,
    /// Allow wall clock access.
    pub allow_wall_clock: bool,
    /// Allow monotonic clock access.
    pub allow_monotonic_clock: bool,
    /// Allow cryptographic random.
    pub allow_crypto_random: bool,
    /// Allow insecure random.
    pub allow_insecure_random: bool,
    /// Allow environment variable access.
    pub allow_environment: bool,
    /// Allow stdout.
    pub allow_stdout: bool,
    /// Allow stderr.
    pub allow_stderr: bool,
    /// Allow network access (TCP connect).
    pub allow_tcp_connect: bool,
    /// Allow network access (TCP listen).
    pub allow_tcp_listen: bool,
    /// Allow UDP access.
    pub allow_udp: bool,
    /// Allow HTTP outgoing.
    pub allow_http_outgoing: bool,
}

impl WasiConfiguration {
    /// Create a deny-all WASI configuration.
    pub fn deny_all() -> Self {
        Self {
            preopened_dirs: Vec::new(),
            allow_wall_clock: false,
            allow_monotonic_clock: false,
            allow_crypto_random: false,
            allow_insecure_random: false,
            allow_environment: false,
            allow_stdout: false,
            allow_stderr: false,
            allow_tcp_connect: false,
            allow_tcp_listen: false,
            allow_udp: false,
            allow_http_outgoing: false,
        }
    }

    /// Build a WASI configuration from a resolved capability set.
    ///
    /// # COLD PATH — called once per component instantiation.
    ///
    /// # Postconditions
    /// - Only capabilities present in `resolved` are enabled.
    /// - Filesystem preopens are created for each granted path scope.
    pub fn from_resolved(resolved: &ResolvedCapabilitySet) -> Self {
        let mut config = Self::deny_all();
        let mut preopened_dirs = Vec::new();

        for cap in resolved.capabilities() {
            match cap {
                Capability::FilesystemRead { path } => {
                    preopened_dirs.push(PreopenedDir {
                        host_path: path.root().to_owned(),
                        guest_path: path.root().to_owned(),
                        read: true,
                        write: false,
                    });
                }
                Capability::FilesystemWrite { path } => {
                    // Check if we already have a preopen for this path
                    if let Some(existing) = preopened_dirs
                        .iter_mut()
                        .find(|d| d.host_path == path.root())
                    {
                        existing.write = true;
                    } else {
                        preopened_dirs.push(PreopenedDir {
                            host_path: path.root().to_owned(),
                            guest_path: path.root().to_owned(),
                            read: false,
                            write: true,
                        });
                    }
                }
                Capability::TcpConnect { .. } => config.allow_tcp_connect = true,
                Capability::TcpListen { .. } => config.allow_tcp_listen = true,
                Capability::UdpAccess { .. } => config.allow_udp = true,
                Capability::HttpOutgoing { .. } => config.allow_http_outgoing = true,
                Capability::WallClock => config.allow_wall_clock = true,
                Capability::MonotonicClock => config.allow_monotonic_clock = true,
                Capability::CryptoRandom => config.allow_crypto_random = true,
                Capability::InsecureRandom => config.allow_insecure_random = true,
                Capability::Environment => config.allow_environment = true,
                Capability::Stdout => config.allow_stdout = true,
                Capability::Stderr => config.allow_stderr = true,
                // Torvyn-specific capabilities don't affect WasiCtx
                _ => {}
            }
        }

        config.preopened_dirs = preopened_dirs;
        config
    }
}

impl Default for WasiConfiguration {
    fn default() -> Self {
        Self::deny_all()
    }
}

/// A directory preopened for the component's WASI filesystem access.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PreopenedDir {
    /// The host filesystem path to preopen.
    pub host_path: String,
    /// The guest-visible path (usually same as host_path).
    pub guest_path: String,
    /// Allow read access.
    pub read: bool,
    /// Allow write access.
    pub write: bool,
}

// ---------------------------------------------------------------------------
// SandboxConfig
// ---------------------------------------------------------------------------

/// Complete sandbox configuration for a component instance.
///
/// Produced by the `SandboxConfigurator`, consumed by the host runtime
/// during component instantiation. This is the single struct that carries
/// all security-relevant configuration for a component.
///
/// Per Doc 06 §9.1 and Doc 10 §3.5.
///
/// # Invariants
/// - `resolved_capabilities`, `wasi_config`, `cpu_budget`, and `resource_budget`
///   are all consistent with each other.
/// - `wasi_config` was derived from `resolved_capabilities`.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// The component this config is for.
    pub component_id: ComponentId,
    /// The tenant this component belongs to.
    pub tenant_id: TenantId,
    /// The resolved capability set (shared via Arc for guard construction).
    pub resolved_capabilities: Arc<ResolvedCapabilitySet>,
    /// WASI-layer configuration derived from resolved capabilities.
    pub wasi_config: WasiConfiguration,
    /// CPU budget (fuel + timeout).
    pub cpu_budget: CpuBudget,
    /// Resource budget (memory + handles).
    pub resource_budget: ResourceBudget,
}

// ---------------------------------------------------------------------------
// SandboxConfigurator trait + default impl
// ---------------------------------------------------------------------------

/// Trait for producing a complete sandbox configuration for a component.
///
/// Consumed by the host runtime during component instantiation.
/// Per Doc 06 §9.2 and Doc 10 §3.6.
pub trait SandboxConfigurator: Send + Sync {
    /// Produce a `SandboxConfig` for a component given its declarations and operator grants.
    ///
    /// # COLD PATH — called once per component instantiation.
    ///
    /// # Errors
    /// Returns an error if capability resolution fails (missing required capabilities).
    #[allow(clippy::too_many_arguments)]
    fn configure(
        &self,
        component_id: ComponentId,
        tenant_id: TenantId,
        component_caps: &crate::manifest::ComponentCapabilities,
        operator_grants: &crate::manifest::OperatorGrants,
        resource_budget: ResourceBudget,
        cpu_budget: CpuBudget,
        audit_sink: AuditSinkHandle,
    ) -> Result<SandboxConfig, crate::error::SandboxError>;
}

/// Default sandbox configurator implementation.
///
/// Uses `DefaultCapabilityResolver` for resolution and `WasiConfiguration::from_resolved`
/// for WASI config generation.
pub struct DefaultSandboxConfigurator;

impl SandboxConfigurator for DefaultSandboxConfigurator {
    fn configure(
        &self,
        component_id: ComponentId,
        tenant_id: TenantId,
        component_caps: &crate::manifest::ComponentCapabilities,
        operator_grants: &crate::manifest::OperatorGrants,
        resource_budget: ResourceBudget,
        cpu_budget: CpuBudget,
        audit_sink: AuditSinkHandle,
    ) -> Result<SandboxConfig, crate::error::SandboxError> {
        // Resolve capabilities
        let result = crate::resolver::DefaultCapabilityResolver::resolve(
            component_caps,
            operator_grants,
        )
        .map_err(|errors| crate::error::SandboxError::CapabilityResolutionFailed {
            component_id,
            errors,
        })?;

        // Log warnings
        for warning in &result.warnings {
            audit_sink.record_sync(AuditEvent::new(
                AuditSeverity::Warning,
                Some(component_id),
                None,
                Some(tenant_id.clone()),
                AuditEventKind::CapabilityResolutionWarning {
                    detail: warning.to_string(),
                },
            ));
        }

        // Generate WASI configuration
        let wasi_config = WasiConfiguration::from_resolved(&result.resolved);

        // Log successful resolution
        let cap_strings: Vec<String> = result
            .resolved
            .capabilities()
            .iter()
            .map(|c| c.to_string())
            .collect();
        audit_sink.record_sync(AuditEvent::new(
            AuditSeverity::Info,
            Some(component_id),
            None,
            Some(tenant_id.clone()),
            AuditEventKind::ComponentInstantiated {
                resolved_capabilities: cap_strings,
            },
        ));

        let resolved = Arc::new(result.resolved);

        Ok(SandboxConfig {
            component_id,
            tenant_id,
            resolved_capabilities: resolved,
            wasi_config,
            cpu_budget,
            resource_budget,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Capability, NetScope, PathScope};
    use crate::manifest::{ComponentCapabilities, OperatorGrants};

    #[test]
    fn test_wasi_config_deny_all() {
        let config = WasiConfiguration::deny_all();
        assert!(config.preopened_dirs.is_empty());
        assert!(!config.allow_wall_clock);
        assert!(!config.allow_stdout);
    }

    #[test]
    fn test_wasi_config_from_resolved_clocks() {
        let resolved = ResolvedCapabilitySet::new(vec![
            Capability::WallClock,
            Capability::MonotonicClock,
        ]);
        let config = WasiConfiguration::from_resolved(&resolved);
        assert!(config.allow_wall_clock);
        assert!(config.allow_monotonic_clock);
        assert!(!config.allow_stdout);
        assert!(config.preopened_dirs.is_empty());
    }

    #[test]
    fn test_wasi_config_from_resolved_filesystem() {
        let resolved = ResolvedCapabilitySet::new(vec![
            Capability::FilesystemRead {
                path: PathScope::new("/data/input"),
            },
            Capability::FilesystemWrite {
                path: PathScope::new("/data/output"),
            },
        ]);
        let config = WasiConfiguration::from_resolved(&resolved);
        assert_eq!(config.preopened_dirs.len(), 2);
        assert!(config.preopened_dirs[0].read);
        assert!(!config.preopened_dirs[0].write);
        assert!(config.preopened_dirs[1].write);
    }

    #[test]
    fn test_wasi_config_filesystem_read_write_same_path() {
        let resolved = ResolvedCapabilitySet::new(vec![
            Capability::FilesystemRead {
                path: PathScope::new("/data"),
            },
            Capability::FilesystemWrite {
                path: PathScope::new("/data"),
            },
        ]);
        let config = WasiConfiguration::from_resolved(&resolved);
        assert_eq!(config.preopened_dirs.len(), 1);
        assert!(config.preopened_dirs[0].read);
        assert!(config.preopened_dirs[0].write);
    }

    #[test]
    fn test_wasi_config_network_flags() {
        let resolved = ResolvedCapabilitySet::new(vec![
            Capability::TcpConnect {
                scope: NetScope::unrestricted(),
            },
            Capability::HttpOutgoing {
                scope: NetScope::unrestricted(),
            },
        ]);
        let config = WasiConfiguration::from_resolved(&resolved);
        assert!(config.allow_tcp_connect);
        assert!(config.allow_http_outgoing);
        assert!(!config.allow_tcp_listen);
        assert!(!config.allow_udp);
    }

    #[test]
    fn test_sandbox_configurator_success() {
        let caps = ComponentCapabilities::new(
            vec![Capability::WallClock],
            vec![Capability::Stderr],
        );
        let grants = OperatorGrants::new(vec![Capability::WallClock, Capability::Stderr]);

        let configurator = DefaultSandboxConfigurator;
        let result = configurator.configure(
            ComponentId::new(1),
            TenantId::default_tenant(),
            &caps,
            &grants,
            ResourceBudget::default(),
            CpuBudget::default(),
            AuditSinkHandle::noop(),
        );
        assert!(result.is_ok());
        let config = result.unwrap();
        assert!(config.wasi_config.allow_wall_clock);
        assert!(config.wasi_config.allow_stderr);
    }

    #[test]
    fn test_sandbox_configurator_missing_required() {
        let caps = ComponentCapabilities::new(
            vec![Capability::WallClock, Capability::CryptoRandom],
            vec![],
        );
        let grants = OperatorGrants::new(vec![Capability::WallClock]);

        let configurator = DefaultSandboxConfigurator;
        let result = configurator.configure(
            ComponentId::new(1),
            TenantId::default_tenant(),
            &caps,
            &grants,
            ResourceBudget::default(),
            CpuBudget::default(),
            AuditSinkHandle::noop(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_cpu_budget_defaults() {
        let budget = CpuBudget::default();
        assert_eq!(budget.fuel_per_invocation(), 10_000_000);
        assert!(budget.auto_refuel());
    }

    #[test]
    fn test_resource_budget_defaults() {
        let budget = ResourceBudget::default();
        assert_eq!(budget.max_buffer_bytes(), 64 * 1024 * 1024);
        assert_eq!(budget.max_handles(), 1024);
    }
}
