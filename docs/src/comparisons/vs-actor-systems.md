# Torvyn vs. Actor Frameworks

An honest comparison for teams evaluating actor-based concurrency versus Torvyn's reactive streaming model.

---

## What Actor Frameworks Provide

Actor frameworks (Akka, Erlang/OTP, Microsoft Orleans, Ractor in Rust) structure concurrent systems as isolated actors that communicate through asynchronous message passing. Each actor encapsulates state and processes messages sequentially. Actor systems provide location transparency, fault tolerance through supervision hierarchies, and a natural model for stateful, event-driven workloads.

## Where Torvyn Is a Better Fit

**Stream-oriented scheduling.** Actor systems schedule at the message level — each message to an actor triggers a processing cycle. Torvyn schedules at the stream level — the reactor understands the full pipeline graph, propagates demand from consumer to producer, and applies back-pressure across the entire flow. For streaming pipelines where the relationship between stages is a directed graph, Torvyn's flow-aware scheduling provides better queue management and more predictable latency.

**Typed contracts for composition.** Actor message types are typically language-level types, not cross-component contracts. Torvyn's WIT contracts are language-neutral, version-controlled, and machine-checkable. `torvyn link` validates that an entire pipeline graph is type-compatible before runtime. Actors typically discover type mismatches through runtime errors.

**Observable back-pressure.** Most actor systems handle overload through mailbox overflow policies (drop, backoff, fail) but do not provide structured demand propagation or observable watermark-based flow control. Torvyn's credit-based demand model with hysteresis watermarks provides predictable, measurable back-pressure with clear observability events.

**Explicit ownership semantics.** In actor systems, message passing typically involves serialization (for remote actors) or reference passing (for local actors). Ownership of the message data is implicit. Torvyn's resource manager makes ownership explicit: every buffer has exactly one owner, transfers are tracked, and copy behavior is measurable.

**Sandboxed polyglot execution.** Actors in most frameworks must be written in the framework's language (Scala/Java for Akka, Erlang for OTP, Rust for Ractor). Torvyn components can be written in any language that targets the WebAssembly Component Model, with each component running in a sandboxed environment.

## Where Actor Frameworks Are a Better Fit

**Stateful, entity-centric workloads.** Actor systems excel at modeling stateful entities (user sessions, device twins, game entities) where each entity has its own isolated state and handles messages independently. Torvyn's streaming model is optimized for stateless or lightly-stateful transform pipelines, not for entity-per-actor patterns.

**Supervision and fault tolerance.** OTP and Akka provide sophisticated supervision hierarchies that automatically restart failed actors, isolate failure domains, and implement escalation policies. Torvyn provides flow-level failure isolation and tiered cancellation, but does not include a supervision tree model.

**Location transparency and distribution.** Actor frameworks can distribute actors across machines transparently. Torvyn's current scope is same-node composition.

**Ecosystem and community.** Akka and Erlang/OTP have decades of production use and large communities. Torvyn is a new project.

## When to Use Which

**Use Torvyn when** your workload is a streaming pipeline with defined topology, where flow-aware scheduling and observable back-pressure are more important than per-entity isolation and supervision.

**Use an actor framework when** your workload consists of many independent stateful entities, you need supervision hierarchies for fault recovery, or you need to distribute processing across machines transparently.

**Consider both** for architectures where stateful entities (modeled as actors) produce event streams that feed into processing pipelines (modeled in Torvyn).
