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

Blob payloads through and including 64 KiB MUST be inline. Larger payloads MUST use deterministic FastCDC
profile 0 with an exact 1 MiB target, 256 KiB minimum, and 4 MiB maximum. Chunk
parameters are stored/versioned in the blob schema and must equal this profile.
Chunks and complete blob recipes are content-addressed logical objects. Compression
and authenticated encryption happen per stored record/envelope after canonical ID
calculation. Transfers can resume only at verified record/chunk boundaries.

FastCDC schema-0 algorithm/profile 0 is frozen as follows:

- gear seed `0x7267697466636463`; entry `G[i]` is SplitMix64 of
  `seed + (i + 1) * 0x9e3779b97f4a7c15`, using xor-shifts 30/27/31 and multipliers
  `0xbf58476d1ce4e5b9` and `0x94d049bb133111eb`, with wrapping `u64` arithmetic;
- early mask `(1 << 21) - 1`, late mask `(1 << 19) - 1`, normalization level 1;
- begin a chunk with rolling value zero. After appending a byte, do nothing while
  length is below 256 KiB. At and above that length update
  `H = rotate_left(H, 1) + G[byte]` using wrapping `u64` addition;
- use the early mask below 1 MiB and late mask at or above 1 MiB. Cut after the
  current byte when `H & mask == 0`, or unconditionally at 4 MiB, then reset `H`;
- finalization emits one pending nonempty chunk even below the minimum. Empty input
  emits no chunks and is represented as an empty inline Blob. Complete content at or
  below 64 KiB is inline; larger content uses the FastCDC recipe;
- every ChunkRef length is in `1..=4 MiB`; references remain in stream order and
  their checked sum must equal Blob length. Reader buffer segmentation must not
  affect boundaries, Chunk IDs, the Blob recipe, or Blob ID.

`crates/rgit-objects/tests/vectors/fastcdc-v0.json` is the cross-platform known-answer
set for empty, small, repetitive, periodic, and deterministic pseudorandom inputs.

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

Test power loss at every publish/retire step, truncated/corrupt packs, hostile
compression ratios, leases, and policy-safe deduplication. Exact pack record/table
layout is deferred to a Milestone 1 format specification. FastCDC parameters and
boundary behavior are format-frozen and may change only under a new profile number.
