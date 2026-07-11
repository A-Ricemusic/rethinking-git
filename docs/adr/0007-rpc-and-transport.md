# ADR 0007: Versioned gRPC over HTTP/2 with rustls

- Status: Accepted
- Date: 2026-07-11
- Owners: sync, server, and security

## Context

The native service needs typed cross-language APIs, bidirectional streaming,
backpressure, cancellation, status semantics, and standard proxy/load-balancer support.
The wire protocol is not the canonical object encoding. Authorization must occur
before negotiation reveals restricted object existence.

## Decision

Use protobuf service schemas with `tonic`/`prost`, gRPC over HTTP/2, and TLS 1.3 via
rustls. Production uses mutual TLS for device/service identity plus short-lived,
audience-bound authorization capabilities at the RPC layer. Browser gateways are
optional adapters, not the native protocol.

Every service and message has an explicit protocol version. Protobuf bytes are never
used as object IDs. Bulk object records carry canonical CBOR/encrypted envelope bytes
inside bounded streaming messages, verify incrementally, and resume only from verified
boundaries. Negotiation exposes only algorithms/features allowed by server policy;
authorization precedes have/want, ancestry, size, and error detail. Reference updates
are idempotent compare-and-swap transactions with request IDs.

## Consequences

- Strong tooling and streaming support outweigh HTTP/2/protobuf complexity.
- gRPC limits, proxy behavior, keepalive, and error normalization are security and
  reliability configuration, not defaults to inherit blindly.
- REST/JSON may be provided for administrative views but cannot bypass native policy.
- QUIC/HTTP/3 can be evaluated later without changing canonical objects.

## Rejected alternatives

- Ad hoc HTTP/JSON: weaker streaming/schema/evolution guarantees.
- Custom binary transport: excessive implementation and audit burden.
- Git smart protocol extension: leaks Git data-model assumptions into native security.

## Verification and open work

Test independent clients, compatibility across adjacent versions, retries/idempotency,
slow and malicious peers, cancellation, frame/decompression limits, uniform denial
responses, certificate/capability rotation, and proxy deployments. Define the sync
state machine and offline capability format in dedicated specifications.
