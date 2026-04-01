# Contracts-First Composition: Why Typed Interfaces Are the Center of Torvyn

*How WIT contracts prevent production failures and enable safe evolution.*

---

Most system failures at component boundaries are not caused by a missing technology. They are caused by unstated assumptions — about data format, about field presence, about ownership semantics, about error handling, about versioning. Two systems that "work together" in development fail in production because the contract between them was implicit, untested, or out of date.

Torvyn makes the contract explicit, typed, versioned, and machine-checkable. This is not an afterthought. It is the center of the product.

## WIT as the Contract Language

Torvyn uses WIT (WebAssembly Interface Types) as its contract definition language. WIT is the standard interface language of the WebAssembly Component Model, maintained by the Bytecode Alliance.

A WIT contract in Torvyn defines:

- **Types:** Records, variants, and enums that describe the structure of stream elements, metadata, and error cases.
- **Resources:** Host-managed entities with explicit ownership semantics. The `buffer` resource in Torvyn represents a data payload managed by the host. WIT's `own<T>` and `borrow<T>` handle types make ownership visible in the contract.
- **Interfaces:** The functions a component exports (processor, source, sink) or imports (buffer allocation, capability access).
- **Worlds:** The complete requirement and capability surface of a component — what it exports, what it imports, and what it depends on.

WIT contracts are human-readable, language-neutral, and statically validatable. They serve as both documentation and enforcement mechanism.

## Static Validation Before Runtime

Torvyn's validation pipeline catches errors at three stages:

**Parse-time validation (`torvyn check`)** verifies WIT syntax, manifest format, capability declarations, and world completeness for a single component.

**Semantic validation (`torvyn check`)** checks type consistency, resource usage patterns, and version constraints.

**Composition validation (`torvyn link`)** validates interface compatibility between connected components, verifies capability satisfaction for the full pipeline, checks topology correctness (valid DAG structure, sources have no inputs, sinks have no outputs, router port names match actual destinations), and ensures version ranges across all components have a non-empty intersection.

Errors produce structured messages with error codes, file locations, explanations, and actionable fix suggestions. Interface mismatches, missing capabilities, and version conflicts are caught before any component code executes.

## Version Evolution Rules

Torvyn WIT packages follow semantic versioning with clearly defined rules for what constitutes a breaking change. Removing a type, changing a function signature, removing a record field, or changing ownership semantics (borrow to own or vice versa) requires a major version bump. Adding a new function to an existing interface or adding a new interface to a package is a compatible minor change.

The `torvyn link` command performs structural compatibility checking. Even within compatible version ranges, it verifies that the specific interfaces and functions used by a consumer are present in the provider. A consumer compiled against version 0.2.0 that uses a function added in 0.2.0 will fail to link with a provider that only implements 0.1.0 — and the error message will identify exactly which function is missing and what version would resolve the issue.

## Preventing Production Failures

Consider a concrete scenario. A team updates a policy filter component. The new version expects a `priority` field in the stream element metadata — a field that was added in contract version 0.2.0. The upstream data enricher was compiled against contract version 0.1.0 and does not produce this field.

Without Torvyn, this pipeline deploys successfully and fails at runtime when the filter attempts to access a field that does not exist. The failure may be intermittent (if the field is optional in the filter's implementation), hard to diagnose (the error may surface as unexpected behavior rather than a clean failure), and may only occur in production (if the test environment uses different contract versions).

With Torvyn, `torvyn link` catches this incompatibility before deployment. The error message identifies the conflicting versions, the specific field that causes the mismatch, and suggests upgrading the enricher to a version compiled against contract 0.2.0 or later.

This is not a theoretical benefit. Schema drift, interface mismatch, and version conflict are among the most common causes of distributed system failures. Catching them statically eliminates an entire category of production incidents.

## Why Contracts Must Be the Center

A runtime that treats contracts as optional — where components can interact without formal interfaces, or where the interface is defined by convention rather than type — will always be fragile at scale. As the number of components grows, as teams evolve their components independently, and as third-party components enter the ecosystem, the only reliable coordination mechanism is a machine-checked contract.

Torvyn's design reflects this belief. Contracts are not metadata attached to components. They are the foundation upon which everything else is built: linking, scheduling, resource transfer, capability validation, and version compatibility all flow from the contract layer.
