# Torvyn vs. Traditional Microservices

An honest comparison for teams evaluating whether to decompose a local pipeline into microservices or compose it as Torvyn components.

---

## What Microservices Provide

The microservice architecture decomposes a system into independently deployable services, each with its own process, data store, and team ownership. It enables organizational scaling, independent release cycles, technology diversity, and fault isolation through process boundaries.

## Where Torvyn Is a Better Fit

**Lower boundary overhead for co-located stages.** Microservice boundaries impose serialization, network transport (even if local), load balancing, retries, and observability stitching at every boundary. For pipeline stages that run on the same node and process high-frequency streams, this overhead is a significant fraction of the total processing budget. Torvyn replaces these heavyweight boundaries with contract-defined component boundaries that transfer data through host-managed memory.

**Deterministic resource behavior.** Microservice pipelines are subject to network variability, container scheduling latency, and cross-service retry storms. Torvyn pipelines run within a single process with bounded queues, explicit back-pressure, and configurable memory budgets. Resource behavior is deterministic and observable.

**Typed composition validation.** Microservice interfaces are typically defined by API specifications (OpenAPI, gRPC proto files), but there is no standard mechanism to validate compatibility across an entire service graph before deployment. Torvyn's `torvyn link` validates the full pipeline graph statically.

**Lighter operational footprint.** A Torvyn pipeline runs as a single process. There is no need for container orchestration, service discovery, load balancing, or circuit breaking between pipeline stages. For workloads that do not require independent deployment of individual stages, this reduces operational complexity.

## Where Microservices Are a Better Fit

**Independent deployment and scaling.** Microservices allow each service to be deployed, scaled, and updated independently. Torvyn components within a pipeline are co-deployed and updated together. If different stages have different scaling requirements or different release cadences, microservices provide more flexibility.

**Organizational boundaries.** Microservices align well with team-per-service ownership. Torvyn pipelines work best when the team that owns the pipeline owns all its component stages.

**Failure isolation through process boundaries.** A crash in one microservice does not bring down another. In Torvyn, a Wasm trap in one component is isolated from other components within the same flow, but all flows share the same host process. Torvyn's sandboxing provides strong memory isolation, but the failure domain is the host process, not an independent OS process.

**Distributed workloads.** Microservices run anywhere on the network. Torvyn's current scope is same-node and edge-local composition. If your pipeline stages need to run on different machines, microservices are the correct architecture.

## When to Use Which

**Use Torvyn when** your pipeline stages run on the same machine, your latency budget does not accommodate inter-process or inter-service overhead, your stages share a deployment lifecycle, and you value static contract validation and fine-grained resource observability.

**Use microservices when** your services need independent deployment, independent scaling, or independent team ownership, or when the services run on different machines.

**Combine them** by building a high-performance local pipeline in Torvyn that exposes a single service interface to the rest of your microservice architecture. The pipeline handles the internal, latency-sensitive composition; the microservice handles the external, network-facing interface.
