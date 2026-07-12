//! Transactional SQLite metadata paired with the immutable loose-object store.

use std::{
    collections::BTreeSet,
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use rgit_objects::{ChangeId, LineId, ObjectId, ObjectKind, ReferenceEdge, ReferenceRole, Value};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use sha2::{Digest, Sha256};

use crate::{
    Closure, ExpectedRef, LooseObjectStore, MemoryStore, ObjectPresence, Publication,
    PublicationValidator, PutOutcome, ReferenceKey, ReferenceState, Store, StoreError,
    StoredObject,
    memory::{MemorySnapshot, reference_identity_matches},
    platform,
    sqlite_vfs::{PinnedSqliteRegistration, VFS_NAME},
};

const APPLICATION_ID: i64 = 0x5247_4954;
const SCHEMA_VERSION: i64 = 1;
const MAX_SQLITE_INTEGER: u64 = i64::MAX as u64;
const MIGRATION_1_SHA256: [u8; 32] = [
    0xd7, 0x4f, 0x54, 0x3f, 0xc8, 0xb7, 0x89, 0x73, 0x8e, 0x98, 0xbc, 0x30, 0x03, 0x8c, 0x55, 0x1e,
    0x73, 0x98, 0x8e, 0xa5, 0xd2, 0x67, 0x62, 0x36, 0x91, 0xa2, 0x55, 0x30, 0x39, 0xd4, 0x82, 0x9f,
];

type BlobIntegerInteger = (Vec<u8>, i64, i64);
type BlobIntegerBlob = (Vec<u8>, i64, Vec<u8>);

struct IncidentEvidence {
    id: [u8; 16],
    relative_path: String,
    safe_hash: [u8; 32],
    evidence_hash: [u8; 32],
}

fn acquire_file_lock(
    file: fs::File,
    timeout: Duration,
) -> Result<platform::ExclusiveFileLock, StoreError> {
    let deadline = Instant::now() + timeout;
    loop {
        match platform::try_lock_exclusive(file.try_clone().map_err(|_| StoreError::Database)?) {
            Ok(lock) => return Ok(lock),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || matches!(error.raw_os_error(), Some(11 | 35)) =>
            {
                if Instant::now() >= deadline {
                    return Err(StoreError::RetryableConflict);
                }
                std::thread::yield_now();
            }
            Err(_) => return Err(StoreError::UnsupportedDatabase),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqliteFailurePoint {
    AfterBegin,
    AfterInventory,
    AfterReferences,
    BeforeCommit,
    AfterCommit,
}

pub trait TransactionFailureInjector: Send + Sync {
    fn check(&self, point: SqliteFailurePoint) -> Result<(), StoreError>;
}

#[derive(Debug, Default)]
pub struct NoTransactionFailures;

impl TransactionFailureInjector for NoTransactionFailures {
    fn check(&self, _point: SqliteFailurePoint) -> Result<(), StoreError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct SqliteStoreOptions {
    pub busy_timeout_millis: u32,
    pub failure_injector: Arc<dyn TransactionFailureInjector>,
}

impl fmt::Debug for SqliteStoreOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SqliteStoreOptions")
            .field("busy_timeout_millis", &self.busy_timeout_millis)
            .finish_non_exhaustive()
    }
}

impl Default for SqliteStoreOptions {
    fn default() -> Self {
        Self {
            busy_timeout_millis: 5_000,
            failure_injector: Arc::new(NoTransactionFailures),
        }
    }
}

/// A durable Store implementation. Canonical bytes live only in verified loose
/// records; SQLite contains inventory, typed edges, graph projections and mutable
/// reference checkpoints.
pub struct SqliteStore {
    control: PathBuf,
    loose: LooseObjectStore,
    connection: Mutex<Connection>,
    _vfs_registration: PinnedSqliteRegistration,
    writer: Mutex<()>,
    memory: MemoryStore,
    injector: Arc<dyn TransactionFailureInjector>,
    control_handle: platform::DirectoryHandle,
    metadata_handle: platform::DirectoryHandle,
    _locks_handle: platform::DirectoryHandle,
    incidents_handle: platform::DirectoryHandle,
    writer_lock_file: fs::File,
    writer_lock_identity: platform::FileIdentity,
    database_identity: platform::FileIdentity,
    busy_timeout: Duration,
}

impl fmt::Debug for SqliteStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SqliteStore")
            .field("repository", &"<restricted>")
            .finish_non_exhaustive()
    }
}

impl SqliteStore {
    pub fn open(control: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_options(control, SqliteStoreOptions::default())
    }

    pub fn open_with_options(
        control: impl AsRef<Path>,
        options: SqliteStoreOptions,
    ) -> Result<Self, StoreError> {
        #[cfg(not(unix))]
        return Err(StoreError::UnsupportedDatabase);
        let control = control.as_ref().to_path_buf();
        if !control.is_absolute() {
            return Err(StoreError::UnsupportedDatabase);
        }
        let loose = LooseObjectStore::open(&control).map_err(|_| StoreError::ObjectStorage)?;
        let control_handle =
            platform::open_directory(&control).map_err(|_| StoreError::Database)?;
        let metadata = control.join("metadata");
        let metadata_handle = open_or_create_directory(&control_handle, "metadata")?;
        let locks_handle = open_or_create_directory(&control_handle, "locks")?;
        let incidents_handle = open_or_create_directory(&control_handle, "incidents")?;
        ensure_same_filesystem(
            &control_handle,
            [&metadata_handle, &locks_handle, &incidents_handle],
        )?;
        let writer_lock_file = platform::open_lock_file_at(&locks_handle, "repository.write")
            .map_err(|_| StoreError::UnsupportedDatabase)?;
        writer_lock_file
            .sync_all()
            .map_err(|_| StoreError::Database)?;
        platform::sync_handle(&locks_handle).map_err(|_| StoreError::Database)?;
        let writer_lock_identity =
            platform::file_identity(&writer_lock_file).map_err(|_| StoreError::Database)?;
        let startup_timeout = Duration::from_millis(u64::from(options.busy_timeout_millis));
        let _startup_lock = acquire_file_lock(
            writer_lock_file
                .try_clone()
                .map_err(|_| StoreError::Database)?,
            startup_timeout,
        )?;
        let path = metadata.join("repository.sqlite3");
        let vfs_registration = PinnedSqliteRegistration::register(metadata_handle.raw_fd(), &path)
            .map_err(|_| StoreError::UnsupportedDatabase)?;
        let (database_existed, pinned_database) =
            match platform::open_file_at(&metadata_handle, "repository.sqlite3") {
                Ok(file) => (true, file),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                    false,
                    platform::create_file_at(&metadata_handle, "repository.sqlite3")
                        .map_err(|_| StoreError::Database)?,
                ),
                Err(_) => return Err(StoreError::UnsupportedDatabase),
            };
        if !platform::regular_single_link(&pinned_database).map_err(|_| StoreError::Database)? {
            return Err(StoreError::UnsupportedDatabase);
        }
        if !database_existed {
            pinned_database
                .sync_all()
                .map_err(|_| StoreError::Database)?;
            platform::sync_handle(&metadata_handle).map_err(|_| StoreError::Database)?;
        }
        let pinned_identity =
            platform::file_identity(&pinned_database).map_err(|_| StoreError::Database)?;
        let mut connection = Connection::open_with_flags_and_vfs(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            VFS_NAME,
        )
        .map_err(map_database_error)?;
        let database_file = platform::open_file_at(&metadata_handle, "repository.sqlite3")
            .map_err(|_| StoreError::Database)?;
        if !platform::regular_single_link(&database_file).map_err(|_| StoreError::Database)?
            || !platform::same_entry(&pinned_identity, &database_file)
                .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::UnsupportedDatabase);
        }
        let database_identity =
            platform::file_identity(&database_file).map_err(|_| StoreError::Database)?;
        // Reject foreign/newer databases before changing any persistent pragma.
        let initial_application_id = pragma_i64(&connection, "application_id")?;
        let initial_user_version = pragma_i64(&connection, "user_version")?;
        if initial_application_id != 0 && initial_application_id != APPLICATION_ID {
            return Err(StoreError::UnsupportedDatabase);
        }
        if initial_user_version > SCHEMA_VERSION {
            return Err(StoreError::UpgradeRequired);
        }
        if initial_application_id == 0 && database_existed {
            return Err(StoreError::UnsupportedDatabase);
        }
        if initial_application_id == APPLICATION_ID && initial_user_version < SCHEMA_VERSION {
            return Err(StoreError::MigrationRequired);
        }
        configure_connection(&connection, options.busy_timeout_millis)?;
        initialize_or_verify(&mut connection)?;
        reconcile_orphan_evidence(&mut connection, &incidents_handle)?;
        verify_incidents(&connection, &incidents_handle)?;
        startup_wal_probe(&connection, &path, options.busy_timeout_millis)?;
        verify_sidecar_entries(&metadata_handle)?;
        let reopened = Connection::open_with_flags_and_vfs(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            VFS_NAME,
        )
        .map_err(map_database_error)?;
        configure_connection(&reopened, options.busy_timeout_millis)?;
        verify_header(&reopened)?;
        let expected_checkpoint: (Vec<u8>, i64) = connection
            .query_row(
                "SELECT repository_id,revision FROM repository WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(map_database_error)?;
        let observed_checkpoint: (Vec<u8>, i64) = reopened
            .query_row(
                "SELECT repository_id,revision FROM repository WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(map_database_error)?;
        if observed_checkpoint != expected_checkpoint {
            return Err(StoreError::UnsupportedDatabase);
        }
        drop(reopened);
        let read = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(map_database_error)?;
        let snapshot = load_snapshot(&read, &loose)?;
        read.commit().map_err(map_database_error)?;
        Ok(Self {
            control,
            loose,
            connection: Mutex::new(connection),
            _vfs_registration: vfs_registration,
            writer: Mutex::new(()),
            memory: MemoryStore::from_snapshot(snapshot),
            injector: options.failure_injector,
            control_handle,
            metadata_handle,
            _locks_handle: locks_handle,
            incidents_handle,
            writer_lock_file,
            writer_lock_identity,
            database_identity,
            busy_timeout: Duration::from_millis(u64::from(options.busy_timeout_millis)),
        })
    }

    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.control.join("metadata/repository.sqlite3")
    }

    pub fn verify_metadata(&self) -> Result<(), StoreError> {
        self.refresh()?;
        let connection = self.connection();
        verify_header(&connection)?;
        verify_schema(&connection)?;
        let quick: String = connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(map_database_error)?;
        if quick != "ok" {
            return Err(StoreError::UnsupportedDatabase);
        }
        let foreign_key_error: Option<String> = connection
            .query_row(
                "SELECT printf('%s', \"table\") FROM pragma_foreign_key_check LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_database_error)?;
        if foreign_key_error.is_some() {
            return Err(StoreError::UnsupportedDatabase);
        }
        Ok(())
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn begin_checked<'a>(
        &'a self,
        connection: &'a mut Connection,
        revision: u64,
    ) -> Result<Transaction<'a>, StoreError> {
        self.verify_database_entry()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_database_error)?;
        self.injector.check(SqliteFailurePoint::AfterBegin)?;
        let actual: i64 = transaction
            .query_row(
                "SELECT revision FROM repository WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(map_database_error)?;
        if u64::try_from(actual).ok() != Some(revision) {
            return Err(StoreError::ReferenceConflict);
        }
        Ok(transaction)
    }

    fn finish(
        &self,
        transaction: Transaction<'_>,
        base_revision: u64,
        staged: MemorySnapshot,
    ) -> Result<(), StoreError> {
        self.injector.check(SqliteFailurePoint::BeforeCommit)?;
        transaction.commit().map_err(map_database_error)?;
        if let Err(error) = self.injector.check(SqliteFailurePoint::AfterCommit) {
            // The durable state is authoritative after an unknown commit outcome.
            self.memory.replace_if_revision(base_revision, staged)?;
            return Err(error);
        }
        self.memory.replace_if_revision(base_revision, staged)
    }

    fn stage(&self) -> (u64, MemoryStore) {
        let snapshot = self.memory.snapshot();
        (snapshot.revision, MemoryStore::from_snapshot(snapshot))
    }

    fn refresh(&self) -> Result<(), StoreError> {
        self.verify_database_entry()?;
        verify_sidecar_entries(&self.metadata_handle)?;
        let mut connection = self.connection();
        let read = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(map_database_error)?;
        verify_incidents(&read, &self.incidents_handle)?;
        let snapshot = load_snapshot(&read, &self.loose)?;
        read.commit().map_err(map_database_error)?;
        drop(connection);
        self.memory.replace(snapshot);
        Ok(())
    }

    fn process_writer_lock(&self) -> Result<platform::ExclusiveFileLock, StoreError> {
        let observed = platform::open_lock_file_at(&self._locks_handle, "repository.write")
            .map_err(|_| StoreError::UnsupportedDatabase)?;
        if !platform::same_entry(&self.writer_lock_identity, &observed)
            .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::UnsupportedDatabase);
        }
        acquire_file_lock(
            self.writer_lock_file
                .try_clone()
                .map_err(|_| StoreError::Database)?,
            self.busy_timeout,
        )
    }

    fn verify_database_entry(&self) -> Result<(), StoreError> {
        let observed_control =
            platform::open_directory(&self.control).map_err(|_| StoreError::ObjectStorage)?;
        if !platform::same_directory_entry(&self.control_handle, &observed_control)
            .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::UnsupportedDatabase);
        }
        let observed_metadata = platform::open_directory_at(&observed_control, "metadata")
            .map_err(|_| StoreError::ObjectStorage)?;
        if !platform::same_directory_entry(&self.metadata_handle, &observed_metadata)
            .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::UnsupportedDatabase);
        }
        let file = platform::open_file_at(&self.metadata_handle, "repository.sqlite3")
            .map_err(|_| StoreError::ObjectStorage)?;
        if !platform::regular_single_link(&file).map_err(|_| StoreError::Database)?
            || !platform::same_entry(&self.database_identity, &file)
                .map_err(|_| StoreError::Database)?
        {
            return Err(StoreError::UnsupportedDatabase);
        }
        Ok(())
    }

    fn checkpoint(&self) -> Result<(), StoreError> {
        checkpoint(&self.connection(), "PASSIVE")
    }

    fn ensure_mutable(&self) -> Result<(), StoreError> {
        let incident: i64 = self
            .connection()
            .query_row(
                "SELECT incident_read_only FROM repository WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(map_database_error)?;
        if incident != 0 {
            return Err(StoreError::IncidentReadOnly);
        }
        Ok(())
    }

    fn create_incident_evidence(&self, id: &ObjectId) -> Result<IncidentEvidence, StoreError> {
        let incident: Vec<u8> = self
            .connection()
            .query_row("SELECT randomblob(16)", [], |row| row.get(0))
            .map_err(map_database_error)?;
        let incident: [u8; 16] = incident
            .try_into()
            .map_err(|_| StoreError::UnsupportedDatabase)?;
        let name = format!("{}.evidence", hex::encode(incident));
        let mut evidence = b"RGIT-RESTRICTED-INCIDENT\0".to_vec();
        evidence.extend_from_slice(&id.to_bytes());
        let evidence_hash: [u8; 32] = Sha256::digest(&evidence).into();
        let mut safe = Sha256::new();
        safe.update(b"RGIT-SAFE-INCIDENT-IDENTITY\0");
        safe.update(id.to_bytes());
        let safe_hash: [u8; 32] = safe.finalize().into();
        let mut file = platform::create_file_at(&self.incidents_handle, &name)
            .map_err(|_| StoreError::ObjectStorage)?;
        file.write_all(&evidence)
            .map_err(|_| StoreError::ObjectStorage)?;
        file.sync_all().map_err(|_| StoreError::ObjectStorage)?;
        platform::sync_handle(&self.incidents_handle).map_err(|_| StoreError::ObjectStorage)?;
        Ok(IncidentEvidence {
            id: incident,
            relative_path: format!("incidents/{name}"),
            safe_hash,
            evidence_hash,
        })
    }
}

impl Store for SqliteStore {
    fn put(&self, id: ObjectId, bytes: Vec<u8>) -> Result<PutOutcome, StoreError> {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _process_writer = self.process_writer_lock()?;
        self.refresh()?;
        self.ensure_mutable()?;
        let (base_revision, staged_store) = self.stage();
        let outcome = staged_store.put(id.clone(), bytes.clone())?;
        if outcome == PutOutcome::AlreadyPresent {
            return Ok(outcome);
        }
        self.loose
            .put(&id, &bytes)
            .map_err(|_| StoreError::ObjectStorage)?;
        let object = staged_store.get(&id)?;
        let mut staged = staged_store.snapshot();
        add_edge_promises(&mut staged, &object);
        let staged_store = MemoryStore::from_snapshot(staged.clone());
        let mut connection = self.connection();
        let transaction = self.begin_checked(&mut connection, base_revision)?;
        verify_durable_object(&self.loose, &object)?;
        index_object(&transaction, &self.control, &object, base_revision + 1)?;
        rebuild_graph_indexes(&transaction, &staged_store)?;
        set_revision(&transaction, staged.revision)?;
        self.injector.check(SqliteFailurePoint::AfterInventory)?;
        self.finish(transaction, base_revision, staged)?;
        drop(connection);
        self.checkpoint()?;
        Ok(PutOutcome::New)
    }

    fn get(&self, id: &ObjectId) -> Result<StoredObject, StoreError> {
        self.refresh()?;
        self.memory.get(id)
    }

    fn presence(&self, id: &ObjectId) -> Option<ObjectPresence> {
        self.refresh().ok()?;
        self.memory.presence(id)
    }

    fn mark_promised(&self, id: ObjectId) -> Result<(), StoreError> {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _process_writer = self.process_writer_lock()?;
        self.refresh()?;
        self.ensure_mutable()?;
        let (base_revision, staged_store) = self.stage();
        staged_store.mark_promised(id.clone())?;
        let staged = staged_store.snapshot();
        if staged.revision == base_revision {
            return Ok(());
        }
        let mut connection = self.connection();
        let transaction = self.begin_checked(&mut connection, base_revision)?;
        transaction
            .execute(
                "INSERT INTO objects(object_id,presence,first_seen_revision) VALUES(?1,2,?2)
                 ON CONFLICT(object_id) DO NOTHING",
                params![id.to_bytes(), to_sql_u64(staged.revision)?],
            )
            .map_err(map_database_error)?;
        set_revision(&transaction, staged.revision)?;
        self.injector.check(SqliteFailurePoint::AfterInventory)?;
        self.finish(transaction, base_revision, staged)?;
        drop(connection);
        self.checkpoint()
    }

    fn quarantine(&self, id: &ObjectId) -> Result<(), StoreError> {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _process_writer = self.process_writer_lock()?;
        self.refresh()?;
        let (base_revision, staged_store) = self.stage();
        staged_store.quarantine(id)?;
        let mut staged = staged_store.snapshot();
        if staged.revision == base_revision {
            return Ok(());
        }
        staged.objects.remove(id);
        staged.promised.remove(id);
        let staged_store = MemoryStore::from_snapshot(staged.clone());
        let evidence = self.create_incident_evidence(id)?;
        let mut connection = self.connection();
        let transaction = self.begin_checked(&mut connection, base_revision)?;
        transaction
            .execute(
                "DELETE FROM object_locations WHERE object_id=?1",
                [id.to_bytes()],
            )
            .map_err(map_database_error)?;
        transaction
            .execute(
                "DELETE FROM object_edges WHERE source_id=?1",
                [id.to_bytes()],
            )
            .map_err(map_database_error)?;
        transaction
            .execute(
                "UPDATE objects SET presence=3, canonical_length=NULL WHERE object_id=?1",
                [id.to_bytes()],
            )
            .map_err(map_database_error)?;
        rebuild_graph_indexes(&transaction, &staged_store)?;
        transaction
            .execute(
                "INSERT INTO incidents(
                    incident_id,affected_object_id,safe_identity_sha256,evidence_sha256,
                    reason,evidence_relative_path,fail_closed,state,created_revision,
                    created_utc_seconds)
                 VALUES(?1,?2,?3,?4,2,?5,1,1,?6,unixepoch())",
                params![
                    evidence.id,
                    id.to_bytes(),
                    evidence.safe_hash,
                    evidence.evidence_hash,
                    evidence.relative_path,
                    to_sql_u64(staged.revision)?
                ],
            )
            .map_err(map_database_error)?;
        transaction
            .execute(
                "UPDATE repository SET incident_read_only=1 WHERE singleton=1",
                [],
            )
            .map_err(map_database_error)?;
        set_revision(&transaction, staged.revision)?;
        self.injector.check(SqliteFailurePoint::AfterInventory)?;
        self.finish(transaction, base_revision, staged)?;
        drop(connection);
        self.checkpoint()
    }

    fn reference(&self, key: &ReferenceKey) -> Option<ReferenceState> {
        self.refresh().ok()?;
        self.memory.reference(key)
    }

    fn compare_and_swap(
        &self,
        key: ReferenceKey,
        expected: ExpectedRef,
        target: ObjectId,
        operation: ObjectId,
        validator: &dyn PublicationValidator,
    ) -> Result<ReferenceState, StoreError> {
        let publication = Publication {
            objects: Vec::new(),
            updates: vec![crate::RefUpdate {
                key,
                expected,
                target,
            }],
            operation,
        };
        self.publish(publication, validator)?
            .into_iter()
            .next()
            .ok_or(StoreError::ReferenceConflict)
    }

    fn publish(
        &self,
        publication: Publication,
        validator: &dyn PublicationValidator,
    ) -> Result<Vec<ReferenceState>, StoreError> {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _process_writer = self.process_writer_lock()?;
        self.refresh()?;
        self.ensure_mutable()?;
        let (base_revision, staged_store) = self.stage();
        let results = staged_store.publish(publication.clone(), validator)?;

        // Immutable bytes are made durable before SQLite visibility.
        for candidate in &publication.objects {
            self.loose
                .put(&candidate.id, &candidate.bytes)
                .map_err(|_| StoreError::ObjectStorage)?;
        }
        let mut staged = staged_store.snapshot();
        for candidate in &publication.objects {
            add_edge_promises(&mut staged, &staged_store.get(&candidate.id)?);
        }
        let staged_store = MemoryStore::from_snapshot(staged.clone());
        let mut connection = self.connection();
        let transaction = self.begin_checked(&mut connection, base_revision)?;
        for candidate in &publication.objects {
            let object = staged_store.get(&candidate.id)?;
            verify_durable_object(&self.loose, &object)?;
            index_object(&transaction, &self.control, &object, staged.revision)?;
        }
        rebuild_graph_indexes(&transaction, &staged_store)?;
        self.injector.check(SqliteFailurePoint::AfterInventory)?;
        for (update, state) in publication.updates.iter().zip(&results) {
            write_reference_cas(&transaction, update, state, &publication.operation)?;
        }
        self.injector.check(SqliteFailurePoint::AfterReferences)?;
        set_revision(&transaction, staged.revision)?;
        self.finish(transaction, base_revision, staged)?;
        drop(connection);
        self.checkpoint()?;
        Ok(results)
    }

    fn closure(&self, roots: &[ReferenceEdge]) -> Result<Closure, StoreError> {
        self.refresh()?;
        self.memory.closure(roots)
    }

    fn generation(&self, id: &ObjectId) -> Result<u64, StoreError> {
        self.refresh()?;
        self.memory.generation(id)
    }

    fn is_reachable(&self, ancestor: &ObjectId, descendant: &ObjectId) -> Result<bool, StoreError> {
        self.refresh()?;
        self.memory.is_reachable(ancestor, descendant)
    }
}

fn configure_connection(
    connection: &Connection,
    busy_timeout_millis: u32,
) -> Result<(), StoreError> {
    if rusqlite::version_number() < 3_037_000 {
        return Err(StoreError::UnsupportedDatabase);
    }
    connection
        .load_extension_disable()
        .map_err(map_database_error)?;
    verify_compile_options(connection)?;
    connection
        .busy_timeout(std::time::Duration::from_millis(u64::from(
            busy_timeout_millis,
        )))
        .map_err(map_database_error)?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA trusted_schema=OFF;
             PRAGMA recursive_triggers=OFF;
             PRAGMA temp_store=MEMORY;
             PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA wal_autocheckpoint=0;
             PRAGMA journal_size_limit=67108864;",
        )
        .map_err(map_database_error)?;
    connection
        .set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(map_database_error)?;
    use rusqlite::limits::Limit;
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, 256 * 1024 * 1024),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 1024 * 1024),
        (Limit::SQLITE_LIMIT_COLUMN, 256),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 16),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 1024),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
    ] {
        connection.set_limit(limit, value);
        if connection.limit(limit) > value {
            return Err(StoreError::UnsupportedDatabase);
        }
    }
    // Repository SQL is fixed and short. The handler is a final bound against a
    // corrupt planner or accidental unbounded statement.
    let mut callbacks = 0_u32;
    connection.progress_handler(
        10_000,
        Some(move || {
            callbacks = callbacks.saturating_add(1);
            callbacks > 100_000
        }),
    );
    verify_pragma_i64(connection, "foreign_keys", 1)?;
    verify_pragma_i64(connection, "trusted_schema", 0)?;
    verify_pragma_i64(connection, "recursive_triggers", 0)?;
    verify_pragma_i64(connection, "temp_store", 2)?;
    verify_pragma_i64(connection, "busy_timeout", i64::from(busy_timeout_millis))?;
    verify_pragma_i64(connection, "synchronous", 2)?;
    verify_pragma_i64(connection, "wal_autocheckpoint", 0)?;
    verify_pragma_i64(connection, "journal_size_limit", 67_108_864)?;
    let journal: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(map_database_error)?;
    if !journal.eq_ignore_ascii_case("wal") {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn verify_compile_options(connection: &Connection) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare("PRAGMA compile_options")
        .map_err(map_database_error)?;
    let options = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(map_database_error)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(map_database_error)?;
    if !options.contains("THREADSAFE=1")
        || options.contains("OMIT_FOREIGN_KEY")
        || options.contains("OMIT_TRIGGER")
    {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn initialize_or_verify(connection: &mut Connection) -> Result<(), StoreError> {
    let application_id = pragma_i64(connection, "application_id")?;
    let user_version = pragma_i64(connection, "user_version")?;
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err(StoreError::UnsupportedDatabase);
    }
    if user_version > SCHEMA_VERSION {
        return Err(StoreError::UpgradeRequired);
    }
    if application_id == 0 && user_version == 0 {
        initialize_schema(connection)?;
    } else if application_id != APPLICATION_ID || user_version != SCHEMA_VERSION {
        return Err(StoreError::UnsupportedDatabase);
    }
    verify_header(connection)?;
    verify_schema(connection)?;
    verify_integrity(connection)
}

fn initialize_schema(connection: &mut Connection) -> Result<(), StoreError> {
    let ddl = schema_ddl()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_database_error)?;
    transaction.execute_batch(ddl).map_err(map_database_error)?;
    let migration_hash = migration_1_digest()?;
    transaction
        .execute(
            "INSERT INTO schema_migrations
             (version,name,migration_sha256,sqlite_version,applied_utc_seconds,previous_version)
             VALUES(1,'initial',?1,sqlite_version(),unixepoch(),0)",
            [migration_hash.as_slice()],
        )
        .map_err(map_database_error)?;
    transaction
        .execute(
            "INSERT INTO repository
             (singleton,repository_id,repository_format,loose_record_format,write_hash_code,revision)
             VALUES(1,randomblob(16),0,0,18,0)",
            [],
        )
        .map_err(map_database_error)?;
    transaction
        .execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={SCHEMA_VERSION};"
        ))
        .map_err(map_database_error)?;
    transaction.commit().map_err(map_database_error)
}

fn verify_header(connection: &Connection) -> Result<(), StoreError> {
    if pragma_i64(connection, "application_id")? != APPLICATION_ID {
        return Err(StoreError::UnsupportedDatabase);
    }
    let version = pragma_i64(connection, "user_version")?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::UpgradeRequired);
    }
    if version != SCHEMA_VERSION {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn verify_schema(connection: &Connection) -> Result<(), StoreError> {
    if schema_fingerprint(connection)? != expected_schema_fingerprint()? {
        return Err(StoreError::UnsupportedDatabase);
    }
    let expected = edge_roles()?
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut statement = connection
        .prepare("SELECT role FROM edge_role_registry ORDER BY role")
        .map_err(map_database_error)?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(map_database_error)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(map_database_error)?;
    if actual != expected {
        return Err(StoreError::UnsupportedDatabase);
    }
    let ledger = connection
        .prepare(
            "SELECT version,name,migration_sha256,sqlite_version,previous_version
             FROM schema_migrations ORDER BY version",
        )
        .map_err(map_database_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(map_database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_database_error)?;
    if ledger.len() != 1
        || ledger[0].0 != 1
        || ledger[0].1 != "initial"
        || ledger[0].2 != MIGRATION_1_SHA256
        || ledger[0].3.is_empty()
        || ledger[0].4 != 0
    {
        return Err(StoreError::UnsupportedDatabase);
    }
    let repository_rows: i64 = connection
        .query_row("SELECT count(*) FROM repository", [], |row| row.get(0))
        .map_err(map_database_error)?;
    if repository_rows != 1 {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn verify_integrity(connection: &Connection) -> Result<(), StoreError> {
    let quick: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(map_database_error)?;
    if quick != "ok" {
        return Err(StoreError::UnsupportedDatabase);
    }
    let foreign_key_error: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_database_error)?;
    if foreign_key_error.is_some() {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn startup_wal_probe(
    connection: &Connection,
    path: &Path,
    busy_timeout_millis: u32,
) -> Result<(), StoreError> {
    let original_revision: i64 = connection
        .query_row(
            "SELECT revision FROM repository WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(map_database_error)?;
    let probe_revision = original_revision
        .checked_add(1)
        .ok_or(StoreError::RevisionOverflow)?;
    connection
        .execute(
            "UPDATE repository SET revision=?1 WHERE singleton=1",
            [probe_revision],
        )
        .map_err(map_database_error)?;
    let (busy, log_frames, checkpointed): (i64, i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(map_database_error)?;
    if busy != 0 || log_frames <= 0 || checkpointed <= 0 {
        return Err(StoreError::UnsupportedDatabase);
    }
    let observer = Connection::open_with_flags_and_vfs(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        VFS_NAME,
    )
    .map_err(map_database_error)?;
    configure_connection(&observer, busy_timeout_millis)?;
    let observed: i64 = observer
        .query_row(
            "SELECT revision FROM repository WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(map_database_error)?;
    if observed != probe_revision {
        return Err(StoreError::UnsupportedDatabase);
    }
    drop(observer);
    connection
        .execute(
            "UPDATE repository SET revision=?1 WHERE singleton=1",
            [original_revision],
        )
        .map_err(map_database_error)?;
    checkpoint(connection, "FULL")
}

fn checkpoint(connection: &Connection, mode: &str) -> Result<(), StoreError> {
    let sql = match mode {
        "FULL" => "PRAGMA wal_checkpoint(FULL)",
        "PASSIVE" => "PRAGMA wal_checkpoint(PASSIVE)",
        _ => return Err(StoreError::UnsupportedDatabase),
    };
    let (busy, log_frames, checkpointed): (i64, i64, i64) = connection
        .query_row(sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(map_database_error)?;
    if busy != 0 || checkpointed < 0 || log_frames < checkpointed {
        return Err(StoreError::RetryableConflict);
    }
    Ok(())
}

fn verify_sidecar_entries(metadata: &platform::DirectoryHandle) -> Result<(), StoreError> {
    for name in ["repository.sqlite3-wal", "repository.sqlite3-shm"] {
        match platform::open_file_at(metadata, name) {
            Ok(file) => {
                if !platform::regular_single_link(&file).map_err(|_| StoreError::Database)? {
                    return Err(StoreError::UnsupportedDatabase);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StoreError::UnsupportedDatabase),
        }
    }
    Ok(())
}

fn verify_incidents(
    connection: &Connection,
    incidents: &platform::DirectoryHandle,
) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT affected_object_id,evidence_sha256,evidence_relative_path,fail_closed,state
             FROM incidents ORDER BY incident_id",
        )
        .map_err(map_database_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(map_database_error)?;
    let mut open_fail_closed = false;
    for row in rows {
        let (affected, expected_hash, relative, fail_closed, state) =
            row.map_err(map_database_error)?;
        let name = relative
            .strip_prefix("incidents/")
            .filter(|name| !name.is_empty() && !name.contains(['/', '\\', '\0']))
            .ok_or(StoreError::UnsupportedDatabase)?;
        let file =
            platform::open_file_at(incidents, name).map_err(|_| StoreError::UnsupportedDatabase)?;
        if !platform::regular_single_link(&file).map_err(|_| StoreError::Database)? {
            return Err(StoreError::UnsupportedDatabase);
        }
        let mut bytes = Vec::new();
        file.take(1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|_| StoreError::Database)?;
        if Sha256::digest(&bytes).as_slice() != expected_hash {
            return Err(StoreError::UnsupportedDatabase);
        }
        if state == 1 {
            let presence: Option<i64> = connection
                .query_row(
                    "SELECT presence FROM objects WHERE object_id=?1",
                    [affected],
                    |row| row.get(0),
                )
                .optional()
                .map_err(map_database_error)?;
            if presence != Some(3) {
                return Err(StoreError::UnsupportedDatabase);
            }
            open_fail_closed |= fail_closed == 1;
        }
    }
    let incident_mode: i64 = connection
        .query_row(
            "SELECT incident_read_only FROM repository WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(map_database_error)?;
    if incident_mode != i64::from(open_fail_closed) {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn reconcile_orphan_evidence(
    connection: &mut Connection,
    incidents: &platform::DirectoryHandle,
) -> Result<(), StoreError> {
    let known = connection
        .prepare("SELECT evidence_relative_path FROM incidents")
        .map_err(map_database_error)?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(map_database_error)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(map_database_error)?;
    for name in platform::list_directory_names(incidents).map_err(|_| StoreError::Database)? {
        if !name.ends_with(".evidence") {
            return Err(StoreError::UnsupportedDatabase);
        }
        let relative = format!("incidents/{name}");
        if known.contains(&relative) {
            continue;
        }
        let incident_hex = name
            .strip_suffix(".evidence")
            .ok_or(StoreError::UnsupportedDatabase)?;
        let incident = hex::decode(incident_hex).map_err(|_| StoreError::UnsupportedDatabase)?;
        let incident: [u8; 16] = incident
            .try_into()
            .map_err(|_| StoreError::UnsupportedDatabase)?;
        let file = platform::open_file_at(incidents, &name)
            .map_err(|_| StoreError::UnsupportedDatabase)?;
        if !platform::regular_single_link(&file).map_err(|_| StoreError::Database)? {
            return Err(StoreError::UnsupportedDatabase);
        }
        let mut evidence = Vec::new();
        file.take(1024 * 1024)
            .read_to_end(&mut evidence)
            .map_err(|_| StoreError::Database)?;
        let prefix = b"RGIT-RESTRICTED-INCIDENT\0";
        let object_bytes = evidence
            .strip_prefix(prefix)
            .ok_or(StoreError::UnsupportedDatabase)?;
        let affected =
            ObjectId::from_bytes(object_bytes).map_err(|_| StoreError::UnsupportedDatabase)?;
        let evidence_hash: [u8; 32] = Sha256::digest(&evidence).into();
        let mut safe = Sha256::new();
        safe.update(b"RGIT-SAFE-INCIDENT-IDENTITY\0");
        safe.update(affected.to_bytes());
        let safe_hash: [u8; 32] = safe.finalize().into();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_database_error)?;
        let revision: i64 = transaction
            .query_row(
                "SELECT revision FROM repository WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(map_database_error)?;
        let revision = revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOverflow)?;
        transaction
            .execute(
                "INSERT INTO objects(object_id,presence,first_seen_revision)
                 VALUES(?1,3,?2)
                 ON CONFLICT(object_id) DO UPDATE SET presence=3,canonical_length=NULL",
                params![affected.to_bytes(), revision],
            )
            .map_err(map_database_error)?;
        transaction
            .execute(
                "DELETE FROM object_locations WHERE object_id=?1",
                [affected.to_bytes()],
            )
            .map_err(map_database_error)?;
        transaction
            .execute(
                "INSERT INTO incidents(
                    incident_id,affected_object_id,safe_identity_sha256,evidence_sha256,
                    reason,evidence_relative_path,fail_closed,state,created_revision,
                    created_utc_seconds)
                 VALUES(?1,?2,?3,?4,4,?5,1,1,?6,unixepoch())",
                params![
                    incident,
                    affected.to_bytes(),
                    safe_hash,
                    evidence_hash,
                    relative,
                    revision
                ],
            )
            .map_err(map_database_error)?;
        transaction
            .execute(
                "UPDATE repository SET revision=?1,incident_read_only=1 WHERE singleton=1",
                [revision],
            )
            .map_err(map_database_error)?;
        transaction.commit().map_err(map_database_error)?;
    }
    Ok(())
}

fn schema_fingerprint(
    connection: &Connection,
) -> Result<Vec<(String, String, String, String)>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT type,name,tbl_name,sql FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
        )
        .map_err(map_database_error)?;
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(map_database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_database_error)
}

fn expected_schema_fingerprint() -> Result<Vec<(String, String, String, String)>, StoreError> {
    let connection = Connection::open_in_memory().map_err(map_database_error)?;
    connection
        .execute_batch(schema_ddl()?)
        .map_err(map_database_error)?;
    schema_fingerprint(&connection)
}

fn load_snapshot(
    connection: &Connection,
    loose: &LooseObjectStore,
) -> Result<MemorySnapshot, StoreError> {
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM repository WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(map_database_error)?;
    let mut snapshot = MemorySnapshot {
        revision: u64::try_from(revision).map_err(|_| StoreError::UnsupportedDatabase)?,
        ..MemorySnapshot::default()
    };
    let mut statement = connection
        .prepare(
            "SELECT object_id,presence,kind,object_schema,canonical_length
             FROM objects ORDER BY object_id",
        )
        .map_err(map_database_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(map_database_error)?;
    for row in rows {
        let (bytes, presence, stored_kind, stored_schema, stored_length) =
            row.map_err(map_database_error)?;
        let id = ObjectId::from_bytes(&bytes).map_err(|_| StoreError::UnsupportedDatabase)?;
        match presence {
            1 => {
                let object = loose
                    .read_object_audited(&id)
                    .map_err(|_| StoreError::UnsupportedDatabase)?;
                if stored_kind != Some(object.kind() as i64)
                    || stored_schema != Some(object.object().decoded().schema_version() as i64)
                    || stored_length != i64::try_from(object.bytes().len()).ok()
                {
                    return Err(StoreError::UnsupportedDatabase);
                }
                verify_object_indexes(connection, &object)?;
                snapshot.objects.insert(id, object);
            }
            2 => {
                snapshot.promised.insert(id);
            }
            3 => {
                snapshot.quarantined.insert(id);
            }
            4 => {}
            _ => return Err(StoreError::UnsupportedDatabase),
        }
    }
    let mut statement = connection
        .prepare(
            "SELECT ref_kind,stable_id,target_id,generation,operation_id FROM \"references\"
             ORDER BY ref_kind,stable_id",
        )
        .map_err(map_database_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(map_database_error)?;
    for row in rows {
        let (kind, stable, target, generation, operation) = row.map_err(map_database_error)?;
        let key = decode_reference_key(kind, &stable)?;
        snapshot.references.insert(
            key,
            ReferenceState {
                target: ObjectId::from_bytes(&target)
                    .map_err(|_| StoreError::UnsupportedDatabase)?,
                generation: u64::try_from(generation)
                    .map_err(|_| StoreError::UnsupportedDatabase)?,
                operation: ObjectId::from_bytes(&operation)
                    .map_err(|_| StoreError::UnsupportedDatabase)?,
            },
        );
    }
    let verifier = MemoryStore::from_snapshot(snapshot.clone());
    for (key, state) in &snapshot.references {
        let target = snapshot
            .objects
            .get(&state.target)
            .ok_or(StoreError::UnsupportedDatabase)?;
        let operation = snapshot
            .objects
            .get(&state.operation)
            .ok_or(StoreError::UnsupportedDatabase)?;
        if target.kind() != key.expected_kind()
            || operation.kind() != ObjectKind::Operation
            || !reference_identity_matches(key, target)
            || (matches!(key, ReferenceKey::OperationHead) && state.target != state.operation)
        {
            return Err(StoreError::UnsupportedDatabase);
        }
        verifier
            .closure(&[
                ReferenceEdge {
                    role: ReferenceRole::OperationAfter,
                    expected_kind: Some(key.expected_kind()),
                    id: state.target.clone(),
                },
                ReferenceEdge {
                    role: ReferenceRole::OperationAfter,
                    expected_kind: Some(ObjectKind::Operation),
                    id: state.operation.clone(),
                },
            ])
            .map_err(|_| StoreError::UnsupportedDatabase)?;
        if !matches!(key, ReferenceKey::OperationHead)
            && !operation.references().iter().any(|edge| {
                edge.id == state.target && edge.expected_kind == Some(key.expected_kind())
            })
        {
            return Err(StoreError::UnsupportedDatabase);
        }
    }
    verify_derived_indexes(connection, &snapshot)?;
    Ok(snapshot)
}

fn verify_derived_indexes(
    connection: &Connection,
    snapshot: &MemorySnapshot,
) -> Result<(), StoreError> {
    let store = MemoryStore::from_snapshot(snapshot.clone());
    let mut expected_generations = Vec::new();
    let mut expected_operations = Vec::new();
    let mut expected_snapshot_parents = Vec::new();
    let mut expected_operation_parents = Vec::new();
    for object in snapshot.objects.values() {
        let generation = if matches!(object.kind(), ObjectKind::Snapshot | ObjectKind::Operation) {
            store.generation(object.id()).ok()
        } else {
            None
        };
        if let Some(generation) = generation {
            expected_generations.push((
                object.id().to_bytes(),
                object.kind() as i64,
                to_sql_u64(generation)?,
            ));
            if object.kind() == ObjectKind::Operation {
                expected_operations.push((
                    object.id().to_bytes(),
                    to_sql_u64(generation)?,
                    to_sql_u64(
                        value_unsigned_field(object.object().decoded().value(), 6)
                            .ok_or(StoreError::UnsupportedDatabase)?,
                    )?,
                ));
            }
        }
        let parent_role = match object.kind() {
            ObjectKind::Snapshot => Some(ReferenceRole::SnapshotParent),
            ObjectKind::Operation if generation.is_some() => Some(ReferenceRole::OperationParent),
            _ => None,
        };
        if let Some(parent_role) = parent_role {
            for (position, edge) in object
                .references()
                .iter()
                .filter(|edge| edge.role == parent_role)
                .enumerate()
            {
                let row = (
                    object.id().to_bytes(),
                    to_sql_usize(position)?,
                    edge.id.to_bytes(),
                );
                if object.kind() == ObjectKind::Snapshot {
                    expected_snapshot_parents.push(row);
                } else if store.generation(&edge.id).is_ok() {
                    expected_operation_parents.push(row);
                }
            }
        }
    }
    expected_generations.sort();
    expected_operations.sort();
    expected_snapshot_parents.sort();
    expected_operation_parents.sort();
    let actual_generations = query_i64_triples(
        connection,
        "SELECT object_id,kind,generation FROM graph_generations ORDER BY object_id",
    )?;
    let actual_operations = query_i64_triples(
        connection,
        "SELECT operation_id,generation,logical_time FROM operations ORDER BY operation_id",
    )?;
    let actual_snapshot_parents = query_blob_i64_blob(
        connection,
        "SELECT snapshot_id,parent_position,parent_id FROM snapshot_parents
         ORDER BY snapshot_id,parent_position",
    )?;
    let actual_operation_parents = query_blob_i64_blob(
        connection,
        "SELECT operation_id,parent_position,parent_id FROM operation_parents
         ORDER BY operation_id,parent_position",
    )?;
    if expected_generations != actual_generations
        || expected_operations != actual_operations
        || expected_snapshot_parents != actual_snapshot_parents
        || expected_operation_parents != actual_operation_parents
    {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn query_i64_triples(
    connection: &Connection,
    sql: &str,
) -> Result<Vec<BlobIntegerInteger>, StoreError> {
    let mut statement = connection.prepare(sql).map_err(map_database_error)?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(map_database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_database_error)
}

fn query_blob_i64_blob(
    connection: &Connection,
    sql: &str,
) -> Result<Vec<BlobIntegerBlob>, StoreError> {
    let mut statement = connection.prepare(sql).map_err(map_database_error)?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(map_database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_database_error)
}

fn verify_object_indexes(connection: &Connection, object: &StoredObject) -> Result<(), StoreError> {
    let location: Option<(String, i64)> = connection
        .query_row(
            "SELECT relative_path,stored_length FROM object_locations
             WHERE object_id=?1 AND location_kind=1 AND active=1",
            [object.id().to_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(map_database_error)?;
    let Some((relative_path, stored_length)) = location else {
        return Err(StoreError::UnsupportedDatabase);
    };
    if relative_path != loose_relative_path(object.id()) || stored_length <= 0 {
        return Err(StoreError::UnsupportedDatabase);
    }
    let mut statement = connection
        .prepare(
            "SELECT ordinal,role,expected_kind,target_id FROM object_edges
             WHERE source_id=?1 ORDER BY ordinal",
        )
        .map_err(map_database_error)?;
    let actual = statement
        .query_map([object.id().to_bytes()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(map_database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_database_error)?;
    if actual.len() != object.references().len() {
        return Err(StoreError::UnsupportedDatabase);
    }
    for (ordinal, (actual_ordinal, role, kind, target)) in actual.into_iter().enumerate() {
        let expected = &object.references()[ordinal];
        if actual_ordinal != ordinal as i64
            || role != role_name(expected.role)
            || kind != expected.expected_kind.map(|value| value as i64)
            || target != expected.id.to_bytes()
        {
            return Err(StoreError::UnsupportedDatabase);
        }
    }
    Ok(())
}

fn add_edge_promises(snapshot: &mut MemorySnapshot, object: &StoredObject) {
    for edge in object.references() {
        if !snapshot.objects.contains_key(&edge.id)
            && !snapshot.quarantined.contains(&edge.id)
            && !snapshot.promised.contains(&edge.id)
        {
            snapshot.promised.insert(edge.id.clone());
        }
    }
}

fn verify_durable_object(
    loose: &LooseObjectStore,
    expected: &StoredObject,
) -> Result<(), StoreError> {
    let observed = loose
        .read_object_audited(expected.id())
        .map_err(|_| StoreError::ObjectStorage)?;
    if observed.kind() != expected.kind() || observed.bytes() != expected.bytes() {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn index_object(
    transaction: &Transaction<'_>,
    control: &Path,
    object: &StoredObject,
    revision: u64,
) -> Result<(), StoreError> {
    let id = object.id().to_bytes();
    transaction
        .execute(
            "INSERT INTO objects(object_id,kind,object_schema,presence,canonical_length,first_seen_revision)
             VALUES(?1,?2,?3,1,?4,?5)
             ON CONFLICT(object_id) DO UPDATE SET kind=excluded.kind,object_schema=excluded.object_schema,
             presence=1,canonical_length=excluded.canonical_length,unavailable_reason=NULL",
            params![
                id,
                object.kind() as u64,
                object.object().decoded().schema_version(),
                to_sql_usize(object.bytes().len())?,
                to_sql_u64(revision)?,
            ],
        )
        .map_err(map_database_error)?;
    for edge in object.references() {
        transaction
            .execute(
                "INSERT INTO objects(object_id,kind,presence,first_seen_revision)
                 VALUES(?1,?2,2,?3)
                 ON CONFLICT(object_id) DO UPDATE SET kind=COALESCE(objects.kind,excluded.kind)",
                params![
                    edge.id.to_bytes(),
                    edge.expected_kind.map(|kind| kind as u64),
                    to_sql_u64(revision)?
                ],
            )
            .map_err(map_database_error)?;
    }
    transaction
        .execute("DELETE FROM object_edges WHERE source_id=?1", [id.clone()])
        .map_err(map_database_error)?;
    for (ordinal, edge) in object.references().iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO object_edges(source_id,ordinal,role,expected_kind,target_id)
                 VALUES(?1,?2,?3,?4,?5)",
                params![
                    id,
                    to_sql_usize(ordinal)?,
                    role_name(edge.role),
                    edge.expected_kind.map(|kind| kind as u64),
                    edge.id.to_bytes(),
                ],
            )
            .map_err(map_database_error)?;
    }
    let relative = loose_relative_path(object.id());
    let stored_length = fs::metadata(control.join(&relative))
        .map_err(|_| StoreError::ObjectStorage)?
        .len();
    transaction
        .execute(
            "DELETE FROM object_locations WHERE object_id=?1 AND location_kind=1",
            [&id],
        )
        .map_err(map_database_error)?;
    transaction
        .execute(
            "INSERT INTO object_locations(object_id,location_kind,relative_path,stored_length,active)
             VALUES(?1,1,?2,?3,1)",
            params![id, relative, to_sql_u64(stored_length)?],
        )
        .map_err(map_database_error)?;
    Ok(())
}

fn rebuild_graph_indexes(
    transaction: &Transaction<'_>,
    store: &MemoryStore,
) -> Result<(), StoreError> {
    transaction
        .execute_batch(
            "DELETE FROM snapshot_parents;
             DELETE FROM operation_parents;
             DELETE FROM operations;
             DELETE FROM graph_generations;",
        )
        .map_err(map_database_error)?;
    let snapshot = store.snapshot();
    for object in snapshot.objects.values() {
        if matches!(object.kind(), ObjectKind::Snapshot | ObjectKind::Operation) {
            if let Ok(generation) = store.generation(object.id()) {
                transaction
                    .execute(
                        "INSERT INTO graph_generations(object_id,kind,generation) VALUES(?1,?2,?3)",
                        params![
                            object.id().to_bytes(),
                            object.kind() as u64,
                            to_sql_u64(generation)?
                        ],
                    )
                    .map_err(map_database_error)?;
            }
        }
        if object.kind() == ObjectKind::Operation {
            let logical_time = value_unsigned_field(object.object().decoded().value(), 6)
                .ok_or(StoreError::InvalidGraph)?;
            if let Ok(generation) = store.generation(object.id()) {
                transaction
                    .execute(
                        "INSERT INTO operations(operation_id,generation,logical_time) VALUES(?1,?2,?3)",
                        params![
                            object.id().to_bytes(),
                            to_sql_u64(generation)?,
                            to_sql_u64(logical_time)?
                        ],
                    )
                    .map_err(map_database_error)?;
            }
        }
    }
    for object in snapshot.objects.values() {
        let (role, sql) = match object.kind() {
            ObjectKind::Snapshot => (
                ReferenceRole::SnapshotParent,
                "INSERT INTO snapshot_parents(snapshot_id,parent_position,parent_id) VALUES(?1,?2,?3)",
            ),
            ObjectKind::Operation => (
                ReferenceRole::OperationParent,
                "INSERT INTO operation_parents(operation_id,parent_position,parent_id) VALUES(?1,?2,?3)",
            ),
            _ => continue,
        };
        for (position, edge) in object
            .references()
            .iter()
            .filter(|edge| edge.role == role)
            .enumerate()
        {
            let project = match object.kind() {
                ObjectKind::Snapshot => true,
                ObjectKind::Operation => {
                    store.generation(object.id()).is_ok() && store.generation(&edge.id).is_ok()
                }
                _ => false,
            };
            if project {
                transaction
                    .execute(
                        sql,
                        params![
                            object.id().to_bytes(),
                            to_sql_usize(position)?,
                            edge.id.to_bytes()
                        ],
                    )
                    .map_err(map_database_error)?;
            }
        }
    }
    Ok(())
}

fn write_reference_cas(
    transaction: &Transaction<'_>,
    update: &crate::RefUpdate,
    new_state: &ReferenceState,
    operation: &ObjectId,
) -> Result<(), StoreError> {
    let (kind, stable) = encode_reference_key(&update.key);
    let changed = match &update.expected {
        ExpectedRef::Absent => transaction.execute(
            "INSERT INTO \"references\"
             (ref_kind,stable_id,target_id,target_kind,generation,operation_id)
             SELECT ?1,?2,?3,?4,0,?5
             WHERE NOT EXISTS(SELECT 1 FROM \"references\" WHERE ref_kind=?1 AND stable_id=?2)",
            params![kind, stable, new_state.target.to_bytes(), update.key.expected_kind() as u64, operation.to_bytes()],
        ),
        ExpectedRef::Exact(expected) => transaction.execute(
            "UPDATE \"references\" SET target_id=?1,target_kind=?2,generation=generation+1,operation_id=?3
             WHERE ref_kind=?4 AND stable_id=?5 AND target_id=?6 AND generation=?7
             AND operation_id=?8 AND generation<9223372036854775807",
            params![
                new_state.target.to_bytes(), update.key.expected_kind() as u64, operation.to_bytes(),
                kind, stable, expected.target.to_bytes(), to_sql_u64(expected.generation)?,
                expected.operation.to_bytes(),
            ],
        ),
    }
    .map_err(map_database_error)?;
    if changed != 1 {
        return Err(StoreError::ReferenceConflict);
    }
    Ok(())
}

fn set_revision(transaction: &Transaction<'_>, revision: u64) -> Result<(), StoreError> {
    transaction
        .execute(
            "UPDATE repository SET revision=?1 WHERE singleton=1",
            [to_sql_u64(revision)?],
        )
        .map_err(map_database_error)?;
    Ok(())
}

fn encode_reference_key(key: &ReferenceKey) -> (i64, Vec<u8>) {
    match key {
        ReferenceKey::Line(id) => (1, id.as_bytes().to_vec()),
        ReferenceKey::Change(id) => (2, id.as_bytes().to_vec()),
        ReferenceKey::OperationHead => (3, Vec::new()),
        ReferenceKey::Release(id) => (4, id.as_bytes().to_vec()),
        ReferenceKey::Marker(id) => (5, id.to_vec()),
    }
}

fn decode_reference_key(kind: i64, stable: &[u8]) -> Result<ReferenceKey, StoreError> {
    let stable: [u8; 16] = match kind {
        3 if stable.is_empty() => return Ok(ReferenceKey::OperationHead),
        _ => stable
            .try_into()
            .map_err(|_| StoreError::UnsupportedDatabase)?,
    };
    Ok(match kind {
        1 => ReferenceKey::Line(LineId::from_bytes(stable)),
        2 => ReferenceKey::Change(ChangeId::from_bytes(stable)),
        4 => ReferenceKey::Release(LineId::from_bytes(stable)),
        5 => ReferenceKey::Marker(stable),
        _ => return Err(StoreError::UnsupportedDatabase),
    })
}

fn loose_relative_path(id: &ObjectId) -> String {
    let digest = hex::encode(id.digest());
    format!(
        "objects/loose/{}/{:02x}/{}/{}.rgl",
        id.format_version(),
        id.algorithm() as u64,
        &digest[..2],
        &digest[2..]
    )
}

fn open_or_create_directory(
    parent: &platform::DirectoryHandle,
    name: &str,
) -> Result<platform::DirectoryHandle, StoreError> {
    let created = platform::create_directory_at(parent, name).map_err(|_| StoreError::Database)?;
    let child = platform::open_directory_at(parent, name).map_err(|_| StoreError::Database)?;
    if created {
        platform::sync_handle(&child).map_err(|_| StoreError::Database)?;
        platform::sync_handle(parent).map_err(|_| StoreError::Database)?;
    }
    Ok(child)
}

#[cfg(unix)]
fn ensure_same_filesystem<'a>(
    root: &platform::DirectoryHandle,
    children: impl IntoIterator<Item = &'a platform::DirectoryHandle>,
) -> Result<(), StoreError> {
    use std::os::unix::fs::MetadataExt;
    let device = root.metadata().map_err(|_| StoreError::Database)?.dev();
    if children
        .into_iter()
        .any(|child| child.metadata().map(|value| value.dev()).ok() != Some(device))
    {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_same_filesystem<'a>(
    _: &platform::DirectoryHandle,
    _: impl IntoIterator<Item = &'a platform::DirectoryHandle>,
) -> Result<(), StoreError> {
    Err(StoreError::UnsupportedDatabase)
}

fn role_name(role: ReferenceRole) -> String {
    let debug = format!("{role:?}");
    let mut result = String::with_capacity(debug.len() + 8);
    for (index, character) in debug.chars().enumerate() {
        if character.is_ascii_uppercase() && index != 0 {
            result.push('_');
        }
        result.push(character.to_ascii_lowercase());
    }
    result
}

fn value_unsigned_field(value: &Value, key: u64) -> Option<u64> {
    let Value::Map(map) = value else { return None };
    let Value::Unsigned(value) = &map.iter().find(|(candidate, _)| *candidate == key)?.1 else {
        return None;
    };
    Some(*value)
}

fn schema_ddl() -> Result<&'static str, StoreError> {
    Ok(include_str!("../migrations/0001_initial.sql"))
}

fn migration_1_digest() -> Result<[u8; 32], StoreError> {
    let actual: [u8; 32] = Sha256::digest(schema_ddl()?.as_bytes()).into();
    if actual != MIGRATION_1_SHA256 {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(actual)
}

fn edge_roles() -> Result<Vec<&'static str>, StoreError> {
    let spec = include_str!("../../../spec/sqlite-store.md");
    let section = spec
        .split("complete version-1 closed registry is:")
        .nth(1)
        .ok_or(StoreError::UnsupportedDatabase)?;
    let block = section
        .split("```text")
        .nth(1)
        .ok_or(StoreError::UnsupportedDatabase)?;
    Ok(block
        .split("```")
        .next()
        .ok_or(StoreError::UnsupportedDatabase)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect())
}

fn pragma_i64(connection: &Connection, name: &str) -> Result<i64, StoreError> {
    connection
        .pragma_query_value(None, name, |row| row.get(0))
        .map_err(map_database_error)
}

fn verify_pragma_i64(connection: &Connection, name: &str, expected: i64) -> Result<(), StoreError> {
    if pragma_i64(connection, name)? != expected {
        return Err(StoreError::UnsupportedDatabase);
    }
    Ok(())
}

fn to_sql_u64(value: u64) -> Result<i64, StoreError> {
    if value > MAX_SQLITE_INTEGER {
        return Err(StoreError::RevisionOverflow);
    }
    Ok(value as i64)
}

fn to_sql_usize(value: usize) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::RevisionOverflow)
}

fn map_database_error(error: rusqlite::Error) -> StoreError {
    if let rusqlite::Error::SqliteFailure(details, _) = &error {
        if matches!(
            details.code,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
        ) {
            return StoreError::RetryableConflict;
        }
    }
    StoreError::Database
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_and_registry_are_present() {
        assert_eq!(
            migration_1_digest().expect("migration digest"),
            MIGRATION_1_SHA256
        );
        assert!(
            schema_ddl()
                .expect("DDL")
                .contains("CREATE TABLE repository")
        );
        assert_eq!(edge_roles().expect("roles").len(), 97);
        assert!(edge_roles().expect("roles").contains(&"operation_parent"));
    }

    #[test]
    fn role_names_match_the_frozen_registry() {
        assert_eq!(
            role_name(ReferenceRole::CiRunnerIdentity),
            "ci_runner_identity"
        );
        assert_eq!(
            role_name(ReferenceRole::OperationParent),
            "operation_parent"
        );
    }
}
