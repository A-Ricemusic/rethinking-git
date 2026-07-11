# RGit Object Model, Version 0

Status: initial normative specification  
Audience: object-store, graph, policy, crypto, workspace, sync, and compatibility implementers  
Last updated: 2026-07-11

## 1. Purpose and conformance

RGit is an immutable object graph with transactional mutable references. It is not a
Git object graph with additional access-control fields. This document defines the
logical objects that all implementations must agree on before storage encryption,
database layout, or network framing is considered.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are normative. Version
0 is an implementation target, not a promise that the format is frozen forever.
Incompatible changes require a new schema version and migration plan.

An implementation conforms when it:

- accepts and emits only canonical encodings described in
  [canonical-encoding.md](canonical-encoding.md);
- derives object IDs from the canonical logical plaintext object;
- validates every referenced object's expected kind before publishing a reference;
- evaluates authorization before disclosing an object ID or object metadata;
- treats unknown required fields, kinds, algorithms, and schema versions as errors;
- never treats storage envelopes, indexes, or redacted projections as canonical
  logical objects unless this specification assigns them an object kind.

## 2. Foundational types

### 2.1 Object ID

`ObjectId` is a tuple `(format_version, hash_algorithm, digest)` encoded as described
in ADR 0002. The digest covers a domain separator, object kind, schema version, and
canonical payload. Consequently, identical bytes used as different object kinds do
not share an ID.

Object IDs are not secrets. Policy evaluation MUST occur before an ID is revealed,
because possession of a digest enables confirmation attacks against guessed content.

### 2.2 Stable IDs

`ChangeId`, `LineId`, `PolicyId`, `ActorId`, and `DeviceId` are 128-bit random values.
They identify evolving logical entities and MUST come from a cryptographically secure
random generator. They are distinct types and MUST NOT be interchangeable. Display
names are mutable labels and are never identity.

### 2.3 Time

`LogicalTime` is an unsigned 64-bit counter interpreted only within the operation
graph. `WallTime` is an RFC 3339 instant normalized to UTC seconds plus signed numeric
offset captured at entry. Wall time is informational and MUST NOT establish operation
ordering, authorization, freshness, or signature validity.

### 2.4 Policy reference

Every discoverable logical object has a `policy_ref` containing a `PolicyId` and an
exact policy object version ID. Policy inheritance is resolved when the object is
created; a child MAY be more restrictive than its parent but MUST NOT silently become
less restrictive. Derived-data rules are specified by the referenced policy.

### 2.5 Signature

A signature record contains the signature algorithm, signer `ActorId`, signing-key
ID, signature bytes, and signed-purpose string. Signatures cover a domain-separated
canonical object or transaction statement, never a display serialization.

## 3. Common object header

Every hashed object begins with:

| Field | Type | Rule |
| --- | --- | --- |
| `kind` | closed enum | Must match the hash domain and expected reference kind. |
| `schema_version` | unsigned integer | Version of that object kind, initially `0`. |
| `policy_ref` | policy reference | Omitted only for the repository root policy. |

Extension data is not an untyped map. Optional extensions use registered numeric
field keys and declare whether an older reader may safely ignore them. Unknown
critical fields make the object unsupported.

## 4. Content and filesystem objects

### 4.1 Blob

A `Blob` represents logical file bytes:

- `byte_length`: unsigned 64-bit length;
- exactly one of `inline_bytes` (up to 64 KiB) or `chunks`;
- `chunks`: ordered `ChunkRef` values containing a chunk ID and plaintext length;
- optional non-authoritative `content_hint` such as MIME type;
- `policy_ref`.

A `Chunk` contains raw bytes and has its own content-derived object ID. Chunking is a
storage and streaming concern; concatenating chunk plaintext MUST produce exactly
`byte_length` bytes. A blob ID is independent of storage compression and encryption.

### 4.2 Secret reference

`SecretRef` versions configuration without pretending that version control is a
general secret manager:

- provider kind and provider-neutral locator;
- optional exact secret version locator;
- value schema ID and materialization variable/path;
- policy reference and required materialization capability;
- optional encrypted development value reference.

Commands MUST redact provider locators if their policy denies discovery. Resolved
secret plaintext MUST NOT enter snapshots, diffs, operation payloads, caches, or logs.

### 4.3 Path segment

Paths are arrays of normalized UTF-8 `PathSegment` values, not host path strings.
Segments MUST be valid UTF-8 in Unicode NFC, non-empty, and must not be `.`, `..`,
contain NUL, `/`, or `\\`. Absolute paths and platform prefixes are invalid. The
portable profile additionally rejects Windows reserved names, trailing spaces/dots,
and paths that collide under Unicode default case folding.

Repositories declare the portable profile by default. A non-portable repository MUST
record its filesystem profile and may be impossible to materialize on another host.

### 4.4 Manifest

A `Manifest` is one directory. Entries are sorted by the canonical encoded path
segment and contain:

- `name`: one `PathSegment`;
- `entry_kind`: `file`, `directory`, `symlink`, `subproject`, or `secret_ref`;
- typed object reference;
- `mode`: `regular` or `executable` for files; absent otherwise;
- exact `policy_ref` effective for the entry.

Duplicate names and case/normalization collisions are invalid. Symlink content is a
blob containing the uninterpreted target. Materialization MUST reject a symlink that
would escape the workspace. A `Subproject` records a system kind, immutable external
repository identity, exact revision, and optional native RGit projection ID.

Empty directories MAY be represented by an empty manifest. Special files, device
nodes, sockets, ACLs, and platform extended attributes are excluded from schema 0.

## 5. History objects

### 5.1 Snapshot

A `Snapshot` is an immutable project state:

- `root_manifest`: manifest ID;
- ordered, duplicate-free `parents`: zero or more snapshot IDs;
- `change_id`: owning stable change;
- author `ActorId` and `DeviceId`;
- `logical_time` and informational `wall_time`;
- optional message blob;
- `policy_ref`.

Parent order is significant: parent 0 is the integration/mainline parent. A snapshot
does not grant access to its parents or children. Implementations MUST tolerate a
policy-filtered graph in which an otherwise valid adjacent object is undiscoverable.

### 5.2 Change revision and change state

A `ChangeRevision` is immutable and contains:

- stable `change_id`;
- previous revision ID, absent for the first revision;
- title and description blob IDs;
- base and current snapshot IDs;
- target `LineId` and observed generation;
- owner and author actor IDs;
- state: `open`, `abandoned`, `landed`, or `superseded`;
- review/landing policy references;
- `policy_ref`.

The mutable lookup from `ChangeId` to latest `ChangeRevision` is a transactional
reference, not a hashed object. Split and combine operations create explicit relation
objects so provenance is retained. A stable change ID MUST survive ordinary snapshot,
description, and retarget updates.

### 5.3 Line state

`LineState` is the signed value installed through compare-and-swap for a stable
`LineId`:

- display name;
- head snapshot ID;
- monotonically increasing unsigned 64-bit generation;
- previous line-state object ID;
- integration, approval, release, and visibility policy references;
- transaction operation ID and signature.

A line update is valid only when the expected generation and previous state both
match. Generation overflow permanently freezes the line pending a format migration.
The names `main`, `release`, and similar have no special storage semantics.

### 5.4 Typed marker

A `Marker` contains a marker kind (`release`, `deployment`, `review`, `policy`, or
`bookmark`), target object ID and kind, issuer, issue time, policy, typed payload, and
signature. Marker meaning is determined by its kind and schema; clients MUST NOT infer
release or approval semantics from a free-form name.

### 5.5 Release and audience projection

A `Release` is immutable and contains:

- source line ID, generation, and exact source snapshot;
- audience policy and projection-rules object IDs;
- projected root manifest and projection proof;
- version identifier and optional release notes blob;
- build provenance and artifact references;
- policy decision evidence, issue time, and one or more signatures.

Projection creates new manifests containing only audience-readable content. It MUST
fail closed when a reachable entry has no applicable rule. A projection proof binds
the source snapshot, rule set, audience policy version, and projected root without
revealing excluded object IDs. A release signature covers the entire release object.

## 6. Policy and identity objects

### 6.1 Policy

A `Policy` is immutable and identified by `(PolicyId, version object ID)`. It contains:

- version sequence and previous policy version;
- principals expressed as actor, group, role, or service identities;
- grants for `discover`, `read`, `materialize`, `derive`, `review`, `integrate`,
  `release`, `administer`, and `audit`;
- redaction mode: `omit`, `opaque_placeholder`, or `typed_summary`;
- derivation rule and declassification requirements;
- key epoch and key-envelope set reference;
- administrators, activation constraints, and signatures.

Denials take precedence over grants. Absence of a grant is denial. Policy evaluation
uses an exact signed policy version; clients MUST NOT substitute a newer or older
version. Schema 0 does not permit arbitrary executable policy code.

### 6.2 Identity and membership

An `Identity` binds an `ActorId` or `DeviceId` to public signing and encryption keys,
validity bounds, issuer, status, and signatures. `GroupMembership` is a signed,
versioned statement with validity bounds. Identity revocation prevents future access
and signatures from being accepted after the effective boundary; it cannot retract
plaintext or object IDs already delivered.

## 7. Collaboration and audit objects

### 7.1 Conflict

A `Conflict` stores typed base/left/right object references, affected path, conflict
kind, merge-driver identity/version, optional structured regions, and policy. Its
effective policy MUST be at least as restrictive as every input. A resolution creates
a new resolution object; it does not mutate the conflict.

### 7.2 Operation

An `Operation` records every state-changing command:

- operation ID derived from the canonical object;
- ordered parent operation IDs;
- actor, device, logical time, and informational wall time;
- typed action records with before/after references;
- recovery or inverse payload references where reversal is supported;
- public/redacted envelope and private/audit payload references;
- policy, signature, and client implementation identity.

Operations form a DAG. A local journal transaction may contain several object writes
and reference compare-and-swaps but publishes one operation as their audit boundary.
Undo creates a new operation; existing operations are never deleted or rewritten.

### 7.3 Review, approval, and CI evidence

Review decisions, approvals, and CI results are immutable typed evidence objects. They
bind the exact snapshot, policy/ruleset version, issuer identity, outcome, validity
conditions, and signature. Advancing a change invalidates evidence unless its policy
explicitly permits a compatible successor.

## 8. Storage envelopes and views

`StorageEnvelope` is a non-logical record containing logical object ID, cryptographic
suite, nonce, ciphertext, authenticated header, compression information, key epoch,
and recipient key envelopes. Re-encryption and key rotation MAY create many envelopes
for one logical object ID. Servers MUST verify authorization independently of whether
they possess a matching envelope.

A `View` is an evaluated, signed projection for an actor/device at exact policy and
line generations. It is not permission to fetch arbitrary referenced IDs. Omitted and
redacted entries MUST not expose restricted IDs, sizes, timestamps, names, graph
shape, or change frequency unless the applicable redaction policy explicitly allows
those fields.

## 9. Reference validation and publication

Before publishing a mutable reference, an implementation MUST atomically establish:

1. every newly referenced immutable object is durably stored and digest-verified;
2. every typed reference resolves to an object of the expected kind and supported
   schema for the publishing actor's view;
3. effective policies satisfy derivation and no-downgrade constraints;
4. required signatures and policy decisions verify at the transaction boundary;
5. compare-and-swap preconditions still hold;
6. the operation and updated indexes commit together.

Missing restricted objects are distinguishable from corruption only to actors with
`audit` permission. Other clients receive a policy-safe unavailable result.

## 10. Schema evolution

Each object kind evolves independently. Additive fields require registered keys and
defined default semantics. A writer MUST emit the lowest schema version that exactly
represents the object. Readers preserve unknown non-critical extensions byte-for-byte
when proxying but MUST NOT reinterpret them. Hash or encoding changes create new IDs;
repository migration records old-to-new mappings in signed migration objects.

## 11. Explicit non-goals and open questions

Schema 0 does not define remote protocol framing, a policy language with conditions,
cross-repository object identity, semantic merge formats, transparency-log anchoring,
or confidential-computing enforcement. These require later specifications and ADRs.

Before schema 0 is frozen, test vectors must settle numeric field assignments, maximum
object/reference counts, projection-proof construction, group-membership evaluation
time, and whether inline blobs are worth their additional representation.
