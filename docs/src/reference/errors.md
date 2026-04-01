# Error Code Reference

Every error produced by the Torvyn CLI and runtime includes a unique error code, a description of what went wrong, the location where it occurred, why it is an error, and how to fix it. Error codes are stable across versions and searchable in documentation.

## Error Code Ranges

| Range | Category |
|-------|----------|
| `E0001–E0099` | General errors (I/O, configuration, environment) |
| `E0100–E0199` | Contract and WIT errors |
| `E0200–E0299` | Linking and composition errors |
| `E0300–E0399` | Resource manager errors |
| `E0400–E0499` | Reactor and scheduling errors |
| `E0500–E0599` | Security and capability errors |
| `E0600–E0699` | Packaging and distribution errors |
| `E0700–E0799` | Configuration errors |

## General Errors (E0001–E0099)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0001` | `ManifestNotFound` | `Torvyn.toml` not found at the specified or default path. | Run `torvyn init` to create a project, or specify `--manifest <PATH>`. |
| `E0002` | `ManifestParseError` | `Torvyn.toml` contains invalid TOML syntax. | Check the indicated line and column for syntax errors. |
| `E0003` | `DirectoryExists` | Target directory for `torvyn init` already exists and is not empty. | Use `--force` to overwrite, or choose a different name. |
| `E0010` | `ToolchainMissing` | A required external tool (`cargo-component`, `wasm-tools`, etc.) was not found. | Run `torvyn doctor` for installation instructions. |

## Contract Errors (E0100–E0199)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0100` | `WitSyntaxError` | A `.wit` file contains a syntax error. | Check the indicated file, line, and column. |
| `E0101` | `WitResolutionError` | A `use` statement or package reference could not be resolved. | Ensure the referenced package exists in `wit/deps/`. Run `torvyn init` or re-vendor dependencies. |
| `E0102` | `WorldIncomplete` | The component's world does not export any Torvyn processing interface. | Add an `export` for at least one of: `processor`, `source`, `sink`, `filter`, `router`, `aggregator`. |
| `E0103` | `CapabilityMismatch` | WIT imports require a capability that is not declared in the manifest. | Add the required capability to `[capabilities.required]` in `Torvyn.toml`. |
| `E0110` | `DeprecatedContractVersion` | The targeted contract version is deprecated. | Upgrade to a supported contract version. |

## Linking Errors (E0200–E0299)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0200` | `InterfaceIncompatible` | An upstream component's output type is incompatible with the downstream component's input type. | Ensure both components target compatible contract versions. |
| `E0201` | `VersionMismatch` | Components target different major versions of the same contract package. | Recompile one or both components against a compatible contract version. |
| `E0202` | `CapabilityNotGranted` | A required capability is not granted in the pipeline configuration. | Add the capability to `[security.grants.<component>]`. |
| `E0203` | `TopologyInvalid` | The pipeline graph has structural errors (cycles, disconnected nodes, role violations). | Review the flow definition. Sources must have no inputs. Sinks must have no outputs. The graph must be a DAG. |
| `E0204` | `RouterPortUnknown` | A router returned a port name that does not match any downstream connection. | Verify router port names match the edge definitions in the flow topology. |
| `E0210` | `ComponentNotFound` | A component referenced in the flow definition could not be located. | Check the `component` path in the flow node definition. Run `torvyn build` first. |

## Resource Errors (E0300–E0399)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0300` | `AllocationFailed` | Buffer allocation failed (pool exhausted and system allocator failed). | Increase pool sizes or reduce component memory budgets. |
| `E0301` | `PoolExhausted` | Buffer pool for the requested tier is empty. | Increase pool size for the affected tier, or switch exhaustion policy to `fallback-alloc`. |
| `E0302` | `NotOwner` | A component attempted to access a buffer it does not own. | This indicates a contract or runtime bug. Report the issue. |
| `E0303` | `StaleHandle` | A component used a buffer handle that has been invalidated (wrong generation). | This indicates a use-after-free pattern in component code. Review component logic. |
| `E0304` | `BorrowsOutstanding` | Attempted to transfer or free a buffer while borrows are still active. | Ensure all borrows are released before transferring ownership. |
| `E0305` | `CapacityExceeded` | A write to a mutable buffer would exceed its capacity. | Allocate a larger buffer or write data in smaller chunks. |
| `E0310` | `BudgetExceeded` | A component exceeded its per-component memory budget. | Increase `max_memory_per_component` or optimize component memory usage. |

## Reactor Errors (E0400–E0499)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0400` | `FlowDeadlineExceeded` | The flow exceeded its configured maximum execution time. | Increase the timeout or investigate which component is slow. |
| `E0401` | `ComponentTimeout` | A single component invocation exceeded its per-call timeout. | Increase `fuel_per_invocation` or optimize the component. |
| `E0402` | `FlowCancelled` | The flow was cancelled by operator command or fatal error. | Check the cancellation reason in the flow trace. |
| `E0410` | `FatalComponentError` | A component returned `process-error::fatal`. | The component cannot process further elements. Review component logs for the cause. |

## Security Errors (E0500–E0599)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0500` | `CapabilityDenied` | A component attempted to use a capability it was not granted. | Grant the capability in `[security.grants.<component>]`, or remove the capability use from the component. |
| `E0501` | `SandboxViolation` | A component attempted an operation outside its sandbox. | Review component code for disallowed operations. |
| `E0510` | `AuditLogFailed` | The audit log could not be written. | Check audit log target configuration and disk space. |

## Packaging Errors (E0600–E0699)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0600` | `ArtifactInvalid` | The packaged artifact failed validation. | Run `torvyn check` before `torvyn pack`. |
| `E0601` | `RegistryAuthFailed` | Authentication with the OCI registry failed. | Check registry credentials. |
| `E0602` | `PushFailed` | The artifact could not be pushed to the registry. | Check network connectivity and registry availability. |
| `E0603` | `SigningFailed` | Artifact signing failed. | Check signing key configuration. |

## Configuration Errors (E0700–E0799)

| Code | Name | Cause | Fix |
|------|------|-------|-----|
| `E0700` | `InvalidFieldType` | A configuration field has the wrong type. | Check the field type in the configuration reference. |
| `E0701` | `UnknownField` | An unrecognized field name in `Torvyn.toml`. | Check for typos. Refer to the configuration reference for valid field names. |
| `E0702` | `ConstraintViolation` | A configuration value is outside its valid range. | Check the valid range in the configuration reference. |
