# ADR 0002: BLAKE3-256 with explicit hash agility

- Status: Accepted
- Date: 2026-07-11
- Owners: objects, crypto, and storage

## Context

RGit hashes large content continuously and needs collision resistance, fast software
implementations, streaming, and safe parallelism. Long-lived repositories must not
embed an unversioned digest assumption. Some regulated deployments require SHA-256.

## Decision

New native repositories use BLAKE3-256. Every object ID carries an ID-format version,
registered algorithm code, and digest length. SHA-256 is registered for migration,
interoperability, and explicitly configured compliance profiles. Hash input is
domain-separated by system, object kind, and schema version.

Repositories record an allowed algorithm set and one write algorithm. Readers verify
all allowed algorithms; writers never select an algorithm from untrusted input.
Changing the write algorithm creates new IDs through a signed, resumable migration
object mapping old IDs to new IDs. No “same digest bytes means same object” shortcut
is allowed across algorithms.

## Consequences

- BLAKE3 provides high throughput and a well-defined 256-bit output.
- IDs are slightly larger and all maps/indexes must use a parsed typed ID.
- Supporting multiple algorithms expands verification and downgrade test surfaces.
- FIPS-oriented deployments may write SHA-256, subject to a separately validated
  cryptographic module; RGit does not claim FIPS validation itself.

## Rejected alternatives

- Unversioned SHA-256 or BLAKE3: no safe agility.
- SHA-1 compatibility: collision resistance is inadequate.
- Hashing ciphertext: re-encryption would destroy logical identity and deduplication.

## Verification and open work

Use official known-answer vectors, mutation tests, streaming/chunk-boundary tests, and
dual-hash migration fixtures. Define repository-scale migration UX and whether a
future transparency witness must bind both IDs.
