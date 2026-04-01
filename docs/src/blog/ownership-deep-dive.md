# Why "Ownership-Aware," Not "Zero-Copy"

*On being precise about what happens at component boundaries.*

---

When describing a streaming runtime's data transfer model, the tempting marketing claim is "zero-copy." It is short, impressive, and immediately suggests performance. Torvyn deliberately avoids this claim. Here is why.

## The Problem with "Zero-Copy"

The WebAssembly Component Model provides real memory isolation between components. Each component has its own linear memory. The host has its own address space. When Component A produces data and Component B needs to read or transform that data, the bytes must, at some point, exist in Component B's linear memory.

This is not a limitation of Torvyn's design. It is a consequence of the memory isolation that makes sandboxing possible. The same isolation that prevents Component A from corrupting Component B's memory is the isolation that requires data copies when components need to operate on shared data.

Claiming "zero-copy" would mean either: (a) claiming something that is physically impossible across Wasm component boundaries for all cases, or (b) redefining "zero-copy" in a way that is technically defensible but misleading to practitioners who understand the term in its conventional sense.

Neither option builds trust.

## What "Ownership-Aware" Means in Practice

Torvyn's resource manager tracks every buffer's lifecycle from allocation to reclamation. At any given moment, every buffer in the system has exactly one owner, and the host knows who that owner is.

This ownership tracking enables three distinct transfer scenarios:

**Handle pass-through (zero-copy in the payload path).** When a buffer moves from Component A to Component B and neither component needs to read or modify the payload, the transfer updates the ownership record in the resource table. The payload bytes do not move. This path is available for routing stages, metadata-only inspection, fan-out distribution, and any stage where the component operates on the element's metadata without touching the payload.

**Payload read (one copy into consumer memory).** When Component B needs to read the payload, the resource manager copies the bytes from the host-managed buffer into Component B's linear memory. This is one copy, performed on demand, only when the component calls `buffer.read()`.

**Payload write (one copy from producer memory).** When a component allocates a new buffer and writes output data, the resource manager copies the bytes from the component's linear memory into a host-managed buffer. Again, one copy, on demand.

The key insight is that many pipeline stages do not need to read every payload. Routing decisions, content-type checking, trace context propagation, policy evaluation based on metadata, priority assignment, and fan-out distribution can all operate on metadata alone. For these stages, the payload transfer is genuinely zero-copy.

## Copy Accounting Makes This Measurable

Torvyn does not ask you to trust that copies are minimal. It proves it.

Every copy operation produces a `TransferRecord` with the timestamp, source and destination entities, byte count, copy reason (metadata marshaling, payload read, payload write, or host serialization), and the flow the copy belongs to.

These records aggregate into per-flow copy statistics: total payload bytes copied, total metadata bytes copied, copy count per component boundary, and a copy amplification ratio — the ratio of payload bytes copied to payload bytes produced. A metadata-routing pipeline should show a copy amplification ratio near 0.0. A transform-heavy pipeline should show a ratio near 1.0.

The `torvyn bench` command reports these metrics. They are part of the standard output for every benchmark run.

## Why This Matters for Performance Engineering

A runtime that claims "zero-copy" gives you nothing to measure. Either everything is zero-copy (which is impossible), or some transfers involve copies and you have no visibility into which ones or how many.

A runtime that provides copy accounting gives you a performance engineering tool. You can identify which component boundaries produce the most copies, evaluate whether a particular stage could be restructured to operate on metadata alone, measure the impact of changing a pipeline topology, and verify that optimization efforts actually reduce data movement.

This is the difference between a marketing claim and an engineering tool.

## The Honest Position

Torvyn minimizes copies where the architecture permits, makes every copy visible, and provides the instrumentation to understand and improve data transfer behavior. It does not pretend that sandboxed composition is free of data movement costs.

This is what "ownership-aware" means: the runtime knows who owns what, knows when and why data moves, and makes that information available to you.
