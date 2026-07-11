# RGit SQLite Metadata Store, Version 1

Status: initial normative specification; implementation is not claimed

Audience: storage, graph, recovery, platform, and security implementers

Last updated: 2026-07-11

## 1. Scope and authority

This document specifies the version-1 SQLite metadata database used by a local RGit
repository. It refines [ADR 0003](../docs/adr/0003-embedded-metadata-database.md)
and the database boundary named by the
[loose-record specification](loose-record.md). The immutable logical object graph is
defined by [objects.md](objects.md); canonical object bytes and loose-record files,
not rows reconstructed from this database, determine an object's ID and content.

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are normative. This is a
storage specification, not evidence that the described backend has been implemented
or crash-qualified.

The database is authorization-neutral, as is the current `rgit-store::Store`
contract. Authorization MUST occur before a caller uses that contract to probe an
object ID, reference key, location, size, kind, edge, or error detail. Ordinary
callers MUST receive a policy-safe unavailable result rather than learning whether
denied content is absent, promised, quarantined, or corrupt.

### 1.1 Sources of truth

The following order resolves disagreement:

1. A verified immutable record is authoritative for canonical bytes, object ID,
   object kind, schema version, and outgoing typed edges.
2. Signed immutable Operation and LineState objects provide the auditable history of
   mutations.
3. The `references` table is the local atomic checkpoint of currently published
   mutable reference state.
4. Repository identity, schema migration history, promises, quarantine disposition,
   physical-pack inventory, durable incidents, GC tombstones, and active leases are
   durable local control state.
5. Object-location inventories and graph/query tables are derived indexes. They MUST agree
   with verified objects but MUST NOT be used to fabricate or repair object bytes.

Indexes described as rebuildable may be dropped and reconstructed from verified
records. A rebuild MUST preserve the reference checkpoint and local control state.

## 2. File identity and SQLite header

The sole metadata database is `.rgit/metadata/repository.sqlite3`. Its WAL and shared
memory files, when present, use SQLite's adjacent `-wal` and `-shm` names. The
database, `.rgit`, loose objects, packs, temporary objects, quarantine, and lock files
MUST be on the same supported local filesystem or volume. Network filesystems,
cloud-synchronized folders, and filesystem snapshots with unproven SQLite/WAL
semantics are unsupported.

The SQLite header fields are frozen as follows:

```text
PRAGMA application_id = 0x52474954; -- ASCII "RGIT", decimal 1380403540
PRAGMA user_version   = 1;          -- exact schema version in this document
```

An opener MUST read both fields before interpreting application tables. A nonzero
unknown `application_id` MUST be rejected as a foreign database. Zero is accepted
only by the atomic new-repository initialization path before any application table
exists. A `user_version` newer than the binary supports MUST open read-only with a
typed upgrade-required diagnostic; it MUST NOT guess at compatibility. A lower
supported version enters the migration protocol in section 11.

SQLite's own file-format version is independent of RGit's repository, loose-record,
object-schema, and metadata-schema versions. Implementations MUST NOT infer any one
from another.

## 3. Connection and filesystem profile

### 3.1 Required SQLite build

The application uses the project-pinned bundled SQLite through `rusqlite`. On every
connection it MUST verify a supported runtime library version and the compile
options required for STRICT tables, foreign keys, the online backup API, and the
defensive configuration in this section. Loading extensions is disabled. SQL from a
repository is never executed.

Before a writable open, the platform adapter MUST perform ADR 0009's identity,
link/reparse-point, same-volume, atomic no-replace rename, file flush, directory
flush, and local-filesystem probes. Probe results are keyed by actual filesystem and
volume identity, not merely an OS name. A failed or unknown durability probe permits
an explicitly diagnosed read-only inspection but not mutation.

### 3.2 Per-connection settings

Every connection MUST apply and verify:

```sql
PRAGMA foreign_keys = ON;
PRAGMA trusted_schema = OFF;
PRAGMA recursive_triggers = OFF;
PRAGMA temp_store = MEMORY;
PRAGMA busy_timeout = 5000; -- deployment may lower or boundedly raise this
```

The native API MUST additionally enable `SQLITE_DBCONFIG_DEFENSIVE`, disable extension
loading, install a bounded progress handler, and set conservative length, SQL text,
column, expression-depth, compound-select, variable, and attached-database limits.
The main repository connection MUST reject `ATTACH`, `VACUUM INTO` to an unapproved
path, writable-schema mode, and application-issued arbitrary SQL.

Writer connections MUST also apply and verify:

```sql
PRAGMA journal_mode = WAL;      -- returned value MUST be "wal"
PRAGMA synchronous = FULL;
PRAGMA wal_autocheckpoint = 0;  -- the writer coordinator checkpoints explicitly
PRAGMA journal_size_limit = 67108864;
```

`FULL` is mandatory for every transaction that publishes an object inventory row,
promise/quarantine transition, reference, operation checkpoint, migration, lease,
or GC tombstone. It MUST NOT be temporarily weakened. Read-only connections do not
change persistent pragmas and MUST use SQLite's immutable mode only for a genuinely
quiescent, checkpointed copy, never for a live WAL database.

After selecting WAL, startup performs a disposable transaction, checkpoint, database
file/WAL identity check, and reopen check. Failure to create adjacent WAL/SHM files,
obtain correct locks, checkpoint, or observe committed data fails the writable-open
probe. `DELETE` journal fallback is not automatic; it would require a future
repository format/profile and crash-test evidence.

## 4. Encodings and closed registries

All identifiers are binary:

- `object_id` is the complete self-delimiting binary `ObjectId`, never a digest alone
  or an abbreviated/text ID;
- `stable_id` is exactly 16 bytes for line, change, release, and marker references;
- the operation-head stable ID is the zero-length BLOB;
- repository and lease IDs are exactly 16 random bytes;
- hashes used to identify migrations are exactly 32 bytes.

Object kind is the closed schema-0 numeric registry from `rgit-objects::ObjectKind`:
1 through 31 (`Chunk` through `Ruleset`). Unknown numbers fail closed. Reference-key
kinds are local-store registry values:

| Code | Key | Stable ID length | Required target kind |
| ---: | --- | ---: | ---: |
| 1 | line | 16 | 8 (`LineState`) |
| 2 | change | 16 | 7 (`ChangeRevision`) |
| 3 | operation head | 0 | 10 (`Operation`) |
| 4 | release | 16 | 12 (`Release`) |
| 5 | marker | 16 | 11 (`Marker`) |

Edge roles are stored as stable lower-snake-case names, not Rust discriminants. The
complete version-1 closed registry is:

```text
object_policy
manifest_entry_policy
review_policy
landing_policy
integration_policy
approval_policy
release_policy
visibility_policy
audience_policy
blob_chunk
manifest_entry_target
snapshot_root_manifest
snapshot_parent
snapshot_message
change_previous_revision
change_title
change_description
change_base_snapshot
change_current_snapshot
line_head_snapshot
line_previous_state
line_transaction_operation
subproject_native_projection
secret_development_value
conflict_base
conflict_left
conflict_right
operation_parent
operation_before
operation_after
operation_line_policy
operation_line_head_snapshot
operation_line_previous_state
operation_line_integration_policy
operation_line_approval_policy
operation_line_release_policy
operation_line_visibility_policy
operation_inverse_payload
operation_public_envelope
operation_private_payload
marker_target
release_source_snapshot
release_projection_rules
release_projection_proof
release_projected_root
release_notes
release_build_provenance
release_artifact
release_policy_evidence
policy_previous_version
policy_declassification_requirement
policy_key_envelope_set
repository_root_policy
repository_trusted_identity
repository_bootstrap_key_envelope_set
repository_genesis_operation
repository_initial_line_state
identity_previous
identity_key_not_before
identity_key_not_after
identity_activation_operation
identity_not_after_operation
membership_previous
membership_activation_operation
membership_not_after_operation
change_relation_source
change_relation_result
change_relation_provenance
change_relation_operation
resolution_conflict
resolution_result
resolution_provenance
evidence_target
evidence_snapshot
evidence_ruleset
evidence_related
ci_runner_identity
ci_build_provenance
projection_rules_previous
projection_source_snapshot
projection_rules
projection_audience_policy
projection_manifest
build_snapshot
build_ruleset
build_identity
build_input
build_output
artifact_blob
payload_reference
view_policy
view_line_state
view_manifest
migration_old_object
migration_new_object
migration_tool_identity
ruleset_previous
```

This set corresponds exactly to the schema-0 `rgit_objects::ReferenceRole` registry.
The `edge_role_registry` rows installed by migration 1 are part of that migration and
MUST be compared to this complete set and the binary registry at open. An absent,
extra, or differently spelled row is corruption; applications MUST NOT accept a role
invented by database content. Adding a role requires a new metadata migration even
if the logical object schema changes first. `expected_kind` is copied from the
decoded `ReferenceEdge`; it is not guessed from the role string. Some roles
intentionally allow multiple target kinds and therefore store null.

Presence is closed: 1 `present`, 2 `promised`, 3 `quarantined`, 4 `tombstoned`.
Location kind is closed: 1 `loose`, 2 `pack`. Physical-pack state is closed: 1
`active`, 2 `retiring`, 3 `quarantined`. Incident state is closed: 1 `open`, 2
`resolved`. Lease kind is closed: 1 `reader`, 2 `transfer`, 3 `backup`, 4
`compaction`, 5 `gc`. Reference and object generations are unsigned
`u64`; because SQLite INTEGER is signed, valid stored values are 0 through
9,223,372,036,854,775,807. Reaching that ceiling freezes the affected mutation until
a format migration; wrapping or storing a negative value is forbidden.

## 5. Version-1 schema

The following DDL is normative. Migrations may build equivalent indexes under
temporary names, but a version-1 database after migration MUST have these columns,
constraints, indexes, and semantics. All application tables are STRICT and use no
implicit rowid unless explicitly useful.

```sql
CREATE TABLE repository (
    singleton             INTEGER PRIMARY KEY CHECK (singleton = 1),
    repository_id         BLOB    NOT NULL CHECK (length(repository_id) = 16),
    repository_format     INTEGER NOT NULL CHECK (repository_format >= 0),
    loose_record_format   INTEGER NOT NULL CHECK (loose_record_format = 0),
    write_hash_code       INTEGER NOT NULL CHECK (write_hash_code IN (18, 30)),
    revision              INTEGER NOT NULL CHECK (revision >= 0),
    root_object_id        BLOB,
    root_object_kind      INTEGER CHECK (root_object_kind = 14),
    incident_read_only    INTEGER NOT NULL DEFAULT 0
                                  CHECK (incident_read_only IN (0, 1)),
    FOREIGN KEY (root_object_id, root_object_kind)
        REFERENCES objects(object_id, kind),
    CHECK ((root_object_id IS NULL AND root_object_kind IS NULL) OR
           (root_object_id IS NOT NULL AND root_object_kind = 14))
) STRICT;

CREATE TABLE schema_migrations (
    version               INTEGER PRIMARY KEY CHECK (version > 0),
    name                  TEXT    NOT NULL UNIQUE,
    migration_sha256      BLOB    NOT NULL CHECK (length(migration_sha256) = 32),
    sqlite_version        TEXT    NOT NULL,
    applied_utc_seconds   INTEGER NOT NULL,
    previous_version      INTEGER NOT NULL,
    CHECK (previous_version = version - 1)
) STRICT;

CREATE TABLE edge_role_registry (
    role                  TEXT PRIMARY KEY,
    CHECK (length(role) BETWEEN 1 AND 96),
    CHECK (role NOT GLOB '*[^a-z0-9_]*')
) STRICT, WITHOUT ROWID;

CREATE TABLE objects (
    object_id             BLOB    PRIMARY KEY,
    kind                  INTEGER CHECK (kind BETWEEN 1 AND 31),
    object_schema         INTEGER CHECK (object_schema >= 0),
    presence              INTEGER NOT NULL CHECK (presence IN (1, 2, 3, 4)),
    canonical_length      INTEGER CHECK (canonical_length >= 0),
    first_seen_revision   INTEGER NOT NULL CHECK (first_seen_revision >= 0),
    unavailable_reason    INTEGER,
    UNIQUE (object_id, kind),
    CHECK ((presence = 1 AND kind IS NOT NULL AND object_schema IS NOT NULL AND
            canonical_length IS NOT NULL) OR
           (presence IN (2, 3) AND canonical_length IS NULL) OR
           (presence = 4 AND kind IS NOT NULL AND object_schema IS NOT NULL AND
            canonical_length IS NULL))
) STRICT, WITHOUT ROWID;

-- Packs are physical immutable containers, not logical ObjectKind values.
CREATE TABLE physical_packs (
    pack_id               BLOB    PRIMARY KEY CHECK (length(pack_id) = 16),
    pack_format           INTEGER NOT NULL CHECK (pack_format >= 0),
    relative_path         TEXT    NOT NULL UNIQUE,
    checksum_code         INTEGER NOT NULL CHECK (checksum_code = 18),
    pack_checksum         BLOB    NOT NULL CHECK (length(pack_checksum) = 32),
    stored_length         INTEGER NOT NULL CHECK (stored_length > 0),
    state                 INTEGER NOT NULL CHECK (state IN (1, 2, 3)),
    created_revision      INTEGER NOT NULL CHECK (created_revision >= 0),
    retired_revision      INTEGER CHECK (retired_revision >= created_revision),
    UNIQUE (pack_id, state),
    CHECK ((state = 1 AND retired_revision IS NULL) OR
           (state IN (2, 3) AND retired_revision IS NOT NULL))
) STRICT, WITHOUT ROWID;

CREATE TABLE object_locations (
    location_id           INTEGER PRIMARY KEY,
    object_id             BLOB    NOT NULL,
    location_kind         INTEGER NOT NULL CHECK (location_kind IN (1, 2)),
    relative_path         TEXT,
    pack_id               BLOB,
    pack_state            INTEGER CHECK (pack_state = 1),
    pack_offset           INTEGER,
    stored_length         INTEGER NOT NULL CHECK (stored_length > 0),
    active                INTEGER NOT NULL CHECK (active IN (0, 1)),
    FOREIGN KEY (object_id) REFERENCES objects(object_id) ON DELETE RESTRICT,
    FOREIGN KEY (pack_id, pack_state)
        REFERENCES physical_packs(pack_id, state) ON DELETE RESTRICT,
    CHECK ((location_kind = 1 AND relative_path IS NOT NULL AND
            pack_id IS NULL AND pack_state IS NULL AND pack_offset IS NULL) OR
           (location_kind = 2 AND relative_path IS NULL AND
            pack_id IS NOT NULL AND pack_state = 1 AND pack_offset >= 0)),
    UNIQUE (object_id, location_kind, relative_path, pack_id, pack_offset)
) STRICT;

CREATE UNIQUE INDEX one_active_loose_location
    ON object_locations(object_id) WHERE location_kind = 1 AND active = 1;
CREATE INDEX active_locations_by_object
    ON object_locations(object_id, active);

CREATE TABLE object_edges (
    source_id             BLOB    NOT NULL,
    ordinal               INTEGER NOT NULL CHECK (ordinal >= 0),
    role                  TEXT    NOT NULL,
    expected_kind         INTEGER CHECK (expected_kind BETWEEN 1 AND 31),
    target_id             BLOB    NOT NULL,
    PRIMARY KEY (source_id, ordinal),
    FOREIGN KEY (source_id) REFERENCES objects(object_id) ON DELETE CASCADE,
    FOREIGN KEY (role) REFERENCES edge_role_registry(role),
    FOREIGN KEY (target_id) REFERENCES objects(object_id) ON DELETE RESTRICT,
    FOREIGN KEY (target_id, expected_kind)
        REFERENCES objects(object_id, kind) ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;

CREATE INDEX object_edges_target ON object_edges(target_id);
CREATE INDEX object_edges_role_source ON object_edges(role, source_id);

-- Rebuildable projection of SnapshotParent edges. Parent order is canonical.
CREATE TABLE snapshot_parents (
    snapshot_id           BLOB    NOT NULL,
    snapshot_kind         INTEGER NOT NULL DEFAULT 6 CHECK (snapshot_kind = 6),
    parent_position       INTEGER NOT NULL CHECK (parent_position >= 0),
    parent_id             BLOB    NOT NULL,
    parent_kind           INTEGER NOT NULL DEFAULT 6 CHECK (parent_kind = 6),
    PRIMARY KEY (snapshot_id, parent_position),
    UNIQUE (snapshot_id, parent_id),
    FOREIGN KEY (snapshot_id, snapshot_kind) REFERENCES objects(object_id, kind),
    FOREIGN KEY (parent_id, parent_kind) REFERENCES objects(object_id, kind)
) STRICT, WITHOUT ROWID;

CREATE INDEX snapshot_children ON snapshot_parents(parent_id, snapshot_id);

-- Rebuildable DAG generations. kind is Snapshot or Operation only.
CREATE TABLE graph_generations (
    object_id             BLOB    PRIMARY KEY,
    kind                  INTEGER NOT NULL CHECK (kind IN (6, 10)),
    generation            INTEGER NOT NULL CHECK (generation >= 0),
    FOREIGN KEY (object_id, kind) REFERENCES objects(object_id, kind)
) STRICT, WITHOUT ROWID;

-- Rebuildable Operation index; the immutable object remains authoritative.
CREATE TABLE operations (
    operation_id          BLOB    PRIMARY KEY,
    operation_kind        INTEGER NOT NULL DEFAULT 10 CHECK (operation_kind = 10),
    generation            INTEGER NOT NULL CHECK (generation >= 0),
    logical_time          INTEGER NOT NULL CHECK (logical_time >= 0),
    FOREIGN KEY (operation_id, operation_kind) REFERENCES objects(object_id, kind)
) STRICT, WITHOUT ROWID;

CREATE TABLE operation_parents (
    operation_id          BLOB    NOT NULL,
    parent_position       INTEGER NOT NULL CHECK (parent_position >= 0),
    parent_id             BLOB    NOT NULL,
    PRIMARY KEY (operation_id, parent_position),
    UNIQUE (operation_id, parent_id),
    FOREIGN KEY (operation_id) REFERENCES operations(operation_id) ON DELETE CASCADE,
    FOREIGN KEY (parent_id) REFERENCES operations(operation_id) ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;

-- The complete ReferenceState tuple is authoritative local mutable state.
CREATE TABLE "references" (
    ref_kind              INTEGER NOT NULL CHECK (ref_kind BETWEEN 1 AND 5),
    stable_id             BLOB    NOT NULL,
    target_id             BLOB    NOT NULL,
    target_kind           INTEGER NOT NULL,
    generation            INTEGER NOT NULL CHECK (generation >= 0),
    operation_id          BLOB    NOT NULL,
    operation_kind        INTEGER NOT NULL DEFAULT 10 CHECK (operation_kind = 10),
    PRIMARY KEY (ref_kind, stable_id),
    FOREIGN KEY (target_id, target_kind) REFERENCES objects(object_id, kind),
    FOREIGN KEY (operation_id, operation_kind) REFERENCES objects(object_id, kind),
    CHECK ((ref_kind = 1 AND length(stable_id) = 16 AND target_kind = 8) OR
           (ref_kind = 2 AND length(stable_id) = 16 AND target_kind = 7) OR
           (ref_kind = 3 AND length(stable_id) = 0  AND target_kind = 10) OR
           (ref_kind = 4 AND length(stable_id) = 16 AND target_kind = 12) OR
           (ref_kind = 5 AND length(stable_id) = 16 AND target_kind = 11))
) STRICT, WITHOUT ROWID;

CREATE INDEX references_by_target ON "references"(target_id);
CREATE INDEX references_by_operation ON "references"(operation_id);

CREATE VIEW operation_heads AS
SELECT target_id AS operation_id, generation, operation_id AS publishing_operation
FROM "references" WHERE ref_kind = 3 AND length(stable_id) = 0;

-- Durable restricted evidence for collision, corruption, and quarantine events.
CREATE TABLE incidents (
    incident_id           BLOB    PRIMARY KEY CHECK (length(incident_id) = 16),
    affected_object_id    BLOB    NOT NULL,
    safe_identity_sha256  BLOB    NOT NULL CHECK (length(safe_identity_sha256) = 32),
    evidence_sha256       BLOB    NOT NULL CHECK (length(evidence_sha256) = 32),
    reason                INTEGER NOT NULL CHECK (reason BETWEEN 1 AND 7),
    evidence_relative_path TEXT   NOT NULL,
    fail_closed           INTEGER NOT NULL CHECK (fail_closed IN (0, 1)),
    state                 INTEGER NOT NULL CHECK (state IN (1, 2)),
    created_revision      INTEGER NOT NULL CHECK (created_revision >= 0),
    created_operation_id  BLOB,
    created_operation_kind INTEGER CHECK (created_operation_kind = 10),
    created_utc_seconds   INTEGER NOT NULL,
    resolved_operation_id BLOB,
    resolved_operation_kind INTEGER CHECK (resolved_operation_kind = 10),
    resolved_utc_seconds  INTEGER,
    resolution_code       INTEGER,
    FOREIGN KEY (affected_object_id) REFERENCES objects(object_id) ON DELETE RESTRICT,
    FOREIGN KEY (created_operation_id, created_operation_kind)
        REFERENCES objects(object_id, kind) ON DELETE RESTRICT,
    FOREIGN KEY (resolved_operation_id, resolved_operation_kind)
        REFERENCES objects(object_id, kind) ON DELETE RESTRICT,
    CHECK ((created_operation_id IS NULL AND created_operation_kind IS NULL) OR
           (created_operation_id IS NOT NULL AND created_operation_kind = 10)),
    CHECK ((state = 1 AND resolved_operation_id IS NULL AND
            resolved_operation_kind IS NULL AND resolved_utc_seconds IS NULL AND
            resolution_code IS NULL) OR
           (state = 2 AND resolved_operation_id IS NOT NULL AND
            resolved_operation_kind = 10 AND resolved_utc_seconds IS NOT NULL AND
            resolution_code IS NOT NULL))
) STRICT, WITHOUT ROWID;

CREATE INDEX incidents_by_object_state
    ON incidents(affected_object_id, state);
CREATE INDEX incidents_open_fail_closed
    ON incidents(fail_closed, state) WHERE state = 1;

CREATE TABLE leases (
    lease_id              BLOB    PRIMARY KEY CHECK (length(lease_id) = 16),
    lease_kind            INTEGER NOT NULL CHECK (lease_kind BETWEEN 1 AND 5),
    owner_process_id      BLOB    NOT NULL CHECK (length(owner_process_id) = 16),
    repository_revision   INTEGER NOT NULL CHECK (repository_revision >= 0),
    expires_utc_seconds   INTEGER NOT NULL,
    boot_identity         BLOB    NOT NULL,
    created_utc_seconds   INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE lease_objects (
    lease_id              BLOB NOT NULL,
    object_id             BLOB NOT NULL,
    PRIMARY KEY (lease_id, object_id),
    FOREIGN KEY (lease_id) REFERENCES leases(lease_id) ON DELETE CASCADE,
    FOREIGN KEY (object_id) REFERENCES objects(object_id) ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;

CREATE INDEX leases_by_expiry ON leases(expires_utc_seconds);
CREATE INDEX leased_objects ON lease_objects(object_id);

CREATE TABLE gc_tombstones (
    object_id             BLOB    PRIMARY KEY,
    retired_revision      INTEGER NOT NULL CHECK (retired_revision >= 0),
    reason                INTEGER NOT NULL,
    retired_utc_seconds   INTEGER NOT NULL,
    FOREIGN KEY (object_id) REFERENCES objects(object_id) ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;
```

The repeated closed kind columns make SQLite enforce typed composite foreign keys;
they are not independent metadata and always carry their checked constant. A
Snapshot, Operation, reference target, or publishing operation row MUST name an
object of exactly the stated kind. Migration tests MUST execute the actual shipped
DDL against the pinned SQLite rather than treating this document as a parser test.

`pack_id` is a random 128-bit local physical-container identity. It is deliberately
not an `ObjectId` and is never accepted by logical graph APIs. `pack_checksum` is the
SHA-256 of the complete immutable physical pack under its pack-format domain; code 18
is the registered SHA-256 multihash code. A pack file is fully written, verified,
flushed, renamed without replacement, and directory-flushed before its active row can
commit. The composite location foreign key permits only state-1 packs. Pack retirement
therefore deletes every associated location row before changing pack state in the same
transaction. The version-1 table does not itself define pack record framing; until a
separate normative physical pack format exists, conforming stores create no pack rows
and use loose locations only.

An incident's `safe_identity_sha256` hashes a domain-separated, redacted description
of the observed file identity and expected lookup identity; it is not an object ID.
`evidence_sha256` authenticates the owner-only restricted evidence report at
`evidence_relative_path`. Reason codes are 1 checksum failure, 2 object-ID mismatch,
3 same-ID collision, 4 missing indexed storage, 5 wrong derived path, 6 non-regular or
linked entry, and 7 invalid physical pack. Autonomous startup discovery may have no
creating Operation; a command-created incident records one. Resolution always names
a present kind-10 Operation and never deletes or rewrites the original evidence.

`relative_path`, including a physical-pack or incident-evidence path, is internally
generated and normalized beneath `.rgit`, never caller input. Loose paths MUST equal
the pure derivation in `loose-record.md`.
Paths may not be absolute, contain empty/`.`/`..` components, backslashes, NUL, or
platform aliases. Physical opens remain descriptor/handle-relative and reject links;
the database path is never itself a sandbox.

## 6. Cross-table invariants

SQLite constraints are defense in depth. The writer MUST validate all of these in
application code inside the publication boundary:

1. A present object has at least one active, fully verified physical location. A
   promised, quarantined, or tombstoned object has no active lookup location.
2. A quarantined ID wins over any formerly present or promised state. Ordinary `get`
   returns quarantined and never falls through to bytes. Quarantine records are not
   stored in `object_locations`.
3. `object_edges` exactly equals `AnyObject::references()` in canonical traversal
   order. `ordinal` is that zero-based order. Every non-null `expected_kind` equals
   the referenced object's verified kind. A null expected kind means only that the
   logical edge permits multiple kinds; it does not skip target verification.
4. Snapshot and Operation parent tables exactly project their corresponding edge
   roles and canonical parent order. A root has generation 0; every other node has
   `1 + max(parent generation)`. Cycles, missing parents, overflow, or disagreement
   are `InvalidGraph`, not partial index results.
5. A reference key's stable ID equals the stable identity inside its target object,
   where that target kind carries one. This is the current store contract's
   `reference_identity_matches` rule, not merely a kind check.
6. Every reference target and publishing operation is present, verified, not
   quarantined, and has a complete valid closure. A promised target or any promised
   member in its required closure prevents publication.
7. Every reference update corresponds exactly once to an action in the immutable
   publishing Operation. Operation-head updates point to that same Operation and its
   declared parents agree with the prior head. LineState predecessor, operation, and
   action fields agree with the update. No unused Operation action remains.
8. A newly created reference has generation 0. An existing reference advances by
   exactly one. Generation never decreases, skips, wraps, or derives from graph
   generation.
9. `repository.revision` advances exactly once for each committed state mutation
   that changes inventory, promises, quarantine, references, migrations, leases, or
   GC state. No-op deduplication need not advance it. Overflow freezes mutation.
10. No normal reference, closure, or recovery path can adopt a tombstoned object.
    Explicit reintroduction first publishes and verifies new durable storage, then
    atomically clears the tombstone and registers the object as a new mutation.
11. `repository.root_object_id` is null only during atomic bootstrap. A usable
    repository names a present, verified kind-14 RepositoryRoot with complete closure.
12. Every pack location references an active row in `physical_packs`. There is no
    logical `Pack` object kind in the version-0 object registry: a physical pack ID,
    checksum, or path MUST NOT be decoded, disclosed, or traversed as an `ObjectId`.
    A pack cannot enter retiring or quarantined state while any location row
    references its `(pack_id, active)` composite key.
13. `repository.incident_read_only` is 1 exactly when at least one open incident with
    `fail_closed=1` exists. An affected quarantined object and its incident evidence
    remain durable across restart and repair. Resolution requires a present verified
    Operation, resolution code, and time; only the transaction resolving the last
    fail-closed incident may clear repository incident mode.
14. A `gc_tombstones` row exists exactly when the matching object row has presence 4.
    Tombstoned objects have no locations or derived edges/index rows and are never
    returned, promised, traversed, or recovered from physical orphan bytes.

The writer MUST run `PRAGMA foreign_key_check` and the semantic verifier in section
13 after schema creation/migration and before enabling mutation.

## 7. Locks and transaction discipline

RGit uses three nested controls in this order:

1. a platform writer-process lock at `.rgit/locks/repository.write` acquired without
   following links or reparse points;
2. the in-process bounded writer coordinator;
3. SQLite `BEGIN IMMEDIATE` and its WAL locks.

Only one process may hold the writer-process lock. Read-only processes may coexist.
Migration, restore, destructive repair, and quiescent backup additionally acquire an
exclusive maintenance lock that prevents readers and writers. Locks carry diagnostic
process tokens but correctness comes from OS lock ownership, never stale PID text.
The lock acquisition timeout is bounded and cancellation-safe.

All mutating transactions use `BEGIN IMMEDIATE`, perform checked reads and writes,
and explicitly `COMMIT` or roll back on error. They do not perform network calls,
user prompts, canonical encoding, signature verification, bulk file writes, or policy
evaluation while holding SQLite locks. A busy/locked result after the bounded timeout
is a typed retryable conflict, not permission to bypass the coordinator. Callbacks
such as `PublicationValidator` run against an immutable staged candidate before the
database transaction and MUST be revalidated against the same revision after
`BEGIN IMMEDIATE`.

Readers use explicit short-lived read transactions when several queries must observe
one revision. Long-lived readers pin required object IDs with a renewable reader
lease and release the SQLite snapshot promptly.

## 8. Object admission and publication

### 8.1 Single-object `put`

The durable implementation of `Store::put(id, bytes)` MUST:

1. strictly decode and canonically re-encode the bytes, recompute the complete ID,
   collect typed edges, validate resource limits, and stage promised target rows;
2. execute loose-record publication steps 1 through 7, including temp verification,
   no-replace rename, and directory durability, without a database transaction;
3. acquire the writer controls, `BEGIN IMMEDIATE`, recheck incident mode, current
   presence, final-file identity, and repository revision;
4. insert/update the object inventory, active location, ordered edges and graph
   projections, remove its promised status, and increment revision;
5. commit with `synchronous=FULL`.

Thus durable object bytes always precede database visibility. A crash before step 5
leaves an unindexed orphan which startup may verify and register as unreferenced. A
database row MUST never precede or substitute for durable bytes. If an identical
verified final record and matching row already exist, the outcome is
`AlreadyPresent`. Any same-ID byte disagreement enters the collision/quarantine path;
the existing final file is never overwritten.

An edge target absent from inventory is inserted as `promised` during the same
transaction. This records graph incompleteness without implying remote authorization
or availability.

### 8.2 Atomic publication

For `Store::publish`, all candidate object files are independently made durable first.
The writer then stages decoded objects, updates, full closures, graph indexes, and the
immutable Operation in memory. It rejects duplicate reference keys. After the
validator accepts that exact candidate, it acquires controls and executes one
`BEGIN IMMEDIATE` transaction:

1. require the repository revision used for staging;
2. recheck every object's durable identity and insert inventory/index rows;
3. require a present Operation of kind 10 and validate action/parent agreement;
4. evaluate every reference CAS against the pre-transaction reference set;
5. validate target kind, stable identity, predecessor, complete closure, and graph;
6. insert all reference states and the operation/index rows together;
7. advance repository revision once and commit `FULL`.

No subset is observable. On validation, CAS, I/O, disk-full, or commit failure, no
reference update commits; already durable object files remain safe orphans. Retrying
the complete publication with the same expected states is safe but, after an unknown
commit outcome, the caller MUST read the reference states and operation ID before
deciding whether to retry.

## 9. Compare-and-swap semantics

CAS compares a complete `(target_id, generation, operation_id)` state, not target ID
alone. `ExpectedRef::Absent` matches only no row. `ExpectedRef::Exact` is encoded as a
single conditional update; application-side prechecks do not replace it.

New reference example (generation 0):

```sql
INSERT INTO "references"
    (ref_kind, stable_id, target_id, target_kind, generation, operation_id)
SELECT :kind, :stable_id, :target, :target_kind, 0, :operation
WHERE NOT EXISTS (
    SELECT 1 FROM "references"
    WHERE ref_kind = :kind AND stable_id = :stable_id
);
-- sqlite3_changes() MUST equal 1
```

Exact-state example:

```sql
UPDATE "references"
SET target_id = :new_target,
    target_kind = :new_kind,
    generation = generation + 1,
    operation_id = :new_operation
WHERE ref_kind = :kind
  AND stable_id = :stable_id
  AND target_id = :expected_target
  AND generation = :expected_generation
  AND operation_id = :expected_operation
  AND generation < 9223372036854775807;
-- sqlite3_changes() MUST equal 1; zero is conflict or overflow, diagnosed by reread
```

All updates in a multi-reference Publication are prechecked and then conditionally
written in one transaction. If any affected-row count is not one, the whole
transaction rolls back with `ReferenceConflict` (or `GenerationOverflow` after a
safe reread). SQL ordering MUST NOT make one update's new state the expected state of
another update to the same key; duplicate keys are rejected before SQL.

## 10. Promises, quarantine, leases, and garbage collection

### 10.1 Promised objects

`mark_promised` inserts state only when an object is neither present nor already
promised. Marking a present object promised is a no-op. Materializing and durably
registering verified bytes changes promised to present atomically. Promises may be
edge targets but never satisfy `get`, closure, reference publication, reachability,
generation, release, or GC-root completeness. A promise carries no proof that the
current principal is allowed to discover or fetch the object.

### 10.2 Quarantine

Quarantine is a fail-closed disposition, not another object location. Quarantining
an ID changes presence to quarantined, removes lookup locations, and records durable
restricted evidence without ordinary logs containing IDs or paths. If no object row
exists, the same transaction inserts a presence-3 row with the affected ID; kind and
schema remain null unless independently verified. Physical evidence
is first published and flushed under the random quarantine naming rules in
`loose-record.md`; only then does one FULL transaction persist the incident, presence,
and repository fail-closed state. A crash before that transaction leaves evidence
which startup inventories into an open incident, never an object location.

Quarantine evidence is never indexed as content. A quarantined ID returns
`Quarantined` even if a stale valid physical file remains. Any collision or incident
affecting a published reference, required closure, pack integrity, or store identity
has `fail_closed=1`. Resolving an incident is append-only in meaning: an authorized
repair Operation sets its resolution fields and state 2 but retains the row and
evidence according to audit retention. The same transaction may restore verified
content or leave it quarantined. It clears `incident_read_only` only after proving no
other open fail-closed incident exists. Reopening an incident creates a new incident
row linked in restricted evidence; it does not erase the resolution history.

### 10.3 Leases and GC

A lease has a random ID, process incarnation token, repository revision, expiry, boot
identity, kind, and exact pinned object set. Expiry uses a conservative wall-clock
deadline plus boot identity: after reboot, clock rollback, or uncertain liveness,
the implementation retains pins for the recovery grace period rather than expiring
early. Renewal is a short `BEGIN IMMEDIATE` mutation. Transfer and backup leases MUST
be established before exposing or enumerating the object set they protect.

GC obtains the writer coordinator and takes a consistent reference/lease snapshot.
Roots include every mutable reference, repository root, promised retention rule,
active lease object, in-flight publication, quarantine retention rule, backup pin,
incident-retained affected object and audit Operation, and repository-defined
retention checkpoint. It verifies complete typed closure;
missing/promised/corrupt closure aborts collection. Derived graph tables are never the
only source of liveness.

Sweep uses explicit presence-4 tombstones; it does not delete object rows. Before the
transaction, the collector proves that no selected object is a repository root,
reference target, publishing Operation, external-edge target, active lease target,
incident-retained affected object or audit Operation, or other retention root. For
one closed selected set,
one FULL `BEGIN IMMEDIATE` transaction performs this exact logical order:

1. delete `snapshot_parents` rows whose snapshot or parent is selected;
2. delete `operation_parents` rows whose operation or parent is selected, then the
   selected `operations` and `graph_generations` cache rows;
3. delete selected-source `object_edges`, after proving no unselected source points
   into the set;
4. delete all selected `object_locations` rows;
5. insert one `gc_tombstones` row per selected object;
6. update each selected `objects` row to presence 4, clear `canonical_length`, set a
   GC reason, advance repository revision once, and commit.

Every step rolls back together and `foreign_key_check` must remain empty. Promises,
quarantined objects, incident evidence, and rows referenced by a lease are handled by
their retention rules rather than this sweep. The collector then deletes loose files
or retire-compacts packs and durably flushes directories. A pack can change from
active to retiring only after all of its location rows are gone; its physical file is
removed after leases and the recovery grace period. A crash leaves unreachable bytes
whose presence-4 object row and tombstone both prevent resurrection. Failure to delete
is retryable and does not restore visibility.

Explicit reintroduction first publishes and verifies new durable bytes. Its FULL
transaction removes the matching tombstone, restores the object row to presence 1,
registers locations/edges/caches, and advances revision. No scan, repair, or promise
operation may clear presence 4 implicitly. Tombstones are otherwise retained.

## 11. Migrations

Migrations are embedded, ordered, immutable Rust resources named
`0001_<name>.sql`, `0002_<name>.sql`, and so on, each with a build-time SHA-256.
The binary supports a contiguous interval of source versions and exactly one target
version. Migration is strictly monotonic; downgrade-in-place is forbidden.

Before migration the command:

1. verifies filesystem capabilities, database header, integrity, foreign keys,
   migration hashes, and current semantic invariants;
2. obtains the exclusive maintenance lock and confirms no live leases/processes;
3. creates and verifies the coherent backup described in section 12;
4. reopens the original with writer settings and checks free-space headroom;
5. applies each next migration in its own `BEGIN IMMEDIATE` transaction, rebuilding
   derived tables from verified records rather than trusting old cached values;
6. inserts the matching `schema_migrations` row and sets `user_version` as the final
   statements in that same transaction;
7. commits FULL, checkpoints, reopens, and runs the complete verifier before
   proceeding to the next version.

The migration runner requires `previous_version == current`, `version == current+1`,
an exact embedded hash, and no preexisting conflicting row. It never skips a version,
rewrites an applied row, resumes in the middle of a SQL file, or interprets repository
SQL. Transactional DDL makes interruption yield either the old or new version. A
nontransactional physical rewrite uses new immutable files, verified no-replace
publication, a later pointer transaction, leases, and deferred retirement.

Failed post-migration verification leaves the repository read-only and preserves the
backup. Rollback means restoring the complete backup while quiescent, not issuing
reverse SQL against a partially migrated database.

## 12. Backup, restore, and copying

Copying `repository.sqlite3` alone while it is live is never a backup. Copying the
main file and racing `-wal`/`-shm` files is also forbidden.

### 12.1 Online backup

An online backup coordinator acquires a backup lease and briefly enters the bounded
writer coordinator. In one consistent read snapshot it records repository identity,
revision, schema/application versions, reference states, GC tombstones, and the exact
verified object/location closure, physical-pack inventory, and incident ledger. It
uses SQLite's online backup API to copy the
database to a new owner-only file on an approved filesystem, with bounded retries.
The object set remains pinned until all selected immutable records/packs are copied
and independently verified. Restricted incident evidence is copied under the same
retention and access controls and independently checksum-verified. A
signed/checksummed backup manifest records database hash, object IDs, physical pack
and evidence-file hashes, formats, revision, and completion marker.
The manifest and destination directory are flushed before success is reported.

Objects durably published after the captured revision may be copied as harmless
extras but MUST NOT be added to the manifest. References cannot require bytes absent
from the captured closure because objects precede their database publication. GC may
not retire selected bytes until the backup lease ends.

### 12.2 Quiescent copy

A filesystem-level copy requires the exclusive maintenance lock, no connections or
leases, `PRAGMA wal_checkpoint(TRUNCATE)` completion, a FULL sync, and platform flush
of database and metadata directory. Only then may the closed main database and exact
manifested immutable files be copied. `-wal` and `-shm` are runtime artifacts and are
not included in the quiescent copy.

Restore targets a new empty directory, verifies the complete backup manifest and
every immutable record before installation, pins filesystem identity, then atomically
publishes the repository directory or requires the destination to remain offline.
It runs header, SQLite integrity, foreign-key, semantic, reference-closure, and object
verification before allowing writable open. Restore never merges database files or
silently replaces a repository with a different `repository_id`.

## 13. Startup recovery, verification, and repair

Writable startup holds the writer lock and performs, in order:

1. safe control-directory traversal and filesystem/WAL probes;
2. header/version/migration-ledger checks;
3. `PRAGMA quick_check`, escalating to `integrity_check` on failure, plus
   `foreign_key_check`;
4. schema fingerprint and closed-registry comparison;
5. reconciliation of temp, final loose, physical-pack, quarantine, incident-evidence,
   inventory, and tombstone state according to `loose-record.md` and the registered
   physical formats;
6. verification that present rows have verified active bytes and promised/quarantined
   rows do not;
7. re-decode/re-hash sampling on normal open, or every object for `rg verify --full`;
8. edge, typed-kind, stable-identity, operation/action, reference, closure, DAG,
   generation, physical-pack checksum/location, incident lifecycle/evidence checksum,
   lease, and GC-root verification;
9. rebuilding and byte/row comparison of derived indexes;
10. WAL checkpoint policy and recovery report publication.

Unexpected valid final records become unreferenced orphans only after full
verification and only when neither presence 4 nor a tombstone exists. An unknown pack
is quarantined as a physical container and is never inferred to contain live objects.
An evidence file without a database row creates an open restricted incident; an
incident row with missing or mismatched evidence remains open and fail-closed.
Missing/invalid indexed bytes,
kind/edge disagreement, referenced quarantine, invalid CAS history, or a corrupt
authoritative checkpoint sets incident read-only mode. Recovery never creates a
reference, guesses an Operation, overwrites a final record, or fabricates bytes.

Repair defaults to report-only. Its report separates authoritative damage from
rebuildable-index damage and redacts identifiers unless the caller has audit
capability. `--rebuild-indexes` drops and recreates only declared rebuildable tables
from verified objects inside a new database transaction. Destructive removal,
quarantine release, reference reset, or backup restore requires an explicit audited
command and preserves old evidence. `PRAGMA writable_schema`, ad-hoc SQL, and SQLite
`.recover` output are forensic inputs, never automatic production repair.

## 14. Failure matrix

| Boundary/failure | Required visible state | Recovery/action |
| --- | --- | --- |
| Decode, canonical, ID, or edge validation fails | No file or row becomes visible | Return `InvalidObject`; retain only restricted diagnostics. |
| Temp write/flush/verification fails | No final file or row | Remove aged temp safely; flush temp directory. |
| No-replace finds identical verified final file | Existing bytes remain | Deduplicate; register only after full verification. |
| Same path/ID has different or invalid bytes | Existing final untouched; no reference update | Quarantine candidate, enter incident read-only as required. |
| Evidence file is durable but incident transaction fails | Restricted evidence orphan; repository not falsely repaired | Startup inventories an open incident and evaluates fail-closed scope. |
| Incident row is open or evidence is missing/mismatched | Incident and quarantine disposition persist | Keep/enter incident read-only; only audited repair Operation resolves it. |
| Packed location names an absent/non-active pack | Transaction rejected by composite foreign key | Do not expose location; verify or quarantine physical pack. |
| Active pack checksum or framing fails | No contained location is readable | Quarantine pack, persist incidents for affected objects, enter incident mode if reachable. |
| Crash after durable rename, before DB commit | Valid unindexed orphan | Startup verifies and inventories as unreferenced unless tombstoned. |
| DB insert/commit fails after object publication | Durable orphan; old references | Roll back DB; never delete bytes as transaction rollback. |
| CAS affects zero rows | Old state and generation | Roll back whole publication; return conflict/overflow after reread. |
| Crash during multi-reference commit | All old or all new states | SQLite WAL recovery; validate Operation and closures on reopen. |
| `SQLITE_FULL`, I/O error, or sync error | No success claim; commit outcome may be unknown | Reopen and inspect operation/reference states before retry. |
| Missing promised object during closure | No publication | Return promised/incomplete; fetch only after authorization. |
| Referenced object becomes quarantined/corrupt | References retained for evidence but unusable | Incident read-only; audited repair/restore. |
| WAL/SHM locking or checkpoint probe fails | No writable connection | Diagnosed read-only open; no journal-mode weakening. |
| Writer process dies holding lock | OS releases lock; WAL may remain | Next writer performs SQLite recovery and full startup checks. |
| Migration process dies | Entire current step old or new | Verify ledger/user_version; never skip or reverse a step. |
| Migration verification fails | Repository read-only; backup retained | Restore complete quiescent backup or ship corrective forward migration. |
| Online backup interrupted | No completion marker | Discard/inventory incomplete destination; source and lease recover safely. |
| GC crash after presence-4/tombstone commit | Object invisible; bytes may remain | Retry physical retirement; recovery must not resurrect. |
| GC candidate has an outside edge/FK, lease, reference, or incident retention | Tombstone transaction rejected/aborted | Recompute roots; do not weaken or delete the retaining row. |
| GC sees live/uncertain lease or incomplete closure | Nothing swept | Renew/re-evaluate after grace period. |
| Clock rollback/reboot makes lease expiry uncertain | Pins retained | Apply boot-aware recovery grace; never expire early. |
| Index mismatch with verified object | Authoritative object/reference unchanged | Rebuild index; report if mismatch recurs. |

Every row in this matrix requires deterministic fault injection before the durable
backend may claim production readiness. Process-kill tests must cover every numbered
loose-publication, SQLite-commit, migration, backup, checkpoint, and GC boundary on
each supported OS/filesystem profile.

## 15. Conformance vectors

These logical vectors are mandatory in addition to byte-level object and loose-record
vectors.

### 15.1 Reference generations and exact CAS

Given a verified LineState `L0`, publishing Operation `O0`, and stable line ID `S`, an
absent insert produces `(line,S,L0,generation=0,O0)`. Repeating absent CAS conflicts.
Given verified `L1` and `O1`, exact CAS against all of `L0,0,O0` produces `L1,1,O1`.
CAS using `L0,0,O_wrong`, or `L0,1,O0`, conflicts and leaves `L0,0,O0`. A transaction
containing one valid line update and one conflicting change update commits neither.

### 15.2 Presence precedence

For ID `X`: unknown has no row; promised returns `Promised`; verified durable put
changes it to present; repeat identical put is `AlreadyPresent`; quarantine changes
lookup to `Quarantined`; a stale valid loose file does not make it present again.
Closure through promised `X` is `ClosureError::Promised`; through quarantined `X` is
`ClosureError::Quarantined`.

### 15.3 Graph generations

Snapshots `A()` and `B()` have generation 0. `C(A)` has 1, `D(A,B)` has 1, and
`E(C,D)` has 2 regardless of insertion order. A cycle, duplicate parent, wrong-kind
parent, absent unpromised parent, promised parent during publication, or stored
generation other than those values fails verification.

### 15.4 Crash boundaries

For one Publication containing two objects and two references, kill the process after
every file write, file flush, temp verification, rename, directory flush, `BEGIN
IMMEDIATE`, row group, WAL write, commit, and checkpoint hook. After every restart,
the state is either old references plus zero or more safe orphans, or both new
references plus complete verified closure and matching Operation. No vector permits
one new reference, a row without durable bytes, or resurrection of a tombstone.

### 15.5 Migration and backup

For every supported source version, run every kill point and confirm a contiguous
ledger whose last version equals `user_version`. Restore the pre-migration backup and
compare repository ID, reference tuples, canonical IDs, and closures. For online
backup concurrent with puts, publication, GC, and checkpoint, restore must contain
exactly the manifest revision's references and complete object closure; later objects
may exist only as unreferenced extras.

### 15.6 Physical packs, incidents, and GC retirement

Insert a present object and attempt a kind-2 location whose `pack_id` has no
`physical_packs` row: the foreign key rejects it. Insert an active pack and the same
location: it succeeds. Attempt to mark that pack retiring while the location remains:
the composite `(pack_id,state)` foreign key rejects it. Delete every pack location in
the retirement transaction and then mark it retiring: it succeeds and
`foreign_key_check` is empty.

For affected ID `Q`, persist quarantined presence and an open fail-closed incident,
restart, and verify the evidence checksum and repository incident mode remain. An
unrelated or absent resolution Operation cannot resolve it. A present kind-10 repair
Operation may resolve it; repository mode clears only when no other open fail-closed
row remains. Corrupt or remove the evidence file and verify restart remains
fail-closed rather than silently resolving or deleting the incident.

For unreachable object `G`, execute every GC transaction boundary. Before commit,
either `G` remains present with its locations/derived rows and no tombstone, or the
transaction rolls back. After commit, `G` has presence 4, one tombstone, and no
location, edge, parent, operation, or generation row; `foreign_key_check` is empty.
Leaving a reference, outside edge, lease, or retained incident Operation causes
selection to abort. A valid orphan file for `G` does not change its presence after
restart. Verified explicit reintroduction removes the tombstone and restores presence
and indexes in one transaction.

## 16. Implementation review gates

Conformance requires all of the following evidence; prose or passing happy-path unit
tests alone are insufficient:

- actual shipped DDL parsed on the pinned SQLite and schema-fingerprinted;
- property tests comparing durable behavior with `MemoryStore` for every `Store`
  method and error precedence;
- object registry and `ReferenceRole` registry exhaustiveness tests;
- adversarial malformed database, path, record, ID, kind, edge, and generation tests;
- concurrent process CAS, busy timeout, lease, backup, checkpoint, and GC tests;
- physical-pack foreign-key/state/checksum and durable incident lifecycle tests;
- deterministic disk-full, short-write, sync failure, corruption, clock anomaly, and
  process-kill recovery tests;
- migration tests from every supported version and verified backup/restore drills;
- Linux, macOS, and Windows filesystem qualification required by ADR 0009;
- independent storage/security review of authorization-before-probe behavior and
  restricted diagnostics.

Until those gates pass, this document describes the required design only.
