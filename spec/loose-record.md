# Loose Record and Repository Layout, Version 0

Status: normative Milestone 1 specification

Decision records: [ADR 0001](../docs/adr/0001-canonical-object-encoding.md),
[ADR 0002](../docs/adr/0002-hash-algorithm-agility.md),
[ADR 0003](../docs/adr/0003-embedded-metadata-database.md),
[ADR 0004](../docs/adr/0004-immutable-packs-and-chunks.md), and
[ADR 0009](../docs/adr/0009-supported-platforms-and-filesystems.md)

## 1. Scope and conformance

This document freezes the schema-0 physical format and publication protocol for one
canonical logical object stored as an immutable loose record. The logical payload,
object kinds, and object IDs remain governed by [the object specification](objects.md)
and [the canonical-encoding specification](canonical-encoding.md). A loose record is
not a new logical object and is never referenced by its physical checksum or path.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are normative. A
version-0 loose-store implementation conforms only if it uses the byte grammar,
layout, verification, exclusive publication, durability, and recovery rules below.
It MUST NOT claim crash durability on a filesystem that has not passed the startup
capability probe required by ADR 0009.

This format stores canonical plaintext. Compression, authenticated encryption,
recipient envelopes, packs, and remote quarantine use different physical formats and
are outside this version. They MUST NOT be smuggled into version 0 with flags or an
alternate interpretation of `payload`.

## 2. Version-0 record grammar

A loose record is exactly:

```text
magic[8]                    = 52 47 49 54 4c 4f 4f 53 (ASCII "RGITLOOS")
record_format               = uvarint(0)
object_id                   = complete binary ObjectId
object_kind                 = uvarint
object_schema               = uvarint
canonical_payload_length    = uvarint
canonical_payload           = byte[canonical_payload_length]
checksum                    = byte[32]
```

`ObjectId` is self-delimiting and uses its complete binary grammar:

```text
id_format || hash_code || digest_length || digest[digest_length]
```

All seven numeric fields shown in the two grammars use shortest-form unsigned LEB128,
as in the canonical object hash preimage. No fixed-width integer exists in this
format, so endianness has no other application. A decoder MUST reject an overflowing,
unterminated, longer-than-10-byte, or non-minimal `u64` varint. Version 0 accepts only
ObjectId format 0, registered hash codes `0x12` and `0x1e`, and a 32-byte digest.
It accepts only registered object kinds and supported schema versions. Reserved or
unknown values fail closed.

`canonical_payload` is the byte-for-byte deterministic CBOR object. The repeated
kind and schema MUST equal fields 0 and 1 in that payload, the values used in the
ObjectId hash preimage, and the kind/schema expected by a typed caller. The length is
the canonical payload length alone; it does not include framing or checksum.

The trailing checksum is:

```text
SHA-256("RGIT-LOOSE-CHECKSUM\0" || every preceding record byte)
```

The separator includes its NUL. The checksum detects physical corruption and framing
mistakes; it does not replace recomputing the logical ObjectId and provides no
authenticity. A future checksum change requires a new `record_format`.

There are no padding bytes, alignment gaps, optional fields, flags, or trailing data.
The expected file length is calculated with checked arithmetic before payload
allocation and MUST exactly equal the actual regular-file length.

## 3. Admission and allocation limits

The canonical payload ceilings in `canonical-encoding.md` apply before allocation:

| Kind | Maximum canonical payload |
| --- | ---: |
| Chunk (1), Blob (2) | 16,777,216 bytes |
| Every other schema-0 kind | 1,048,576 bytes |

The conservative absolute pre-parse ceiling is 16,777,358 bytes: the maximum payload
plus 8 magic bytes, seven maximum-width `u64` varints, a 32-byte schema-0 digest, and
the 32-byte checksum. No valid current record reaches that size because registered
format, algorithm, kind, and schema values have shorter encodings. After this first
ceiling, exact framing, minimal encodings, the fixed 32-byte digest, exact file length,
and the tighter kind profile apply.
Implementations MUST also enforce the CBOR byte-string, text, collection, and depth
ceilings from the canonical specification. They MAY set a documented stricter
deployment limit but MUST NOT relax a schema ceiling.

Readers MUST parse the fixed prefix and bounded varints into small stack buffers.
They MUST reject the advertised payload length before reserving or mapping it, use
checked conversions to platform sizes, and never preallocate from the file length or
an untrusted CBOR count without the corresponding limit check. Version 0 is stored
uncompressed, so decompression and compression-ratio limits do not apply.

## 4. Repository layout and path derivation

The production repository control directory has this version-0 storage layout:

```text
.rgit/
  format
  metadata/
    repository.sqlite3
    repository.sqlite3-wal       # present only while SQLite uses it
    repository.sqlite3-shm       # present only while SQLite uses it
  objects/
    loose/
      <id-format-decimal>/
        <hash-code-lower-hex>/
          <first-digest-byte-lower-hex>/
            <remaining-digest-lower-hex>.rgl
    tmp/
      put-<32-lower-hex-random>
    quarantine/
      record-<32-lower-hex-random>.rgl
```

`format` and the SQLite schema are specified by the repository-format work, not by
this record grammar. They MUST record a repository format that opts into loose-record
version 0 before these files are interpreted.

The final path is a pure function of the parsed full ObjectId tuple. Numbers have no
prefix or leading zero except that the hash code is exactly two lowercase hexadecimal
digits for registered schema-0 algorithms. The 32-byte digest is lowercase hex; its
first byte is the two-character fan-out directory and the remaining 31 bytes form the
62-character filename. The filename suffix is `.rgl`. Thus the BLAKE3 vector in
section 10 is stored at:

```text
.rgit/objects/loose/0/1e/fc/b8cf563145b1628a69e25e3a775d0d11cf3ae40cab63cc3bf94af1bfcbb166.rgl
```

Readers derive this path internally. They MUST NOT accept a caller-supplied relative
or absolute object path, follow symlinks/reparse points in `.rgit`, or use an
abbreviated/text ObjectId as durable lookup input. A verified record whose embedded
ObjectId does not derive its actual path is corrupt even when all other checks pass.

Temporary names contain 128 bits from the operating system CSPRNG, use `create_new`
semantics, and disclose no ObjectId. A collision generates a new name. Implementations
MUST NOT use a system temporary directory: `tmp`, `loose`, `quarantine`, metadata,
and the `.rgit` root MUST be on the same supported local filesystem/volume. Startup
compares POSIX device IDs or Windows volume identity and fails safely on a mismatch.
New directories are created with owner-only access, without traversing links, and
their creation is made durable before they receive a published record.

## 5. Verified read behavior

Authorization is resolved before an object path is opened or its presence is probed.
After authorization, a reader MUST perform all of these checks:

1. Open a regular, non-link file without traversing symlinks or reparse points and
   reject a file outside the repository's pinned filesystem identity.
2. Apply the absolute pre-parse size ceiling, then parse magic, record format, the
   complete ObjectId, kind, schema, and payload length with minimal-varint checks.
3. Apply the kind-specific payload ceiling and require the checked expected file
   length to equal the observed length exactly.
4. Stream the frame through the domain-separated SHA-256 checksum and compare the
   trailing checksum in constant time.
5. Strictly decode the payload, validate all schema invariants and resource limits,
   re-encode it, and require byte equality.
6. Require the payload kind/schema, frame kind/schema, typed request (when present),
   ObjectId hash domain, and path derivation to agree.
7. Recompute the ObjectId with its declared registered algorithm and require exact
   equality with the complete embedded ObjectId.

Only then is the object **verified**. No unverified payload byte may be returned to a
logical decoder, materializer, indexer, or caller. A streaming API therefore returns
a verified handle only after a full verification pass and rewinds/reopens the pinned
file for streaming; the second pass MUST ensure it is reading the same open file
identity and immutable length. Alternatively it may stream from a bounded verified
spool. Reopening an unpinned pathname after verification is forbidden.

The record checksum, embedded ID, and final path are deliberately redundant. Passing
only one or two of them is not sufficient. Non-auditors receive one policy-safe
unavailable result for denied, absent, quarantined, and corrupt objects; detailed
diagnostics are restricted to local administrators/auditors.

## 6. Immutable publication protocol

One bounded store writer performs publication. For each candidate it MUST execute
the following order, with failure injection points between every numbered step:

1. Canonically encode and validate the logical object, select the repository's
   configured write algorithm, compute the full ObjectId, and construct the frame.
2. Create a random file in `objects/tmp` with exclusive `create_new` and owner-only
   permissions. Opening an existing path, link, or reparse point is an error.
3. Write the complete frame with short-write/EINTR handling. A zero-progress write is
   an error. Do not expose a partially written file under the final path.
4. Flush file content and metadata using the Tier-1 platform primitive, then close or
   otherwise establish a stable read handle.
5. Verify the temporary file through the complete section 5 pipeline, including its
   logical ID, before attempting publication.
6. Create and durably flush any missing fan-out directories, then atomically rename
   the temporary file to its derived final path with **no replacement** semantics.
7. Flush the final parent directory (and any newly created ancestor) using the tested
   platform durability adapter. Publication is not durable until this succeeds.
8. Only after the loose file is durable, begin and commit the SQLite transaction that
   registers its location/index data and publishes any operation/reference that may
   reach it. Reference rows, operation rows, and their index changes commit together
   with `synchronous=FULL` where required by ADR 0003.

If step 8 fails, the file remains a safe unindexed orphan; it MUST NOT be removed as
rollback. If any earlier step fails, no database transaction may expose the object.
Cancellation is disabled or deferred across steps 6--8, and an error reports the
last durable boundary rather than pretending rollback removed published bytes.

The writer MUST retain the verified temporary file descriptor (or an equivalent
pinned file-identity handle) continuously from the verification in step 5 through
the no-replace rename in step 6. It MUST compare that retained identity with the
published destination before releasing the handle. Closing the verified descriptor
and then renaming by pathname alone is non-conforming because an attacker or race
could substitute a different temporary file between verification and publication.

The rename operation MUST be same-filesystem, atomic, and exclusive. A generic rename
API that can silently replace its destination is non-conforming. Implementations MUST
use the platform's tested no-replace primitive and treat “destination exists” as the
deduplication/collision path below.

### 6.1 Existing destination and collision handling

An immutable final file is never overwritten, truncated, repaired in place, or used
as a rename target with replacement enabled.

When the final path already exists, the writer keeps its verified candidate temp and
independently verifies the existing final record. If both records are fully valid,
carry the expected complete ObjectId, and have identical frame bytes, the write is a
successful deduplication; the temp is deleted and its directory entry is flushed.

Every other result is a collision/corruption incident. This includes different
canonical bytes that recompute to the same ObjectId, an invalid existing record, an
embedded/path ID mismatch, or different framing for the same schema-0 logical object.
The writer MUST:

- leave the existing final path untouched;
- atomically move the candidate temp into `objects/quarantine` under a random name
  and durably flush that directory;
- record restricted evidence containing both observed identities and verification
  failures without copying plaintext into ordinary logs;
- fail the mutation and place reference publication/store writes in fail-closed
  read-only incident mode until an authorized repair workflow resolves it.

Quarantine files are never object lookup candidates, never indexed as reachable, and
never automatically restored. Repair is non-destructive and produces an audit record.

## 7. Tier-1 platform semantics

All platforms reject links/reparse points in control-directory traversal and pin
opened-file identity across verification. Permission errors, sharing violations, a
read-only volume, and unsupported durability primitives are typed failures, never a
reason to weaken the sequence.

### 7.1 POSIX (Linux and macOS)

- Create temp files using directory-relative `openat`-style traversal with
  `O_CREAT|O_EXCL|O_NOFOLLOW` and mode `0600`; verify the resulting descriptor is a
  regular file with link count one.
- Complete writes, call `fdatasync` when it is documented sufficient for the target
  filesystem or otherwise `fsync`, then verify from the pinned descriptor.
- Publish exclusively using `renameat2(RENAME_NOREPLACE)` on Linux or
  `renameatx_np(RENAME_EXCL)` on macOS. A deployment lacking the selected primitive
  fails its capability probe; it MUST NOT fall back to an overwriting rename.
- `fsync` the final directory descriptor after rename. When creating hierarchy,
  flush each child and its parent from the leaf toward `.rgit`. Flush `tmp` after
  temp removal and `quarantine` after quarantine publication.

Signals/process termination may interrupt user-space work but cannot reverse a
successfully flushed rename. Startup recovery handles every incomplete boundary.

### 7.2 Windows on local NTFS

- Traverse handles relative to the repository root, reject reparse points, create
  the temp with `CREATE_NEW`, owner-restricted security, and sharing flags compatible
  with later rename but not replacement.
- Complete writes and call `FlushFileBuffers` on the temp handle before verification.
- Rename on the same volume with no-replace semantics and a write-through operation
  (for example `MoveFileExW` with `MOVEFILE_WRITE_THROUGH` and without
  `MOVEFILE_REPLACE_EXISTING`, after the adapter has pinned and revalidated both
  parent paths); never pass a replace-existing flag. Flush the renamed file and use
  the project's tested NTFS directory/volume durability adapter before database
  commit.
- Antivirus/indexer sharing violations MAY receive a bounded, jittered retry before
  the rename boundary. Exhaustion is a typed publication failure. It MUST NOT trigger
  copy-and-delete, replacement, or an early metadata commit.

The Windows adapter MUST be crash-tested on every supported Windows release. If the
host/NTFS combination cannot provide and verify the required write-through rename
and metadata durability behavior, the repository opens read-only with a diagnostic;
successful API return alone is not treated as durability evidence.

## 8. Startup recovery and verification

Recovery acquires the repository writer lock before changing storage state. It is
idempotent and interruption-safe:

- A file in `objects/tmp` is never reachable. Recovery verifies it only for a repair
  report, then removes it after a configurable grace period and flushes `tmp`.
  Recovery does not infer an intended database transaction from a temp record.
- A valid final loose record absent from the metadata inventory is the expected
  remnant of a crash between steps 7 and 8. Recovery verifies it and transactionally
  registers it as an **unreferenced orphan**. It does not create a line, change,
  operation, or other mutable reference. Normal reachability/operation recovery may
  later adopt it; garbage collection observes the recovery grace period.
- An inventory row whose final file is absent or invalid is corruption. Recovery
  marks it unavailable, emits a restricted non-destructive repair report, and blocks
  every reference transaction that could expose it. It never fabricates bytes from
  index data.
- A valid record at the wrong path, a non-regular entry, unexpected file, invalid
  frame, checksum failure, canonical failure, or ID mismatch is quarantined or
  reported without overwriting either candidate. Already referenced corrupt content
  forces fail-closed read-only incident mode.
- Quarantine is inventoried for auditors but never scanned into the logical store.
  Old quarantine data is retained or destroyed only by an explicit audited policy.

Rebuildable indexes may be discarded and reconstructed exclusively from fully
verified final records and authoritative signed graph/reference data. Recovery MUST
prove every published reference's transitive required objects before reopening for
writes. A missing restricted object is disclosed as corruption only to an authorized
auditor, as required by the object specification.

## 9. Confidentiality and envelope boundary

Loose-record version 0 is suitable only for an access-controlled local repository
whose host and storage are inside the plaintext trust boundary. It does **not** make
per-object authorization enforceable against a disk, backup, administrator, malware,
or cloud-sync operator that can read `.rgit`.

Even after encrypted `StorageEnvelope` support exists, physical layout can reveal
ObjectIds, equality, approximate payload size, object count, fan-out population,
write timing, access timing, and key/policy epochs. Naming plaintext records by their
logical digest also enables confirmation attacks on guessed content. Separate policy
domains may therefore require randomized envelope names, padding, duplicate
ciphertext, and segregated indexes. Authorization MUST precede existence checks, and
paths, IDs, sizes, timing, quarantine details, and differentiated errors MUST NOT be
exposed to an unauthorized caller.

`StorageEnvelope` is explicitly not a logical object kind. Its authenticated header,
nonce, compression, ciphertext, key epoch, recipients, naming, and leakage controls
require a separate normative physical-record specification. Implementers MUST NOT
claim that this plaintext loose format satisfies the vision's hostile-storage or
fine-grained secret-at-rest goals.

## 10. Byte-level conformance vectors

These vectors reuse the canonical Chunk payload from
`crates/rgit-objects/tests/vectors/chunk-v0.json` and its BLAKE3 ObjectId from the
same fixture's dual-algorithm entry in
`crates/rgit-objects/tests/vectors/schema-v0.json`.

| Field | Value |
| --- | --- |
| Magic | `524749544c4f4f53` |
| Record format | `00` |
| Complete ObjectId | `001e20fcb8cf563145b1628a69e25e3a775d0d11cf3ae40cab63cc3bf94af1bfcbb166` |
| Kind | `01` (Chunk) |
| Schema | `00` |
| Payload length | `44` (ULEB128 for decimal 68) |
| Canonical payload | `a40001010002a200500000000000000000000000000000000001582300122000000000000000000000000000000000000000000000000000000000000000000343616263` |
| Checksum | `06967a7dc69492425c9d81d219dd62ab37434c8e3690f00686fd6d7c47059e06` |

The complete 147-byte record is:

```text
524749544c4f4f5300001e20fcb8cf563145b1628a69e25e3a775d0d11cf3ae40cab63cc3bf94af1bfcbb166010044a40001010002a20050000000000000000000000000000000000158230012200000000000000000000000000000000000000000000000000000000000000000034361626306967a7dc69492425c9d81d219dd62ab37434c8e3690f00686fd6d7c47059e06
```

Offsets are zero-based: magic `0..8`, record format `8`, ObjectId `9..44`, kind
`44`, schema `45`, payload length `46`, payload `47..115`, and checksum `115..147`.
The fixture's derived final path is shown in section 4.

Conformance suites MUST include at least these one-variable corruption cases. Unless
specified otherwise, mutation without checksum repair fails the checksum as well as
the named invariant; implementations must test a repaired-checksum variant to prove
the deeper check is independently enforced.

| Mutation | Required result |
| --- | --- |
| Flip magic byte 0 | Reject bad magic before payload allocation. |
| Set record format byte 8 to `01` | Reject unsupported record format. |
| Encode record format 0 as `80 00` | Reject non-minimal varint and shifted framing. |
| Set ObjectId format byte 9 to `01` | Reject unsupported ObjectId format. |
| Set hash byte 10 to `13` | Reject unknown hash algorithm. |
| Set digest length byte 11 to `1f` or `21` | Reject invalid schema-0 digest length. |
| Flip digest byte 12 and repair checksum | Reject recomputed ObjectId/path mismatch. |
| Set frame kind byte 44 to `02` and repair checksum | Reject payload/frame kind mismatch. |
| Set frame schema byte 45 to `01` and repair checksum | Reject unsupported/mismatched schema. |
| Set payload length byte 46 to `43` | Reject exact file-length mismatch/trailing byte. |
| Set payload length byte 46 to `45` | Reject truncation before checksum. |
| Encode length 68 as `c4 00` | Reject non-minimal length varint. |
| Advertise 16,777,217 for Chunk | Reject the allocation request before allocation. |
| Change payload field 0 from kind 1 to 2 and repair checksum | Reject canonical payload/frame mismatch and ObjectId mismatch. |
| Change canonical `00` to non-minimal CBOR `18 00`, adjust length, and repair checksum | Reject non-canonical CBOR even if semantic value matches. |
| Flip payload byte 114 and repair checksum | Reject logical ObjectId mismatch. |
| Flip checksum byte 115 | Reject checksum mismatch. |
| Truncate any byte, including checksum byte 146 | Reject exact length/truncation. |
| Append `00` after byte 146 | Reject trailing data. |
| Store valid bytes under a different derived filename | Reject path/ObjectId mismatch. |
| Replace the final file with a symlink/reparse point | Reject without following it. |

Tests MUST exercise fragmented reads at every frame boundary and every payload byte,
short writes, zero-progress writes, concurrent identical publishers, concurrent
hostile destination creation, disk-full at every phase, permission loss, process
termination after every numbered publication step, and recovery repeated after each
recovery-side interruption.

## 11. Pack deferral and evolution

This document does not define a pack header, record table, index, compression codec,
encryption envelope, pack checksum, lease, retirement protocol, or transfer frame.
Those remain deferred as required by ADR 0004. A pack implementation may reuse
logical payloads and ObjectIds but MUST NOT label a pack entry as loose-record version
0 unless its bytes exactly match this grammar.

Loose records remain the crash-safe ingestion baseline after packs arrive. Compaction
publishes a completely verified immutable pack before transactionally switching
location indexes, and retains old loose records until reader leases and recovery
grace periods expire. Pack evolution never changes logical ObjectIds or permits a
reference to become reachable before at least one verified durable representation
exists.
