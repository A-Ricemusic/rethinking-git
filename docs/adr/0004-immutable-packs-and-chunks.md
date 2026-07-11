# ADR 0004: Immutable pack files with content-defined large-file chunks

- Status: Accepted
- Date: 2026-07-11
- Owners: storage and sync

## Context

Loose objects give simple crash safety but waste inodes and make transfer inefficient.
Mutable pack files complicate concurrent reads and recovery. Large files need streaming
and deduplication across edits without forcing the graph identity to equal a physical
layout.

## Decision

Begin with durable immutable loose records, then compact them into immutable,
checksummed pack files. A pack is written to a temporary file, fully verified and
fsynced, atomically renamed, then indexed transactionally. Packs are never modified;
compaction writes replacements and retires old packs after reader leases and a recovery
grace period. Pack indexes are independently checksummed and rebuildable by scanning.

Blob payloads through 64 KiB may be inline. Larger payloads use deterministic FastCDC
content-defined chunking targeting 1 MiB average chunks, initially bounded to 256 KiB
minimum and 4 MiB maximum. Chunk parameters are stored/versioned in the blob schema.
Chunks and complete blob recipes are content-addressed logical objects. Compression
and authenticated encryption happen per stored record/envelope after canonical ID
calculation. Transfers can resume only at verified record/chunk boundaries.

## Consequences

- Readers require no pack write locks; crashes leave either old or new complete packs.
- Content-defined chunks improve deduplication for insertions in large artifacts, with
  additional CPU and small-object overhead.
- Physical duplicate ciphertext may be retained across policy domains to prevent a
  cross-domain deduplication oracle.
- GC must account for references, active transactions/readers, key epochs, backups,
  quarantined input, and recovery grace periods.

## Rejected alternatives

- Mutable append-only packs as the only store: harder crash truncation and concurrency.
- Fixed-size chunks: simpler, but poor reuse after insertions.
- One object per whole large file: poor streaming, sync resumption, and delta reuse.

## Verification and open work

Benchmark parameters before format freeze; test power loss at every publish/retire
step, truncated/corrupt packs, hostile compression ratios, chunk-boundary determinism,
leases, and policy-safe deduplication. Exact pack record/table layout is deferred to a
Milestone 1 format specification.
