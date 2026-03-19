//! Capability taxonomy for the Torvyn security model.
//!
//! Capabilities represent specific permissions that a component may request
//! or be granted. They are organized into WASI-aligned capabilities (filesystem,
//! network, clock, random, environment, stdio) and Torvyn-specific capabilities
//! (resource pools, stream operations, runtime inspection).
//!
//! # Design
//!
//! Capabilities are represented as a typed enum internally for exhaustive matching,
//! with well-defined string serialization for manifests and audit logs.
//! Per Doc 06 §2.2, this provides compiler-checked safety while remaining
//! serializable.
//!
//! # String Format
//!
//! The canonical string format is `"<domain>:<action>"` or `"<domain>:<action>:<scope>"`.
//! Examples: `"filesystem:read:/data"`, `"clock:wall"`, `"torvyn:resource-allocate"`.

use std::fmt;
use std::str::FromStr;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// PathScope
// ---------------------------------------------------------------------------

/// Scoped access to a filesystem subtree.
///
/// The component can access `root` and all descendant paths.
/// For example, `PathScope { root: "/data/input".into() }` grants
/// access to `/data/input/foo.txt`, `/data/input/subdir/bar.csv`, etc.
///
/// # Invariants
/// - `root` must be an absolute path (starts with `/`).
/// - `root` is normalized (no trailing `/` unless root itself).
///
/// # Examples
/// ```
/// use torvyn_security::PathScope;
///
/// let scope = PathScope::new("/data/input");
/// assert!(scope.contains_path("/data/input/foo.txt"));
/// assert!(!scope.contains_path("/data/output/bar.txt"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PathScope {
    /// The root directory. Access is granted to this path and all descendants.
    root: String,
}

impl PathScope {
    /// Create a new `PathScope`.
    ///
    /// The path is normalized: trailing slashes are removed unless the path is `/`.
    ///
    /// # COLD PATH — called during config parsing.
    ///
    /// # Preconditions
    /// - `root` should be an absolute path starting with `/`.
    ///
    /// # Postconditions
    /// - The stored root has no trailing `/` (unless root is `/`).
    pub fn new(root: impl Into<String>) -> Self {
        let mut root = root.into();
        // Normalize: remove trailing slash unless root is "/"
        while root.len() > 1 && root.ends_with('/') {
            root.pop();
        }
        Self { root }
    }

    /// Returns the root path.
    #[inline]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Check whether `path` is within this scope.
    ///
    /// A path is within scope if it equals the root or is a descendant
    /// (starts with `root` followed by `/`).
    ///
    /// # COLD PATH — called during scope intersection checks.
    ///
    /// # Examples
    /// ```
    /// use torvyn_security::PathScope;
    ///
    /// let scope = PathScope::new("/data");
    /// assert!(scope.contains_path("/data"));
    /// assert!(scope.contains_path("/data/sub/file.txt"));
    /// assert!(!scope.contains_path("/data2"));
    /// assert!(!scope.contains_path("/dat"));
    /// ```
    pub fn contains_path(&self, path: &str) -> bool {
        if path == self.root {
            return true;
        }
        // path must start with root + "/"
        if self.root == "/" {
            // Root scope "/" contains everything
            return true;
        }
        path.starts_with(&self.root) && path.as_bytes().get(self.root.len()) == Some(&b'/')
    }

    /// Check whether this scope fully contains another scope.
    ///
    /// `self` contains `other` if every path within `other` is also within `self`.
    /// This is true when `other.root` starts with `self.root/` or equals `self.root`.
    ///
    /// # COLD PATH — called during capability resolution.
    pub fn contains_scope(&self, other: &PathScope) -> bool {
        self.contains_path(&other.root)
    }

    /// Compute the intersection of two path scopes.
    ///
    /// Returns `Some(narrower_scope)` if the scopes overlap, `None` if disjoint.
    /// The intersection is the more specific (deeper) of the two scopes.
    ///
    /// # COLD PATH — called during capability resolution.
    pub fn intersect(&self, other: &PathScope) -> Option<PathScope> {
        if self.contains_scope(other) {
            Some(other.clone())
        } else if other.contains_scope(self) {
            Some(self.clone())
        } else {
            None
        }
    }
}

impl fmt::Display for PathScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.root)
    }
}

// ---------------------------------------------------------------------------
// PortRange
// ---------------------------------------------------------------------------

/// A contiguous range of TCP/UDP ports.
///
/// # Invariants
/// - `start <= end`.
///
/// # Examples
/// ```
/// use torvyn_security::PortRange;
///
/// let range = PortRange::new(80, 443).unwrap();
/// assert!(range.contains(80));
/// assert!(range.contains(443));
/// assert!(!range.contains(8080));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PortRange {
    start: u16,
    end: u16,
}

impl PortRange {
    /// Create a new port range.
    ///
    /// # Errors
    /// Returns `Err` if `start > end`.
    ///
    /// # COLD PATH — called during config parsing.
    pub fn new(start: u16, end: u16) -> Result<Self, String> {
        if start > end {
            return Err(format!("invalid port range: start ({start}) > end ({end})"));
        }
        Ok(Self { start, end })
    }

    /// Create a single-port range.
    #[inline]
    pub fn single(port: u16) -> Self {
        Self {
            start: port,
            end: port,
        }
    }

    /// Returns the start port.
    #[inline]
    pub fn start(&self) -> u16 {
        self.start
    }

    /// Returns the end port (inclusive).
    #[inline]
    pub fn end(&self) -> u16 {
        self.end
    }

    /// Check whether a port is within this range.
    ///
    /// # COLD PATH — called during network capability checks.
    #[inline]
    pub fn contains(&self, port: u16) -> bool {
        port >= self.start && port <= self.end
    }

    /// Check whether this range fully contains another range.
    #[inline]
    pub fn contains_range(&self, other: &PortRange) -> bool {
        self.start <= other.start && self.end >= other.end
    }
}

impl fmt::Display for PortRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

// ---------------------------------------------------------------------------
// NetScope
// ---------------------------------------------------------------------------

/// Scoped access to network endpoints.
///
/// Restricts network capabilities to specific hosts and port ranges.
/// Empty `hosts` means "all hosts" (if the capability is granted at all).
/// Empty `ports` means "all ports".
///
/// Host patterns support exact match and prefix wildcard (e.g., `"*.example.com"`).
///
/// # Invariants
/// - Host patterns are lowercase.
/// - An empty `NetScope` (no hosts, no ports) represents unrestricted access.
///
/// # Examples
/// ```
/// use torvyn_security::{NetScope, PortRange};
///
/// let scope = NetScope::new(
///     vec!["api.example.com".into()],
///     vec![PortRange::new(443, 443).unwrap()],
/// );
/// assert!(scope.matches_host("api.example.com"));
/// assert!(!scope.matches_host("evil.com"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NetScope {
    /// Allowed host patterns. Supports exact match and `*.domain` wildcard.
    /// Empty means all hosts are allowed.
    hosts: Vec<String>,
    /// Allowed port ranges. Empty means all ports are allowed.
    ports: Vec<PortRange>,
}

impl NetScope {
    /// Create a new `NetScope`.
    ///
    /// Host patterns are normalized to lowercase.
    ///
    /// # COLD PATH — called during config parsing.
    pub fn new(hosts: Vec<String>, ports: Vec<PortRange>) -> Self {
        let hosts = hosts.into_iter().map(|h| h.to_lowercase()).collect();
        Self { hosts, ports }
    }

    /// Create an unrestricted scope (all hosts, all ports).
    pub fn unrestricted() -> Self {
        Self {
            hosts: Vec::new(),
            ports: Vec::new(),
        }
    }

    /// Returns `true` if this scope has no restrictions.
    pub fn is_unrestricted(&self) -> bool {
        self.hosts.is_empty() && self.ports.is_empty()
    }

    /// Returns the host patterns.
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    /// Returns the port ranges.
    pub fn ports(&self) -> &[PortRange] {
        &self.ports
    }

    /// Check whether a hostname matches this scope's host patterns.
    ///
    /// # COLD PATH — called during network capability enforcement.
    pub fn matches_host(&self, hostname: &str) -> bool {
        if self.hosts.is_empty() {
            return true; // unrestricted
        }
        let hostname_lower = hostname.to_lowercase();
        self.hosts.iter().any(|pattern| {
            if let Some(suffix) = pattern.strip_prefix("*.") {
                // Wildcard: *.example.com matches sub.example.com
                hostname_lower == suffix || hostname_lower.ends_with(&format!(".{suffix}"))
            } else {
                hostname_lower == *pattern
            }
        })
    }

    /// Check whether a port matches this scope's port ranges.
    ///
    /// # COLD PATH — called during network capability enforcement.
    pub fn matches_port(&self, port: u16) -> bool {
        if self.ports.is_empty() {
            return true; // unrestricted
        }
        self.ports.iter().any(|r| r.contains(port))
    }

    /// Check whether this scope fully contains another scope.
    ///
    /// `self` contains `other` if every endpoint allowed by `other` is also
    /// allowed by `self`.
    ///
    /// # COLD PATH — called during capability resolution.
    pub fn contains_scope(&self, other: &NetScope) -> bool {
        // If self is unrestricted, it contains everything
        if self.is_unrestricted() {
            return true;
        }
        // If other is unrestricted but self is not, self does not contain other
        if other.is_unrestricted() {
            return false;
        }

        // Every host in other must be matched by self
        let hosts_ok = other.hosts.iter().all(|oh| {
            self.hosts.iter().any(|sh| {
                if sh == oh {
                    true
                } else if let Some(suffix) = sh.strip_prefix("*.") {
                    // self wildcard covers other exact or wildcard
                    if let Some(other_suffix) = oh.strip_prefix("*.") {
                        other_suffix == suffix || other_suffix.ends_with(&format!(".{suffix}"))
                    } else {
                        oh == suffix || oh.ends_with(&format!(".{suffix}"))
                    }
                } else {
                    false
                }
            })
        });

        // Every port range in other must be covered by some range in self
        let ports_ok = if self.ports.is_empty() {
            true // self allows all ports
        } else if other.ports.is_empty() {
            false // other allows all ports, self does not
        } else {
            other
                .ports
                .iter()
                .all(|op| self.ports.iter().any(|sp| sp.contains_range(op)))
        };

        hosts_ok && ports_ok
    }

    /// Compute the intersection of two net scopes.
    ///
    /// Returns `Some(scope)` if the scopes overlap, `None` if completely disjoint.
    /// The intersection is the more restrictive of the two.
    ///
    /// # COLD PATH — called during capability resolution.
    pub fn intersect(&self, other: &NetScope) -> Option<NetScope> {
        if self.contains_scope(other) {
            Some(other.clone())
        } else if other.contains_scope(self) {
            Some(self.clone())
        } else {
            // Partial overlap: for Phase 0 simplicity, treat as disjoint.
            None
        }
    }
}

impl fmt::Display for NetScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unrestricted() {
            write!(f, "*:*")
        } else {
            let hosts = if self.hosts.is_empty() {
                "*".to_owned()
            } else {
                self.hosts.join(",")
            };
            let ports = if self.ports.is_empty() {
                "*".to_owned()
            } else {
                self.ports
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            write!(f, "{hosts}:{ports}")
        }
    }
}

// ---------------------------------------------------------------------------
// PoolScope
// ---------------------------------------------------------------------------

/// Scoped access to a named resource pool.
///
/// # Invariants
/// - `pool_name` is non-empty.
///
/// # Examples
/// ```
/// use torvyn_security::PoolScope;
///
/// let scope = PoolScope::new("default");
/// assert_eq!(scope.pool_name(), "default");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PoolScope {
    pool_name: String,
}

impl PoolScope {
    /// Create a new `PoolScope`.
    ///
    /// # COLD PATH — called during config parsing.
    pub fn new(pool_name: impl Into<String>) -> Self {
        Self {
            pool_name: pool_name.into(),
        }
    }

    /// Returns the pool name.
    #[inline]
    pub fn pool_name(&self) -> &str {
        &self.pool_name
    }
}

impl fmt::Display for PoolScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.pool_name)
    }
}

// ---------------------------------------------------------------------------
// Capability
// ---------------------------------------------------------------------------

/// A single capability that a component may request or be granted.
///
/// Per Doc 06 §2.1, capabilities are organized into WASI-aligned and
/// Torvyn-specific domains. Each variant maps to a specific WASI interface
/// group or a Torvyn host function set.
///
/// # String Serialization
///
/// Each variant has a canonical string form used in manifest files:
///
/// | Variant | String Form |
/// |---------|-------------|
/// | `FilesystemRead` | `"filesystem:read:<path>"` |
/// | `FilesystemWrite` | `"filesystem:write:<path>"` |
/// | `TcpConnect` | `"network:tcp-connect[:<scope>]"` |
/// | `TcpListen` | `"network:tcp-listen[:<scope>]"` |
/// | `UdpAccess` | `"network:udp[:<scope>]"` |
/// | `HttpOutgoing` | `"network:http-outgoing[:<scope>]"` |
/// | `WallClock` | `"clock:wall"` |
/// | `MonotonicClock` | `"clock:monotonic"` |
/// | `CryptoRandom` | `"random:crypto"` |
/// | `InsecureRandom` | `"random:insecure"` |
/// | `Environment` | `"environment:read"` |
/// | `Stdout` | `"stdio:stdout"` |
/// | `Stderr` | `"stdio:stderr"` |
/// | `ResourceAllocate` | `"torvyn:resource-allocate[:<pool>]"` |
/// | `PoolAccess` | `"torvyn:pool-access:<pool>"` |
/// | `EmitBackpressure` | `"torvyn:emit-backpressure"` |
/// | `InspectFlowMeta` | `"torvyn:inspect-flow-meta"` |
/// | `RuntimeInspect` | `"torvyn:runtime-inspect"` |
/// | `CustomMetrics` | `"torvyn:custom-metrics"` |
///
/// # Examples
/// ```
/// use torvyn_security::{Capability, PathScope};
///
/// let cap = Capability::FilesystemRead { path: PathScope::new("/data") };
/// assert_eq!(cap.to_string(), "filesystem:read:/data");
///
/// let parsed: Capability = "filesystem:read:/data".parse().unwrap();
/// assert_eq!(cap, parsed);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Capability {
    // --- WASI Filesystem ---
    /// Read files from a specified directory subtree.
    /// Maps to `wasi:filesystem/read`.
    FilesystemRead {
        /// The filesystem path scope.
        path: PathScope,
    },
    /// Write files to a specified directory subtree.
    /// Maps to `wasi:filesystem/write`.
    FilesystemWrite {
        /// The filesystem path scope.
        path: PathScope,
    },

    // --- WASI Network ---
    /// Initiate outbound TCP connections.
    /// Maps to `wasi:sockets/tcp-connect`.
    TcpConnect {
        /// The network scope.
        scope: NetScope,
    },
    /// Listen for inbound TCP connections.
    /// Maps to `wasi:sockets/tcp-listen`.
    TcpListen {
        /// The network scope.
        scope: NetScope,
    },
    /// Send and receive UDP datagrams.
    /// Maps to `wasi:sockets/udp`.
    UdpAccess {
        /// The network scope.
        scope: NetScope,
    },
    /// Make outbound HTTP requests.
    /// Maps to `wasi:http/outgoing`.
    HttpOutgoing {
        /// The network scope.
        scope: NetScope,
    },

    // --- WASI Clocks ---
    /// Read the system wall clock. Maps to `wasi:clocks/wall-clock`.
    WallClock,
    /// Read the monotonic clock. Maps to `wasi:clocks/monotonic-clock`.
    MonotonicClock,

    // --- WASI Random ---
    /// Access cryptographic random bytes. Maps to `wasi:random/random`.
    CryptoRandom,
    /// Access non-cryptographic random bytes. Maps to `wasi:random/insecure`.
    InsecureRandom,

    // --- WASI Environment & Stdio ---
    /// Read environment variables. Maps to `wasi:cli/environment`.
    Environment,
    /// Write to standard output. Maps to `wasi:cli/stdout`.
    Stdout,
    /// Write to standard error. Maps to `wasi:cli/stderr`.
    Stderr,

    // --- Torvyn Resource Pools ---
    /// Allocate new buffers from the host buffer pool.
    /// Maps to `torvyn:resources/allocate`.
    ResourceAllocate {
        /// Optional pool scope restriction.
        pool: Option<PoolScope>,
    },
    /// Access a named resource pool.
    /// Maps to `torvyn:resources/pool-access`.
    PoolAccess {
        /// The pool scope.
        pool: PoolScope,
    },

    // --- Torvyn Stream ---
    /// Emit backpressure signals to the reactor.
    /// Maps to `torvyn:stream/emit-backpressure`.
    EmitBackpressure,
    /// Read flow metadata beyond what is in the stream element.
    /// Maps to `torvyn:stream/inspect-meta`.
    InspectFlowMeta,

    // --- Torvyn Runtime ---
    /// Query runtime diagnostic information.
    /// Maps to `torvyn:runtime/inspect`.
    RuntimeInspect,
    /// Emit custom metrics to the observability system.
    /// Maps to `torvyn:runtime/metrics`.
    CustomMetrics,
}

impl Capability {
    /// Returns the capability domain as a static string.
    ///
    /// # HOT PATH — used for audit event categorization.
    #[inline]
    pub fn domain(&self) -> &'static str {
        match self {
            Capability::FilesystemRead { .. } | Capability::FilesystemWrite { .. } => "filesystem",
            Capability::TcpConnect { .. }
            | Capability::TcpListen { .. }
            | Capability::UdpAccess { .. }
            | Capability::HttpOutgoing { .. } => "network",
            Capability::WallClock | Capability::MonotonicClock => "clock",
            Capability::CryptoRandom | Capability::InsecureRandom => "random",
            Capability::Environment => "environment",
            Capability::Stdout | Capability::Stderr => "stdio",
            Capability::ResourceAllocate { .. } | Capability::PoolAccess { .. } => {
                "torvyn:resources"
            }
            Capability::EmitBackpressure | Capability::InspectFlowMeta => "torvyn:stream",
            Capability::RuntimeInspect | Capability::CustomMetrics => "torvyn:runtime",
        }
    }

    /// Returns `true` if this is a WASI-aligned capability (enforced by WasiCtx).
    ///
    /// # COLD PATH — called during sandbox configuration.
    pub fn is_wasi(&self) -> bool {
        matches!(
            self,
            Capability::FilesystemRead { .. }
                | Capability::FilesystemWrite { .. }
                | Capability::TcpConnect { .. }
                | Capability::TcpListen { .. }
                | Capability::UdpAccess { .. }
                | Capability::HttpOutgoing { .. }
                | Capability::WallClock
                | Capability::MonotonicClock
                | Capability::CryptoRandom
                | Capability::InsecureRandom
                | Capability::Environment
                | Capability::Stdout
                | Capability::Stderr
        )
    }

    /// Returns `true` if this capability is on the hot path (checked per element).
    ///
    /// Hot-path capabilities are: ResourceAllocate, EmitBackpressure, CustomMetrics,
    /// InspectFlowMeta. These use the pre-computed bitmask in `HotPathCapabilities`
    /// instead of the full `CapabilityGuard::check` path.
    ///
    /// # COLD PATH — called during HotPathCapabilities construction.
    pub fn is_hot_path(&self) -> bool {
        matches!(
            self,
            Capability::ResourceAllocate { .. }
                | Capability::EmitBackpressure
                | Capability::CustomMetrics
                | Capability::InspectFlowMeta
        )
    }

    /// Check whether this capability (as a grant) satisfies a request.
    ///
    /// A grant satisfies a request if they are the same capability kind and
    /// the grant's scope contains the request's scope (or is unrestricted).
    ///
    /// # COLD PATH — called during capability resolution.
    ///
    /// # Postconditions
    /// - Returns `true` if `self` (grant) covers `request`.
    pub fn satisfies(&self, request: &Capability) -> bool {
        match (self, request) {
            (
                Capability::FilesystemRead { path: grant },
                Capability::FilesystemRead { path: req },
            ) => grant.contains_scope(req),
            (
                Capability::FilesystemWrite { path: grant },
                Capability::FilesystemWrite { path: req },
            ) => grant.contains_scope(req),
            (Capability::TcpConnect { scope: grant }, Capability::TcpConnect { scope: req }) => {
                grant.contains_scope(req)
            }
            (Capability::TcpListen { scope: grant }, Capability::TcpListen { scope: req }) => {
                grant.contains_scope(req)
            }
            (Capability::UdpAccess { scope: grant }, Capability::UdpAccess { scope: req }) => {
                grant.contains_scope(req)
            }
            (
                Capability::HttpOutgoing { scope: grant },
                Capability::HttpOutgoing { scope: req },
            ) => grant.contains_scope(req),
            // Unscoped capabilities: exact kind match
            (Capability::WallClock, Capability::WallClock) => true,
            (Capability::MonotonicClock, Capability::MonotonicClock) => true,
            (Capability::CryptoRandom, Capability::CryptoRandom) => true,
            (Capability::InsecureRandom, Capability::InsecureRandom) => true,
            (Capability::Environment, Capability::Environment) => true,
            (Capability::Stdout, Capability::Stdout) => true,
            (Capability::Stderr, Capability::Stderr) => true,
            // Resource pool: unscoped grant satisfies any pool request
            (
                Capability::ResourceAllocate { pool: grant_pool },
                Capability::ResourceAllocate { pool: req_pool },
            ) => match (grant_pool, req_pool) {
                (None, _) => true,        // unscoped grant covers anything
                (Some(_), None) => false, // scoped grant doesn't cover unscoped request
                (Some(g), Some(r)) => g.pool_name() == r.pool_name(),
            },
            (
                Capability::PoolAccess { pool: grant_pool },
                Capability::PoolAccess { pool: req_pool },
            ) => grant_pool.pool_name() == req_pool.pool_name(),
            (Capability::EmitBackpressure, Capability::EmitBackpressure) => true,
            (Capability::InspectFlowMeta, Capability::InspectFlowMeta) => true,
            (Capability::RuntimeInspect, Capability::RuntimeInspect) => true,
            (Capability::CustomMetrics, Capability::CustomMetrics) => true,
            _ => false, // Different capability kinds never satisfy each other
        }
    }

    /// Compute the effective capability from the intersection of a grant and a request.
    ///
    /// Returns `Some(effective)` if the grant satisfies the request (the effective
    /// capability is the most restrictive combination). Returns `None` if disjoint.
    ///
    /// # COLD PATH — called during capability resolution.
    pub fn intersect(&self, request: &Capability) -> Option<Capability> {
        match (self, request) {
            (
                Capability::FilesystemRead { path: grant },
                Capability::FilesystemRead { path: req },
            ) => {
                if grant.contains_scope(req) {
                    Some(Capability::FilesystemRead { path: req.clone() })
                } else {
                    None
                }
            }
            (
                Capability::FilesystemWrite { path: grant },
                Capability::FilesystemWrite { path: req },
            ) => {
                if grant.contains_scope(req) {
                    Some(Capability::FilesystemWrite { path: req.clone() })
                } else {
                    None
                }
            }
            (Capability::TcpConnect { scope: grant }, Capability::TcpConnect { scope: req }) => {
                if grant.contains_scope(req) {
                    Some(Capability::TcpConnect { scope: req.clone() })
                } else {
                    None
                }
            }
            (Capability::TcpListen { scope: grant }, Capability::TcpListen { scope: req }) => {
                if grant.contains_scope(req) {
                    Some(Capability::TcpListen { scope: req.clone() })
                } else {
                    None
                }
            }
            (Capability::UdpAccess { scope: grant }, Capability::UdpAccess { scope: req }) => {
                if grant.contains_scope(req) {
                    Some(Capability::UdpAccess { scope: req.clone() })
                } else {
                    None
                }
            }
            (
                Capability::HttpOutgoing { scope: grant },
                Capability::HttpOutgoing { scope: req },
            ) => {
                if grant.contains_scope(req) {
                    Some(Capability::HttpOutgoing { scope: req.clone() })
                } else {
                    None
                }
            }
            // Unscoped capabilities: if both match, return the capability
            _ if self.satisfies(request) => Some(request.clone()),
            _ => None,
        }
    }

    /// Returns `true` if this capability has the same kind as `other`,
    /// ignoring scope differences.
    ///
    /// # COLD PATH — used for unused-grant detection.
    pub fn same_kind(&self, other: &Capability) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::FilesystemRead { path } => write!(f, "filesystem:read:{path}"),
            Capability::FilesystemWrite { path } => write!(f, "filesystem:write:{path}"),
            Capability::TcpConnect { scope } => write!(f, "network:tcp-connect:{scope}"),
            Capability::TcpListen { scope } => write!(f, "network:tcp-listen:{scope}"),
            Capability::UdpAccess { scope } => write!(f, "network:udp:{scope}"),
            Capability::HttpOutgoing { scope } => write!(f, "network:http-outgoing:{scope}"),
            Capability::WallClock => write!(f, "clock:wall"),
            Capability::MonotonicClock => write!(f, "clock:monotonic"),
            Capability::CryptoRandom => write!(f, "random:crypto"),
            Capability::InsecureRandom => write!(f, "random:insecure"),
            Capability::Environment => write!(f, "environment:read"),
            Capability::Stdout => write!(f, "stdio:stdout"),
            Capability::Stderr => write!(f, "stdio:stderr"),
            Capability::ResourceAllocate { pool: Some(p) } => {
                write!(f, "torvyn:resource-allocate:{p}")
            }
            Capability::ResourceAllocate { pool: None } => {
                write!(f, "torvyn:resource-allocate")
            }
            Capability::PoolAccess { pool } => write!(f, "torvyn:pool-access:{pool}"),
            Capability::EmitBackpressure => write!(f, "torvyn:emit-backpressure"),
            Capability::InspectFlowMeta => write!(f, "torvyn:inspect-flow-meta"),
            Capability::RuntimeInspect => write!(f, "torvyn:runtime-inspect"),
            Capability::CustomMetrics => write!(f, "torvyn:custom-metrics"),
        }
    }
}

impl FromStr for Capability {
    type Err = CapabilityParseError;

    /// Parse a capability from its canonical string form.
    ///
    /// # COLD PATH — called during config parsing.
    ///
    /// # Errors
    /// Returns `CapabilityParseError` if the string does not match any known capability.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        if parts.len() < 2 {
            return Err(CapabilityParseError::InvalidFormat {
                input: s.to_owned(),
                reason: "expected format '<domain>:<action>[:<scope>]'".to_owned(),
            });
        }

        let domain = parts[0];
        let action = parts[1];
        let scope_str = parts.get(2).copied();

        match (domain, action) {
            ("filesystem", "read") => {
                let path = scope_str.ok_or_else(|| CapabilityParseError::MissingScope {
                    input: s.to_owned(),
                    expected: "path".to_owned(),
                })?;
                Ok(Capability::FilesystemRead {
                    path: PathScope::new(path),
                })
            }
            ("filesystem", "write") => {
                let path = scope_str.ok_or_else(|| CapabilityParseError::MissingScope {
                    input: s.to_owned(),
                    expected: "path".to_owned(),
                })?;
                Ok(Capability::FilesystemWrite {
                    path: PathScope::new(path),
                })
            }
            ("network", "tcp-connect") => {
                let scope = parse_net_scope_str(scope_str);
                Ok(Capability::TcpConnect { scope })
            }
            ("network", "tcp-listen") => {
                let scope = parse_net_scope_str(scope_str);
                Ok(Capability::TcpListen { scope })
            }
            ("network", "udp") => {
                let scope = parse_net_scope_str(scope_str);
                Ok(Capability::UdpAccess { scope })
            }
            ("network", "http-outgoing") => {
                let scope = parse_net_scope_str(scope_str);
                Ok(Capability::HttpOutgoing { scope })
            }
            ("clock", "wall") => Ok(Capability::WallClock),
            ("clock", "monotonic") => Ok(Capability::MonotonicClock),
            ("random", "crypto") => Ok(Capability::CryptoRandom),
            ("random", "insecure") => Ok(Capability::InsecureRandom),
            ("environment", "read") => Ok(Capability::Environment),
            ("stdio", "stdout") => Ok(Capability::Stdout),
            ("stdio", "stderr") => Ok(Capability::Stderr),
            ("torvyn", "resource-allocate") => {
                let pool = scope_str.map(PoolScope::new);
                Ok(Capability::ResourceAllocate { pool })
            }
            ("torvyn", "pool-access") => {
                let pool_name = scope_str.ok_or_else(|| CapabilityParseError::MissingScope {
                    input: s.to_owned(),
                    expected: "pool name".to_owned(),
                })?;
                Ok(Capability::PoolAccess {
                    pool: PoolScope::new(pool_name),
                })
            }
            ("torvyn", "emit-backpressure") => Ok(Capability::EmitBackpressure),
            ("torvyn", "inspect-flow-meta") => Ok(Capability::InspectFlowMeta),
            ("torvyn", "runtime-inspect") => Ok(Capability::RuntimeInspect),
            ("torvyn", "custom-metrics") => Ok(Capability::CustomMetrics),
            _ => Err(CapabilityParseError::UnknownCapability {
                input: s.to_owned(),
            }),
        }
    }
}

/// Parse a network scope from the optional scope portion of a capability string.
///
/// Format: `"host1,host2:port1-port2,port3"` or `"*"` for unrestricted.
///
/// # COLD PATH — called during config parsing.
fn parse_net_scope_str(scope_str: Option<&str>) -> NetScope {
    let scope_str = match scope_str {
        None | Some("*") | Some("*:*") => return NetScope::unrestricted(),
        Some("") => return NetScope::unrestricted(),
        Some(s) => s,
    };

    // Try to split on the last ':' that separates hosts from ports.
    let (host_part, port_part) = if let Some(last_colon) = scope_str.rfind(':') {
        let after = &scope_str[last_colon + 1..];
        // If everything after the last colon is digits/dashes/commas, it's a port spec
        if !after.is_empty()
            && after
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == ',')
        {
            (&scope_str[..last_colon], Some(after))
        } else {
            (scope_str, None)
        }
    } else {
        (scope_str, None)
    };

    let hosts: Vec<String> = if host_part == "*" || host_part.is_empty() {
        Vec::new()
    } else {
        host_part
            .split(',')
            .map(|h| h.trim().to_lowercase())
            .filter(|h| !h.is_empty())
            .collect()
    };

    let ports: Vec<PortRange> = match port_part {
        None => Vec::new(),
        Some(p) => p
            .split(',')
            .filter_map(|seg| {
                let seg = seg.trim();
                if seg.is_empty() {
                    return None;
                }
                if let Some((s, e)) = seg.split_once('-') {
                    let start = s.parse::<u16>().ok()?;
                    let end = e.parse::<u16>().ok()?;
                    PortRange::new(start, end).ok()
                } else {
                    let port = seg.parse::<u16>().ok()?;
                    Some(PortRange::single(port))
                }
            })
            .collect(),
    };

    NetScope::new(hosts, ports)
}

// ---------------------------------------------------------------------------
// CapabilityParseError
// ---------------------------------------------------------------------------

/// Error parsing a capability from a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityParseError {
    /// The string does not match the expected format.
    InvalidFormat {
        /// The input string.
        input: String,
        /// The reason for the error.
        reason: String,
    },
    /// A required scope component is missing.
    MissingScope {
        /// The input string.
        input: String,
        /// The expected scope type.
        expected: String,
    },
    /// The capability identifier is not recognized.
    UnknownCapability {
        /// The input string.
        input: String,
    },
}

impl fmt::Display for CapabilityParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilityParseError::InvalidFormat { input, reason } => {
                write!(
                    f,
                    "[E0510] Invalid capability format '{input}': {reason}. \
                     Expected format: '<domain>:<action>[:<scope>]'."
                )
            }
            CapabilityParseError::MissingScope { input, expected } => {
                write!(
                    f,
                    "[E0511] Capability '{input}' requires a {expected} scope. \
                     Example: 'filesystem:read:/data/input'."
                )
            }
            CapabilityParseError::UnknownCapability { input } => {
                write!(
                    f,
                    "[E0512] Unknown capability '{input}'. \
                     Run `torvyn inspect --capabilities` to see available capabilities."
                )
            }
        }
    }
}

impl std::error::Error for CapabilityParseError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- PathScope ---

    #[test]
    fn test_path_scope_contains_exact() {
        let scope = PathScope::new("/data");
        assert!(scope.contains_path("/data"));
    }

    #[test]
    fn test_path_scope_contains_descendant() {
        let scope = PathScope::new("/data");
        assert!(scope.contains_path("/data/input/file.txt"));
    }

    #[test]
    fn test_path_scope_does_not_contain_sibling() {
        let scope = PathScope::new("/data");
        assert!(!scope.contains_path("/data2"));
    }

    #[test]
    fn test_path_scope_does_not_contain_parent() {
        let scope = PathScope::new("/data/input");
        assert!(!scope.contains_path("/data"));
    }

    #[test]
    fn test_path_scope_root_contains_all() {
        let scope = PathScope::new("/");
        assert!(scope.contains_path("/anything/at/all"));
    }

    #[test]
    fn test_path_scope_trailing_slash_normalized() {
        let scope = PathScope::new("/data/input/");
        assert_eq!(scope.root(), "/data/input");
    }

    #[test]
    fn test_path_scope_contains_scope() {
        let broad = PathScope::new("/data");
        let narrow = PathScope::new("/data/input");
        assert!(broad.contains_scope(&narrow));
        assert!(!narrow.contains_scope(&broad));
    }

    #[test]
    fn test_path_scope_intersect_contained() {
        let broad = PathScope::new("/data");
        let narrow = PathScope::new("/data/input");
        let result = broad.intersect(&narrow);
        assert_eq!(result, Some(PathScope::new("/data/input")));
    }

    #[test]
    fn test_path_scope_intersect_disjoint() {
        let a = PathScope::new("/data/input");
        let b = PathScope::new("/data/output");
        assert_eq!(a.intersect(&b), None);
    }

    // --- PortRange ---

    #[test]
    fn test_port_range_single() {
        let range = PortRange::single(443);
        assert!(range.contains(443));
        assert!(!range.contains(80));
    }

    #[test]
    fn test_port_range_span() {
        let range = PortRange::new(80, 443).unwrap();
        assert!(range.contains(80));
        assert!(range.contains(200));
        assert!(range.contains(443));
        assert!(!range.contains(79));
        assert!(!range.contains(444));
    }

    #[test]
    fn test_port_range_invalid() {
        assert!(PortRange::new(443, 80).is_err());
    }

    #[test]
    fn test_port_range_contains_range() {
        let outer = PortRange::new(80, 8080).unwrap();
        let inner = PortRange::new(443, 443).unwrap();
        assert!(outer.contains_range(&inner));
        assert!(!inner.contains_range(&outer));
    }

    // --- NetScope ---

    #[test]
    fn test_net_scope_unrestricted() {
        let scope = NetScope::unrestricted();
        assert!(scope.is_unrestricted());
        assert!(scope.matches_host("anything.com"));
        assert!(scope.matches_port(12345));
    }

    #[test]
    fn test_net_scope_exact_host() {
        let scope = NetScope::new(vec!["api.example.com".into()], vec![]);
        assert!(scope.matches_host("api.example.com"));
        assert!(!scope.matches_host("evil.com"));
    }

    #[test]
    fn test_net_scope_wildcard_host() {
        let scope = NetScope::new(vec!["*.example.com".into()], vec![]);
        assert!(scope.matches_host("api.example.com"));
        assert!(scope.matches_host("sub.deep.example.com"));
        assert!(scope.matches_host("example.com")); // suffix match
        assert!(!scope.matches_host("evil.com"));
    }

    #[test]
    fn test_net_scope_host_case_insensitive() {
        let scope = NetScope::new(vec!["API.Example.COM".into()], vec![]);
        assert!(scope.matches_host("api.example.com"));
    }

    #[test]
    fn test_net_scope_port_filter() {
        let scope = NetScope::new(
            vec![],
            vec![PortRange::single(443), PortRange::new(8080, 8090).unwrap()],
        );
        assert!(scope.matches_port(443));
        assert!(scope.matches_port(8085));
        assert!(!scope.matches_port(80));
    }

    #[test]
    fn test_net_scope_contains_scope() {
        let broad = NetScope::unrestricted();
        let narrow = NetScope::new(vec!["api.example.com".into()], vec![PortRange::single(443)]);
        assert!(broad.contains_scope(&narrow));
        assert!(!narrow.contains_scope(&broad));
    }

    // --- Capability parsing ---

    #[test]
    fn test_capability_roundtrip_filesystem_read() {
        let cap = Capability::FilesystemRead {
            path: PathScope::new("/data/input"),
        };
        let s = cap.to_string();
        assert_eq!(s, "filesystem:read:/data/input");
        let parsed: Capability = s.parse().unwrap();
        assert_eq!(cap, parsed);
    }

    #[test]
    fn test_capability_roundtrip_wall_clock() {
        let cap = Capability::WallClock;
        let s = cap.to_string();
        assert_eq!(s, "clock:wall");
        let parsed: Capability = s.parse().unwrap();
        assert_eq!(cap, parsed);
    }

    #[test]
    fn test_capability_roundtrip_resource_allocate_unscoped() {
        let cap = Capability::ResourceAllocate { pool: None };
        let s = cap.to_string();
        assert_eq!(s, "torvyn:resource-allocate");
        let parsed: Capability = s.parse().unwrap();
        assert_eq!(cap, parsed);
    }

    #[test]
    fn test_capability_roundtrip_resource_allocate_scoped() {
        let cap = Capability::ResourceAllocate {
            pool: Some(PoolScope::new("default")),
        };
        let s = cap.to_string();
        assert_eq!(s, "torvyn:resource-allocate:default");
        let parsed: Capability = s.parse().unwrap();
        assert_eq!(cap, parsed);
    }

    #[test]
    fn test_capability_parse_unknown() {
        let result: Result<Capability, _> = "magic:teleport".parse();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CapabilityParseError::UnknownCapability { .. }
        ));
    }

    #[test]
    fn test_capability_parse_missing_scope() {
        let result: Result<Capability, _> = "filesystem:read".parse();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CapabilityParseError::MissingScope { .. }
        ));
    }

    #[test]
    fn test_capability_parse_bad_format() {
        let result: Result<Capability, _> = "nocolon".parse();
        assert!(result.is_err());
    }

    // --- Capability::satisfies ---

    #[test]
    fn test_satisfies_exact_unscoped() {
        assert!(Capability::WallClock.satisfies(&Capability::WallClock));
        assert!(!Capability::WallClock.satisfies(&Capability::MonotonicClock));
    }

    #[test]
    fn test_satisfies_broader_path_covers_narrower() {
        let grant = Capability::FilesystemRead {
            path: PathScope::new("/data"),
        };
        let request = Capability::FilesystemRead {
            path: PathScope::new("/data/input"),
        };
        assert!(grant.satisfies(&request));
    }

    #[test]
    fn test_satisfies_narrower_path_does_not_cover_broader() {
        let grant = Capability::FilesystemRead {
            path: PathScope::new("/data/input"),
        };
        let request = Capability::FilesystemRead {
            path: PathScope::new("/data"),
        };
        assert!(!grant.satisfies(&request));
    }

    #[test]
    fn test_satisfies_different_kind_never_matches() {
        let grant = Capability::FilesystemRead {
            path: PathScope::new("/"),
        };
        let request = Capability::FilesystemWrite {
            path: PathScope::new("/"),
        };
        assert!(!grant.satisfies(&request));
    }

    #[test]
    fn test_satisfies_resource_allocate_unscoped_grant() {
        let grant = Capability::ResourceAllocate { pool: None };
        let request = Capability::ResourceAllocate {
            pool: Some(PoolScope::new("default")),
        };
        assert!(grant.satisfies(&request));
    }

    #[test]
    fn test_satisfies_resource_allocate_scoped_grant_wrong_pool() {
        let grant = Capability::ResourceAllocate {
            pool: Some(PoolScope::new("other")),
        };
        let request = Capability::ResourceAllocate {
            pool: Some(PoolScope::new("default")),
        };
        assert!(!grant.satisfies(&request));
    }

    // --- Capability::domain ---

    #[test]
    fn test_capability_domain() {
        assert_eq!(
            Capability::FilesystemRead {
                path: PathScope::new("/")
            }
            .domain(),
            "filesystem"
        );
        assert_eq!(Capability::WallClock.domain(), "clock");
        assert_eq!(Capability::CustomMetrics.domain(), "torvyn:runtime");
    }

    // --- Capability::is_hot_path ---

    #[test]
    fn test_capability_is_hot_path() {
        assert!(Capability::ResourceAllocate { pool: None }.is_hot_path());
        assert!(Capability::EmitBackpressure.is_hot_path());
        assert!(Capability::CustomMetrics.is_hot_path());
        assert!(Capability::InspectFlowMeta.is_hot_path());
        assert!(!Capability::WallClock.is_hot_path());
        assert!(!Capability::Environment.is_hot_path());
    }
}
