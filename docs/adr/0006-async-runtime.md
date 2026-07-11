# ADR 0006: Tokio as the bounded async runtime

- Status: Accepted
- Date: 2026-07-11
- Owners: sync and server

## Context

Streaming synchronization and the remote service need cancellation, timeouts,
backpressure, concurrent connections, and mature RPC integration. Object encoding,
graph algorithms, local transactions, and filesystem materialization are primarily
synchronous/CPU/blocking work and should not become runtime-coupled.

## Decision

Use Tokio 1.x for `rgit-sync` and `rgit-server` network orchestration. Library APIs in
objects, crypto, policy, graph, and store remain runtime-agnostic and synchronous
unless genuine async I/O requires otherwise. Blocking SQLite/filesystem work runs in
bounded dedicated pools, never directly on Tokio worker threads. Do not expose Tokio
types from foundational crate public APIs.

All spawned tasks have ownership, cancellation, deadlines, concurrency/resource
budgets, and observed failures. Avoid detached tasks. The CLI creates a current-thread
runtime only for commands that need networking; ordinary local commands need none.

## Consequences

- Tokio aligns with tonic/hyper/rustls and has mature instrumentation and testing.
- The boundary between async orchestration and synchronous core adds adapters but
  prevents “async everywhere” and enables deterministic model tests.
- CPU hashing/chunking uses explicit bounded CPU work rather than the async scheduler.

## Rejected alternatives

- async-std/smol: smaller ecosystems for the selected RPC stack.
- no async runtime/thread per connection: weak scalability and cancellation story.
- Tokio types throughout all crates: unnecessary coupling and testing complexity.

## Verification and open work

Use paused-time tests, cancellation at every transfer/transaction phase, backpressure
and slow-peer tests, leak detection, bounded-queue metrics, and graceful shutdown tests.
Pool sizes and service-level limits remain benchmark-driven deployment configuration.
