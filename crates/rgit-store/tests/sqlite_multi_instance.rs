#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
};

use rgit_objects::{
    ActorId, CanonicalObject, HashAlgorithm, KeyEnvelopeSet, KeyEnvelopeSuite, ObjectId, Principal,
    PrincipalKind, RecipientEnvelope,
};
use rgit_store::{ObjectPresence, SqliteStore, Store, StoreError};
use rusqlite::Connection;

const APPLICATION_ID: i64 = 0x5247_4954;

fn repository(name: &str) -> PathBuf {
    let root = std::env::var_os("RGIT_TEST_APP_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = root.join(format!("rgit-sqlite-multi-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("test repository");
    path
}

fn fake_id(seed: u8) -> ObjectId {
    let mut bytes = vec![0, HashAlgorithm::Sha256 as u8, 32];
    bytes.extend([seed; 32]);
    ObjectId::from_bytes(&bytes).expect("test object ID")
}

fn envelope(seed: u8) -> KeyEnvelopeSet {
    KeyEnvelopeSet {
        policy_ref: None,
        epoch: 0,
        suite: KeyEnvelopeSuite::X25519HkdfSha256Aes256Gcm,
        recipients: vec![RecipientEnvelope {
            recipient: Principal {
                kind: PrincipalKind::Actor,
                identifier: ActorId::from_bytes([seed; 16]).as_bytes().to_vec(),
            },
            key_id: [seed; 32],
            envelope: vec![seed; 32],
        }],
    }
}

fn encoded<T: CanonicalObject>(value: &T) -> (ObjectId, Vec<u8>) {
    (
        value.id(HashAlgorithm::Sha256).expect("object ID"),
        value.encode().expect("canonical encoding"),
    )
}

fn create_database(path: &Path, application_id: i64, user_version: i64) {
    fs::create_dir_all(path.parent().expect("database parent")).expect("metadata directory");
    let connection = Connection::open(path).expect("create database");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id={application_id}; PRAGMA user_version={user_version};"
        ))
        .expect("set database header");
}

#[test]
fn instances_observe_promises_and_objects_without_reopen() {
    let path = repository("visibility");
    let control = path.join(".rgit");
    let first = SqliteStore::open(&control).expect("open first store");
    let second = SqliteStore::open(&control).expect("open second store");

    let promised = fake_id(17);
    first
        .mark_promised(promised.clone())
        .expect("first store marks promise");
    assert_eq!(second.presence(&promised), Some(ObjectPresence::Promised));

    let (first_id, first_bytes) = encoded(&envelope(31));
    first
        .put(first_id.clone(), first_bytes)
        .expect("first store puts object");
    assert_eq!(second.presence(&first_id), Some(ObjectPresence::Present));
    assert_eq!(
        second.get(&first_id).expect("second reads object").id(),
        &first_id
    );

    let (second_id, second_bytes) = encoded(&envelope(32));
    second
        .put(second_id.clone(), second_bytes)
        .expect("second store puts object");
    assert_eq!(first.presence(&second_id), Some(ObjectPresence::Present));
    assert_eq!(
        first.get(&second_id).expect("first reads object").id(),
        &second_id
    );

    drop(second);
    drop(first);
    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn concurrent_instances_serialize_process_writers() {
    let path = repository("writers");
    let control = path.join(".rgit");
    let first = Arc::new(SqliteStore::open(&control).expect("open first store"));
    let second = Arc::new(SqliteStore::open(&control).expect("open second store"));
    let barrier = Arc::new(Barrier::new(2));

    let first_thread = {
        let store = Arc::clone(&first);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            for seed in 40..56 {
                store.mark_promised(fake_id(seed)).expect("first writer");
            }
        })
    };
    let second_thread = {
        let store = Arc::clone(&second);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            for seed in 56..72 {
                store.mark_promised(fake_id(seed)).expect("second writer");
            }
        })
    };

    first_thread.join().expect("first writer thread");
    second_thread.join().expect("second writer thread");
    for seed in 40..72 {
        assert_eq!(
            first.presence(&fake_id(seed)),
            Some(ObjectPresence::Promised)
        );
        assert_eq!(
            second.presence(&fake_id(seed)),
            Some(ObjectPresence::Promised)
        );
    }

    drop(second);
    drop(first);
    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn concurrent_writable_startup_is_serialized() {
    let path = repository("startup");
    let control = Arc::new(path.join(".rgit"));
    let barrier = Arc::new(Barrier::new(2));
    let threads = (0..2)
        .map(|_| {
            let control = Arc::clone(&control);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                SqliteStore::open(control.as_ref()).expect("concurrent startup")
            })
        })
        .collect::<Vec<_>>();
    let stores = threads
        .into_iter()
        .map(|thread| thread.join().expect("startup thread"))
        .collect::<Vec<_>>();
    drop(stores);
    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn lock_and_metadata_replacement_fail_closed() {
    for replace_lock in [true, false] {
        let path = repository(if replace_lock {
            "replace-lock"
        } else {
            "replace-metadata"
        });
        let control = path.join(".rgit");
        let store = SqliteStore::open(&control).expect("open");
        if replace_lock {
            let lock = control.join("locks/repository.write");
            fs::rename(&lock, control.join("locks/original.write")).expect("move lock");
            fs::write(&lock, b"replacement").expect("replace lock");
        } else {
            let metadata = control.join("metadata");
            fs::rename(&metadata, control.join("original-metadata")).expect("move metadata");
            fs::create_dir(&metadata).expect("replace metadata directory");
            fs::write(metadata.join("repository.sqlite3"), b"replacement")
                .expect("replace database");
        }
        assert!(matches!(
            store.mark_promised(fake_id(120)),
            Err(StoreError::UnsupportedDatabase | StoreError::ObjectStorage)
        ));
        drop(store);
        fs::remove_dir_all(path).expect("cleanup");
    }
}

#[test]
fn rejects_preexisting_database_with_zero_application_id() {
    let path = repository("zero-application-id");
    let control = path.join(".rgit");
    create_database(&control.join("metadata/repository.sqlite3"), 0, 0);

    assert!(matches!(
        SqliteStore::open(&control),
        Err(StoreError::UnsupportedDatabase)
    ));

    fs::remove_dir_all(path).expect("cleanup");
}

#[test]
fn reports_typed_errors_for_lower_and_newer_schema_versions() {
    let lower_path = repository("lower-version");
    let lower_control = lower_path.join(".rgit");
    create_database(
        &lower_control.join("metadata/repository.sqlite3"),
        APPLICATION_ID,
        0,
    );
    assert!(matches!(
        SqliteStore::open(&lower_control),
        Err(StoreError::MigrationRequired)
    ));
    fs::remove_dir_all(lower_path).expect("lower-version cleanup");

    let newer_path = repository("newer-version");
    let newer_control = newer_path.join(".rgit");
    create_database(
        &newer_control.join("metadata/repository.sqlite3"),
        APPLICATION_ID,
        2,
    );
    assert!(matches!(
        SqliteStore::open(&newer_control),
        Err(StoreError::UpgradeRequired)
    ));
    fs::remove_dir_all(newer_path).expect("newer-version cleanup");
}

#[cfg(unix)]
#[test]
fn rejects_hardlinked_database_file() {
    let path = repository("hardlink");
    let control = path.join(".rgit");
    let store = SqliteStore::open(&control).expect("initialize database");
    let database = store.database_path();
    drop(store);

    fs::hard_link(&database, path.join("repository.sqlite3.alias")).expect("create hard link");
    assert!(matches!(
        SqliteStore::open(&control),
        Err(StoreError::UnsupportedDatabase)
    ));

    fs::remove_dir_all(path).expect("cleanup");
}
