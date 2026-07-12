#![cfg(unix)]

use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use rgit_objects::{
    ActorId, CanonicalObject, DerivationRule, DeviceId, HashAlgorithm, KeyEnvelopeSet,
    KeyEnvelopeSuite, ObjectId, Operation, Policy, PolicyId, PolicyRef, Principal, PrincipalKind,
    RecipientEnvelope, RedactionMode, Signature, SignatureAlgorithm, SignaturePurpose, Value,
    WallTime,
};
use rgit_store::{
    ExpectedRef, ObjectPresence, Publication, PublicationCandidate, PublicationObject,
    PublicationValidator, RefUpdate, ReferenceKey, SqliteFailurePoint, SqliteStore,
    SqliteStoreOptions, Store, StoreError, TransactionFailureInjector,
};
use rusqlite::Connection;

fn allow(_: &PublicationCandidate<'_>) -> Result<(), StoreError> {
    Ok(())
}

fn encoded<T: CanonicalObject>(value: &T) -> (ObjectId, Vec<u8>) {
    (
        value.id(HashAlgorithm::Sha256).expect("id"),
        value.encode().expect("encoding"),
    )
}

fn fake_id(seed: u8) -> ObjectId {
    let mut bytes = vec![0, HashAlgorithm::Sha256 as u8, 32];
    bytes.extend([seed; 32]);
    ObjectId::from_bytes(&bytes).expect("test object ID")
}

fn signature(seed: u8, purpose: SignaturePurpose) -> Signature {
    Signature::new(
        SignatureAlgorithm::Ed25519,
        ActorId::from_bytes([seed; 16]),
        [seed; 32],
        [seed; 64],
        purpose,
    )
    .expect("signature")
}

fn envelope(seed: u8) -> KeyEnvelopeSet {
    KeyEnvelopeSet {
        policy_ref: None,
        epoch: 0,
        suite: KeyEnvelopeSuite::X25519HkdfSha256Aes256Gcm,
        recipients: vec![RecipientEnvelope {
            recipient: Principal {
                kind: PrincipalKind::Actor,
                identifier: vec![seed],
            },
            key_id: [seed; 32],
            envelope: vec![seed],
        }],
    }
}

fn put<T: CanonicalObject>(store: &SqliteStore, value: &T) -> ObjectId {
    let (id, bytes) = encoded(value);
    store.put(id.clone(), bytes).expect("put");
    id
}

fn operation_fixture(store: &SqliteStore, seed: u8) -> (ObjectId, Vec<u8>) {
    let envelopes = put(store, &envelope(seed));
    let policy_id = PolicyId::from_bytes([seed; 16]);
    let policy = Policy {
        policy_ref: None,
        policy_id,
        version_sequence: 0,
        previous_version: None,
        principals: vec![Principal {
            kind: PrincipalKind::Actor,
            identifier: vec![seed],
        }],
        grants: Vec::new(),
        redaction_mode: RedactionMode::Omit,
        derivation_rule: DerivationRule::NoDerivation,
        declassification_requirements: Vec::new(),
        key_epoch: 0,
        key_envelope_set: envelopes,
        administrators: vec![ActorId::from_bytes([seed; 16])],
        activation_constraints: Value::Map(Vec::new()).encode().expect("constraints"),
        signatures: vec![signature(seed, SignaturePurpose::Policy)],
    };
    let policy_version = put(store, &policy);
    encoded(&Operation {
        policy_ref: PolicyRef {
            policy_id,
            version: policy_version,
        },
        parents: Vec::new(),
        actor: ActorId::from_bytes([seed; 16]),
        device: DeviceId::from_bytes([seed.wrapping_add(1); 16]),
        logical_time: u64::from(seed),
        wall_time: WallTime {
            utc_seconds: i64::from(seed),
            offset_seconds: 0,
        },
        actions: Vec::new(),
        inverse_payloads: Vec::new(),
        public_envelope: None,
        private_payload: None,
        signature: signature(seed, SignaturePurpose::Operation),
        client_implementation: "rgit-sqlite-contract".into(),
    })
}

fn repository(name: &str) -> PathBuf {
    let root = std::env::var_os("RGIT_TEST_APP_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = root.join(format!("rgit-sqlite-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("test repository");
    path
}

#[test]
fn metadata_inventory_promises_and_references_survive_reopen() {
    let path = repository("reopen");
    let control = path.join(".rgit");
    let store = SqliteStore::open(&control).expect("open");
    let promised = fake_id(99);
    store.mark_promised(promised.clone()).expect("promise");
    let (operation, bytes) = operation_fixture(&store, 40);
    let result = store
        .publish(
            Publication {
                objects: vec![PublicationObject {
                    id: operation.clone(),
                    bytes,
                }],
                updates: vec![RefUpdate {
                    key: ReferenceKey::OperationHead,
                    expected: ExpectedRef::Absent,
                    target: operation.clone(),
                }],
                operation: operation.clone(),
            },
            &allow,
        )
        .expect("publish");
    assert_eq!(result[0].generation, 0);
    store.verify_metadata().expect("verify");
    drop(store);

    let reopened = SqliteStore::open(&control).expect("reopen");
    assert_eq!(reopened.presence(&promised), Some(ObjectPresence::Promised));
    assert_eq!(
        reopened
            .reference(&ReferenceKey::OperationHead)
            .expect("head")
            .target,
        operation,
    );
    reopened.verify_metadata().expect("reopen verify");
    fs::remove_dir_all(path).expect("cleanup");
}

struct FailOnce {
    point: SqliteFailurePoint,
    armed: AtomicBool,
}

struct KillAt(SqliteFailurePoint);
impl TransactionFailureInjector for KillAt {
    fn check(&self, point: SqliteFailurePoint) -> Result<(), StoreError> {
        if point == self.0 {
            std::process::exit(88);
        }
        Ok(())
    }
}

#[test]
#[ignore = "subprocess crash helper"]
fn sqlite_process_kill_child() {
    let Some(control) = std::env::var_os("RGIT_SQLITE_CRASH_CONTROL") else {
        return;
    };
    let point = match std::env::var("RGIT_SQLITE_CRASH_POINT").as_deref() {
        Ok("begin") => SqliteFailurePoint::AfterBegin,
        Ok("inventory") => SqliteFailurePoint::AfterInventory,
        Ok("before-commit") => SqliteFailurePoint::BeforeCommit,
        Ok("after-commit") => SqliteFailurePoint::AfterCommit,
        _ => panic!("unknown crash point"),
    };
    let store = SqliteStore::open_with_options(
        PathBuf::from(control),
        SqliteStoreOptions {
            busy_timeout_millis: 100,
            failure_injector: Arc::new(KillAt(point)),
        },
    )
    .expect("child open");
    let (id, bytes) = encoded(&envelope(114));
    let _ = store.put(id, bytes);
    panic!("failure point was not reached");
}

#[test]
fn process_kill_at_sqlite_boundaries_recovers_old_or_committed_state() {
    for (name, committed) in [
        ("begin", false),
        ("inventory", false),
        ("before-commit", false),
        ("after-commit", true),
    ] {
        let path = repository(&format!("kill-{name}"));
        let control = path.join(".rgit");
        let status = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "sqlite_process_kill_child",
                "--ignored",
                "--nocapture",
            ])
            .env("RGIT_SQLITE_CRASH_CONTROL", &control)
            .env("RGIT_SQLITE_CRASH_POINT", name)
            .status()
            .expect("crash child");
        assert_eq!(status.code(), Some(88));
        let reopened = SqliteStore::open(&control).expect("recover after process kill");
        let (id, _) = encoded(&envelope(114));
        assert_eq!(
            reopened.presence(&id),
            committed.then_some(ObjectPresence::Present)
        );
        reopened.verify_metadata().expect("verify recovered store");
        drop(reopened);
        fs::remove_dir_all(path).expect("cleanup");
    }
}

impl TransactionFailureInjector for FailOnce {
    fn check(&self, point: SqliteFailurePoint) -> Result<(), StoreError> {
        if point == self.point && self.armed.swap(false, Ordering::SeqCst) {
            Err(StoreError::InjectedTransactionFailure)
        } else {
            Ok(())
        }
    }
}

#[test]
fn injected_failure_rolls_back_inventory_and_revision() {
    let path = repository("rollback");
    let control = path.join(".rgit");
    let injector = Arc::new(FailOnce {
        point: SqliteFailurePoint::AfterInventory,
        armed: AtomicBool::new(true),
    });
    let store = SqliteStore::open_with_options(
        &control,
        SqliteStoreOptions {
            busy_timeout_millis: 100,
            failure_injector: injector,
        },
    )
    .expect("open");
    let value = envelope(55);
    let (id, bytes) = encoded(&value);
    assert_eq!(
        store.put(id.clone(), bytes),
        Err(StoreError::InjectedTransactionFailure)
    );
    assert_eq!(store.presence(&id), None);
    drop(store);

    let reopened = SqliteStore::open(&control).expect("reopen");
    assert_eq!(reopened.presence(&id), None);
    reopened.verify_metadata().expect("verify rollback");
    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn every_precommit_put_failure_rolls_back_and_after_commit_is_recoverable() {
    for (index, point) in [
        SqliteFailurePoint::AfterBegin,
        SqliteFailurePoint::AfterInventory,
        SqliteFailurePoint::BeforeCommit,
        SqliteFailurePoint::AfterCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let path = repository(&format!("put-phase-{index}"));
        let control = path.join(".rgit");
        let store = SqliteStore::open_with_options(
            &control,
            SqliteStoreOptions {
                busy_timeout_millis: 100,
                failure_injector: Arc::new(FailOnce {
                    point,
                    armed: AtomicBool::new(true),
                }),
            },
        )
        .expect("open");
        let (id, bytes) = encoded(&envelope(index as u8 + 70));
        assert_eq!(
            store.put(id.clone(), bytes),
            Err(StoreError::InjectedTransactionFailure)
        );
        drop(store);
        let reopened = SqliteStore::open(&control).expect("reopen");
        if point == SqliteFailurePoint::AfterCommit {
            assert_eq!(reopened.presence(&id), Some(ObjectPresence::Present));
        } else {
            assert_eq!(reopened.presence(&id), None);
        }
        fs::remove_dir_all(path).expect("cleanup");
    }
}

#[test]
fn publication_failure_after_reference_writes_is_fully_atomic() {
    let path = repository("publication-rollback");
    let control = path.join(".rgit");
    let injector = Arc::new(FailOnce {
        point: SqliteFailurePoint::AfterReferences,
        armed: AtomicBool::new(true),
    });
    let store = SqliteStore::open_with_options(
        &control,
        SqliteStoreOptions {
            busy_timeout_millis: 100,
            failure_injector: injector,
        },
    )
    .expect("open");
    let (operation, bytes) = operation_fixture(&store, 80);
    let result = store.publish(
        Publication {
            objects: vec![PublicationObject {
                id: operation.clone(),
                bytes,
            }],
            updates: vec![RefUpdate {
                key: ReferenceKey::OperationHead,
                expected: ExpectedRef::Absent,
                target: operation.clone(),
            }],
            operation: operation.clone(),
        },
        &allow,
    );
    assert_eq!(result, Err(StoreError::InjectedTransactionFailure));
    assert_eq!(store.reference(&ReferenceKey::OperationHead), None);
    assert_eq!(store.presence(&operation), None);
    drop(store);
    let reopened = SqliteStore::open(&control).expect("reopen");
    assert_eq!(reopened.reference(&ReferenceKey::OperationHead), None);
    assert_eq!(reopened.presence(&operation), None);
    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn reopen_rejects_partial_schema_and_closed_registry_corruption() {
    for (index, corrupt) in [
        "DROP INDEX active_locations_by_object",
        "DELETE FROM edge_role_registry WHERE role='operation_parent'",
    ]
    .into_iter()
    .enumerate()
    {
        let path = repository(&format!("corrupt-{index}"));
        let control = path.join(".rgit");
        let store = SqliteStore::open(&control).expect("open");
        let database = store.database_path();
        drop(store);
        let connection = Connection::open(database).expect("raw open");
        connection.execute_batch(corrupt).expect("corrupt fixture");
        drop(connection);
        assert!(matches!(
            SqliteStore::open(&control),
            Err(StoreError::UnsupportedDatabase)
        ));
        fs::remove_dir_all(path).expect("cleanup");
    }
}

#[test]
fn reopen_rejects_partial_derived_and_edge_indexes() {
    for (index, corrupt) in [
        "DELETE FROM graph_generations",
        "DELETE FROM object_edges WHERE source_id IN (SELECT operation_id FROM operations)",
    ]
    .into_iter()
    .enumerate()
    {
        let path = repository(&format!("partial-index-{index}"));
        let control = path.join(".rgit");
        let store = SqliteStore::open(&control).expect("open");
        let (operation, bytes) = operation_fixture(&store, index as u8 + 90);
        store
            .publish(
                Publication {
                    objects: vec![PublicationObject {
                        id: operation.clone(),
                        bytes,
                    }],
                    updates: vec![RefUpdate {
                        key: ReferenceKey::OperationHead,
                        expected: ExpectedRef::Absent,
                        target: operation.clone(),
                    }],
                    operation,
                },
                &allow,
            )
            .expect("publish");
        let database = store.database_path();
        drop(store);
        let connection = Connection::open(database).expect("raw open");
        connection.execute_batch(corrupt).expect("partial fixture");
        drop(connection);
        assert!(matches!(
            SqliteStore::open(&control),
            Err(StoreError::UnsupportedDatabase)
        ));
        fs::remove_dir_all(path).expect("cleanup");
    }
}

struct Reentrant<'a>(&'a SqliteStore);
impl PublicationValidator for Reentrant<'_> {
    fn validate(&self, candidate: &PublicationCandidate<'_>) -> Result<(), StoreError> {
        let _ = self.0.presence(&candidate.publication().operation);
        Ok(())
    }
}

#[test]
fn validator_can_reenter_without_holding_sqlite_transaction() {
    let path = repository("reentrant");
    let control = path.join(".rgit");
    let store = SqliteStore::open(&control).expect("open");
    let (operation, bytes) = operation_fixture(&store, 60);
    store
        .publish(
            Publication {
                objects: vec![PublicationObject {
                    id: operation.clone(),
                    bytes,
                }],
                updates: vec![RefUpdate {
                    key: ReferenceKey::OperationHead,
                    expected: ExpectedRef::Absent,
                    target: operation.clone(),
                }],
                operation,
            },
            &Reentrant(&store),
        )
        .expect("publish");
    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn quarantine_incident_is_durable_fail_closed_and_evidence_authenticated() {
    let path = repository("incident");
    let control = path.join(".rgit");
    let store = SqliteStore::open(&control).expect("open");
    let id = put(&store, &envelope(110));
    store.quarantine(&id).expect("quarantine");
    assert_eq!(store.presence(&id), Some(ObjectPresence::Quarantined));
    assert_eq!(
        store.mark_promised(fake_id(111)),
        Err(StoreError::IncidentReadOnly)
    );
    drop(store);

    let reopened = SqliteStore::open(&control).expect("reopen incident");
    assert_eq!(reopened.presence(&id), Some(ObjectPresence::Quarantined));
    drop(reopened);
    let evidence = fs::read_dir(control.join("incidents"))
        .expect("incident directory")
        .next()
        .expect("evidence entry")
        .expect("evidence")
        .path();
    fs::write(evidence, b"tampered").expect("tamper evidence");
    assert!(matches!(
        SqliteStore::open(&control),
        Err(StoreError::UnsupportedDatabase)
    ));
    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn startup_adopts_orphan_incident_evidence_fail_closed() {
    let path = repository("orphan-incident");
    let control = path.join(".rgit");
    let injector = Arc::new(FailOnce {
        point: SqliteFailurePoint::AfterBegin,
        armed: AtomicBool::new(false),
    });
    let store = SqliteStore::open_with_options(
        &control,
        SqliteStoreOptions {
            busy_timeout_millis: 100,
            failure_injector: injector.clone(),
        },
    )
    .expect("open");
    let id = put(&store, &envelope(112));
    injector.armed.store(true, Ordering::SeqCst);
    assert_eq!(
        store.quarantine(&id),
        Err(StoreError::InjectedTransactionFailure)
    );
    drop(store);

    let reopened = SqliteStore::open(&control).expect("reconcile orphan evidence");
    assert_eq!(reopened.presence(&id), Some(ObjectPresence::Quarantined));
    assert_eq!(
        reopened.mark_promised(fake_id(113)),
        Err(StoreError::IncidentReadOnly)
    );
    drop(reopened);
    fs::remove_dir_all(path).expect("cleanup");
}
