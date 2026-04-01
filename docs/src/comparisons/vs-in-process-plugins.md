# Torvyn vs. Container-Based Composition

An honest comparison for teams evaluating containers versus WebAssembly components for fine-grained pipeline composition.

---

## What Containers Provide

Containers (Docker, OCI) are the standard unit of deployment in modern infrastructure. They provide strong process-level isolation, a portable packaging format, mature orchestration tooling (Kubernetes), broad ecosystem support, and well-understood operational practices.

## Where Torvyn Is a Better Fit

**Startup time.** Container startup involves image pulling, filesystem layering, namespace setup, and process creation. Cold starts are measured in hundreds of milliseconds to seconds. Wasm component instantiation is measured in microseconds to low milliseconds. For pipelines that create and tear down processing stages frequently, or for edge deployments that need rapid recovery, Torvyn's startup time is orders of magnitude faster.

**Memory footprint.** Each container brings a filesystem layer, a process, and runtime dependencies. A minimal container is megabytes; a typical one is tens to hundreds of megabytes. A Torvyn Wasm component is kilobytes to low megabytes. On resource-constrained edge hardware, or when running hundreds of fine-grained stages, the memory savings are significant.

**Inter-stage communication.** Containers communicate via IPC or network (even on the same host). This requires serialization, kernel context switches, and potentially TCP overhead. Torvyn components communicate through host-managed memory transfers within a single process. For high-frequency streaming (thousands to hundreds of thousands of elements per second), the per-element overhead difference is substantial.

**Granularity.** Containers are designed for service-level granularity: each container runs an application or service. Torvyn components are designed for stage-level granularity: each component implements a single processing step. A pipeline that would require 15 containers requires 15 Torvyn components, but with drastically lower overhead per stage.

## Where Containers Are a Better Fit

**Ecosystem maturity.** Container tooling — building, testing, deploying, monitoring — is mature, well-documented, and widely understood. Torvyn is a new project with a growing ecosystem.

**Arbitrary runtimes.** Containers can run any Linux-compatible binary: Node.js services, Python applications, legacy C++ code, databases. Torvyn components must compile to WebAssembly. The Wasm ecosystem is growing rapidly, but not all languages and libraries are Wasm-ready today.

**Orchestration and scaling.** Kubernetes, ECS, and other orchestrators provide declarative scaling, health checking, rolling updates, and service discovery for containerized workloads. Torvyn does not include orchestration — it is a runtime, not a platform. For workloads that need horizontal scaling across machines, containers with orchestration are the right choice.

**Network services.** Containers hosting long-running network services (web servers, databases, API gateways) benefit from process isolation, independent scaling, and network-level routing. Torvyn is designed for stream processing stages, not for hosting network services.

## When to Use Which

**Use Torvyn when** you need fine-grained, high-frequency pipeline composition on a single node or edge device, where container startup time and per-stage overhead are prohibitive.

**Use containers when** you need to deploy arbitrary applications, require orchestrated scaling across machines, or need to run software that does not target WebAssembly.

**Use both** by packaging a Torvyn pipeline as a single container for deployment. The container provides the familiar operational model; Torvyn provides efficient internal composition.
