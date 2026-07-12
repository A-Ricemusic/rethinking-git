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

-- Closed version-1 ReferenceRole registry. This is part of migration 1.
INSERT INTO edge_role_registry(role) VALUES ('object_policy');
INSERT INTO edge_role_registry(role) VALUES ('manifest_entry_policy');
INSERT INTO edge_role_registry(role) VALUES ('review_policy');
INSERT INTO edge_role_registry(role) VALUES ('landing_policy');
INSERT INTO edge_role_registry(role) VALUES ('integration_policy');
INSERT INTO edge_role_registry(role) VALUES ('approval_policy');
INSERT INTO edge_role_registry(role) VALUES ('release_policy');
INSERT INTO edge_role_registry(role) VALUES ('visibility_policy');
INSERT INTO edge_role_registry(role) VALUES ('audience_policy');
INSERT INTO edge_role_registry(role) VALUES ('blob_chunk');
INSERT INTO edge_role_registry(role) VALUES ('manifest_entry_target');
INSERT INTO edge_role_registry(role) VALUES ('snapshot_root_manifest');
INSERT INTO edge_role_registry(role) VALUES ('snapshot_parent');
INSERT INTO edge_role_registry(role) VALUES ('snapshot_message');
INSERT INTO edge_role_registry(role) VALUES ('change_previous_revision');
INSERT INTO edge_role_registry(role) VALUES ('change_title');
INSERT INTO edge_role_registry(role) VALUES ('change_description');
INSERT INTO edge_role_registry(role) VALUES ('change_base_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('change_current_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('line_head_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('line_previous_state');
INSERT INTO edge_role_registry(role) VALUES ('line_transaction_operation');
INSERT INTO edge_role_registry(role) VALUES ('subproject_native_projection');
INSERT INTO edge_role_registry(role) VALUES ('secret_development_value');
INSERT INTO edge_role_registry(role) VALUES ('conflict_base');
INSERT INTO edge_role_registry(role) VALUES ('conflict_left');
INSERT INTO edge_role_registry(role) VALUES ('conflict_right');
INSERT INTO edge_role_registry(role) VALUES ('operation_parent');
INSERT INTO edge_role_registry(role) VALUES ('operation_before');
INSERT INTO edge_role_registry(role) VALUES ('operation_after');
INSERT INTO edge_role_registry(role) VALUES ('operation_line_policy');
INSERT INTO edge_role_registry(role) VALUES ('operation_line_head_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('operation_line_previous_state');
INSERT INTO edge_role_registry(role) VALUES ('operation_line_integration_policy');
INSERT INTO edge_role_registry(role) VALUES ('operation_line_approval_policy');
INSERT INTO edge_role_registry(role) VALUES ('operation_line_release_policy');
INSERT INTO edge_role_registry(role) VALUES ('operation_line_visibility_policy');
INSERT INTO edge_role_registry(role) VALUES ('operation_inverse_payload');
INSERT INTO edge_role_registry(role) VALUES ('operation_public_envelope');
INSERT INTO edge_role_registry(role) VALUES ('operation_private_payload');
INSERT INTO edge_role_registry(role) VALUES ('marker_target');
INSERT INTO edge_role_registry(role) VALUES ('release_source_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('release_projection_rules');
INSERT INTO edge_role_registry(role) VALUES ('release_projection_proof');
INSERT INTO edge_role_registry(role) VALUES ('release_projected_root');
INSERT INTO edge_role_registry(role) VALUES ('release_notes');
INSERT INTO edge_role_registry(role) VALUES ('release_build_provenance');
INSERT INTO edge_role_registry(role) VALUES ('release_artifact');
INSERT INTO edge_role_registry(role) VALUES ('release_policy_evidence');
INSERT INTO edge_role_registry(role) VALUES ('policy_previous_version');
INSERT INTO edge_role_registry(role) VALUES ('policy_declassification_requirement');
INSERT INTO edge_role_registry(role) VALUES ('policy_key_envelope_set');
INSERT INTO edge_role_registry(role) VALUES ('repository_root_policy');
INSERT INTO edge_role_registry(role) VALUES ('repository_trusted_identity');
INSERT INTO edge_role_registry(role) VALUES ('repository_bootstrap_key_envelope_set');
INSERT INTO edge_role_registry(role) VALUES ('repository_genesis_operation');
INSERT INTO edge_role_registry(role) VALUES ('repository_initial_line_state');
INSERT INTO edge_role_registry(role) VALUES ('identity_previous');
INSERT INTO edge_role_registry(role) VALUES ('identity_key_not_before');
INSERT INTO edge_role_registry(role) VALUES ('identity_key_not_after');
INSERT INTO edge_role_registry(role) VALUES ('identity_activation_operation');
INSERT INTO edge_role_registry(role) VALUES ('identity_not_after_operation');
INSERT INTO edge_role_registry(role) VALUES ('membership_previous');
INSERT INTO edge_role_registry(role) VALUES ('membership_activation_operation');
INSERT INTO edge_role_registry(role) VALUES ('membership_not_after_operation');
INSERT INTO edge_role_registry(role) VALUES ('change_relation_source');
INSERT INTO edge_role_registry(role) VALUES ('change_relation_result');
INSERT INTO edge_role_registry(role) VALUES ('change_relation_provenance');
INSERT INTO edge_role_registry(role) VALUES ('change_relation_operation');
INSERT INTO edge_role_registry(role) VALUES ('resolution_conflict');
INSERT INTO edge_role_registry(role) VALUES ('resolution_result');
INSERT INTO edge_role_registry(role) VALUES ('resolution_provenance');
INSERT INTO edge_role_registry(role) VALUES ('evidence_target');
INSERT INTO edge_role_registry(role) VALUES ('evidence_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('evidence_ruleset');
INSERT INTO edge_role_registry(role) VALUES ('evidence_related');
INSERT INTO edge_role_registry(role) VALUES ('ci_runner_identity');
INSERT INTO edge_role_registry(role) VALUES ('ci_build_provenance');
INSERT INTO edge_role_registry(role) VALUES ('projection_rules_previous');
INSERT INTO edge_role_registry(role) VALUES ('projection_source_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('projection_rules');
INSERT INTO edge_role_registry(role) VALUES ('projection_audience_policy');
INSERT INTO edge_role_registry(role) VALUES ('projection_manifest');
INSERT INTO edge_role_registry(role) VALUES ('build_snapshot');
INSERT INTO edge_role_registry(role) VALUES ('build_ruleset');
INSERT INTO edge_role_registry(role) VALUES ('build_identity');
INSERT INTO edge_role_registry(role) VALUES ('build_input');
INSERT INTO edge_role_registry(role) VALUES ('build_output');
INSERT INTO edge_role_registry(role) VALUES ('artifact_blob');
INSERT INTO edge_role_registry(role) VALUES ('payload_reference');
INSERT INTO edge_role_registry(role) VALUES ('view_policy');
INSERT INTO edge_role_registry(role) VALUES ('view_line_state');
INSERT INTO edge_role_registry(role) VALUES ('view_manifest');
INSERT INTO edge_role_registry(role) VALUES ('migration_old_object');
INSERT INTO edge_role_registry(role) VALUES ('migration_new_object');
INSERT INTO edge_role_registry(role) VALUES ('migration_tool_identity');
INSERT INTO edge_role_registry(role) VALUES ('ruleset_previous');
