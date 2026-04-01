# Secure Plugin Systems

## The Problem

Many platforms need extensibility. Users want to add custom processing logic, integrations, and business rules without modifying the core platform. The traditional approach is to offer a plugin API: load user code as a shared library, a scripted extension, or an embedded interpreter.

Each approach carries risks. Shared libraries (C/C++ plugins, Rust dylibs) execute in the same address space as the host — a bug in a plugin can corrupt host memory, and a malicious plugin can access any host resource. Scripted extensions (Lua, JavaScript) provide better isolation but often with limited performance and limited type safety. Docker-based plugin execution provides strong isolation but introduces container orchestration overhead, cold-start latency, and complex lifecycle management.

The fundamental tension is between performance (in-process speed), safety (isolation from the host), and governance (knowing exactly what a plugin can do).

## How Torvyn Solves It

Torvyn resolves this tension through the WebAssembly Component Model's sandboxing combined with an explicit capability system.

**In-process speed, out-of-process safety.** Plugin components are compiled to WebAssembly and execute within the Torvyn host process, avoiding the overhead of inter-process communication or container startup. But each component runs in its own linear memory space, enforced by the Wasm sandbox. A plugin cannot read host memory, another plugin's memory, or any resource it has not been explicitly granted access to.

**Deny-all default.** Every capability — filesystem access, network access, system clocks, random number generation — is denied unless explicitly granted in the pipeline configuration. The component manifest declares what the plugin needs. The operator decides what to grant. The host enforces the grant at runtime.

**Auditable permissions.** Operators can inspect the capability manifest of every deployed component. `torvyn link` validates that capability grants satisfy each component's requirements. Audit events record every capability exercise at runtime. Security teams can review exactly what each plugin is authorized to do and verify that it has not exceeded its grants.

**Safe multi-tenant execution.** Multiple plugins from different tenants can run in the same Torvyn host process with resource isolation. Memory budgets, CPU fuel limits, and queue capacity constraints prevent any single plugin from monopolizing shared resources.

**Contract-governed interfaces.** Plugins interact with the host platform through typed WIT contracts. The interface a plugin can use is defined and versioned. Breaking changes to the plugin API are caught at link time, not at runtime. Plugin authors get stable, documented, type-safe interfaces.

## Example Architecture

```
Platform Event Bus → [Plugin A: Custom Validator] → [Plugin B: Enrichment from External API] → Platform Core
```

Plugin A and Plugin B are third-party Wasm components. Each runs in its own sandbox with its own capability grants. Plugin A is granted no capabilities (it operates only on the event data it receives). Plugin B is granted network egress to a specific API endpoint. Neither plugin can access the host filesystem, the platform's internal state, or each other's data.

## Performance Characteristics

Torvyn's component-to-component boundary overhead adds single-digit microsecond latency per invocation, as measured by the reactor's per-element overhead target (< 5us). For plugin systems where plugins perform meaningful computation (parsing, validation, transformation, API calls), the boundary overhead is negligible compared to the plugin's own execution time.

The security enforcement (capability checking, resource budget tracking, audit event emission) is designed to operate within the reactor's per-element overhead budget. Security is not an opt-in feature with a performance cost — it is always active.

## Get Started

- [Quickstart guide](/docs/quickstart)
- [Tutorial: building a plugin system](/docs/tutorials/plugin-system)
- [Architecture guide: security and capabilities](/docs/architecture/security)
