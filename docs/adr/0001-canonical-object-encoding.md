# ADR 0001: Deterministic CBOR for canonical objects

- Status: Accepted
- Date: 2026-07-11
- Owners: objects and storage

## Context

Object IDs, signatures, cross-platform decoding, and long-term recovery require one
byte representation for each logical value. JSON has ambiguous numeric and Unicode
representations. Protobuf's normal wire form does not promise canonical serialization
and unknown-field behavior complicates hashing. A bespoke format increases parser and
audit risk.

## Decision

Use RFC 8949 deterministic CBOR with the stricter profile in
[canonical-encoding.md](../../spec/canonical-encoding.md): integer map keys, definite
lengths, shortest integers, NFC text, no floats/tags, sorted set encodings, duplicate
rejection, and strict decode/re-encode verification. Object kind and schema version
are domain-separated in the hash preimage. JSON is debug rendering only.

## Consequences

- Compact streaming encodings and implementations in other languages remain feasible.
- Schemas and numeric field registries must be maintained independently of Rust types.
- Generic serde round trips are insufficient; adversarial test vectors gate the CBOR
  library selection and every schema change.
- A canonical-profile change creates a new object-ID format and migration, not an
  in-place reinterpretation.

## Rejected alternatives

- Canonical JSON: larger, weaker binary support, and more normalization hazards.
- Protobuf as hashed form: excellent RPC schema but no default canonical guarantee.
- MessagePack/bincode/postcard: weaker standards/interoperability story or Rust type
  layout coupling.

## Verification and open work

Publish byte vectors for all objects and invalid encodings; execute them on every
supported OS and at least one independent decoder. Select the concrete CBOR crate
only after conformance tests; this ADR does not pre-approve a serializer.
