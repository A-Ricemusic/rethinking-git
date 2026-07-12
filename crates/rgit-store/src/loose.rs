use std::{
    fmt,
    fs::{self, File},
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(all(test, unix))]
use std::fs::OpenOptions;

use rgit_objects::{
    AnyObject, BULK_MAX_ENCODED_BYTES, CanonicalLimits, HashAlgorithm, METADATA_MAX_ENCODED_BYTES,
    ObjectId, ObjectKind,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{LooseStoreError, StoredObject, platform};

const MAGIC: &[u8; 8] = b"RGITLOOS";
const CHECKSUM_DOMAIN: &[u8] = b"RGIT-LOOSE-CHECKSUM\0";
const CHECKSUM_LEN: usize = 32;
const ABSOLUTE_RECORD_MAX: u64 = 16_777_358;
const TEMP_ATTEMPTS: usize = 128;

/// Deterministic boundaries used by crash and I/O-failure tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailurePoint {
    AfterEncode,
    AfterTempCreate,
    AfterFrameWrite,
    AfterTempSync,
    AfterTempVerify,
    AfterDirectoryCreate,
    BeforePublish,
    AfterPublish,
    AfterParentSync,
}

/// Injectable publication failure policy. Production uses [`NoFailures`].
pub trait FailureInjector: Send + Sync {
    fn check(&self, point: FailurePoint) -> Result<(), LooseStoreError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoFailures;

impl FailureInjector for NoFailures {
    fn check(&self, _: FailurePoint) -> Result<(), LooseStoreError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PutLooseOutcome {
    New,
    AlreadyPresent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryEntryKind {
    StaleTemporary,
    UnindexedOrphan,
    Quarantined,
    Unexpected,
}

/// A restricted startup-recovery observation. It never includes file contents.
#[derive(Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    pub kind: InventoryEntryKind,
    pub relative_path: PathBuf,
}

impl fmt::Debug for InventoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InventoryEntry")
            .field("kind", &self.kind)
            .field("relative_path", &"<restricted>")
            .finish()
    }
}

/// A bounded, already-verified payload reader.
///
/// Verification completes before construction. The private immutable spool means
/// no byte can be observed before checksum, canonical, ID, kind, and path checks.
pub struct VerifiedReader {
    cursor: Cursor<Arc<[u8]>>,
    object: AnyObject,
}

impl fmt::Debug for VerifiedReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedReader")
            .field("kind", &self.object.decoded().kind())
            .field("length", &self.cursor.get_ref().len())
            .finish_non_exhaustive()
    }
}

impl VerifiedReader {
    #[must_use]
    pub const fn object(&self) -> &AnyObject {
        &self.object
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cursor.get_ref().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Read for VerifiedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.cursor.read(output)
    }
}

impl Seek for VerifiedReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(position)
    }
}

/// Immutable, canonical loose-object storage rooted at a repository control dir.
pub struct LooseObjectStore {
    control: PathBuf,
    loose: PathBuf,
    temporary: PathBuf,
    quarantine: PathBuf,
    _control_handle: platform::DirectoryHandle,
    _objects_handle: platform::DirectoryHandle,
    loose_handle: platform::DirectoryHandle,
    temporary_handle: platform::DirectoryHandle,
    quarantine_handle: platform::DirectoryHandle,
    injector: Arc<dyn FailureInjector>,
    incident: AtomicBool,
}

impl fmt::Debug for LooseObjectStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LooseObjectStore")
            .field("root", &"<redacted>")
            .field("incident", &self.incident.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl LooseObjectStore {
    /// Opens or initializes the loose layout below an existing repository control
    /// directory. Every created component is owner-only and durably synced.
    pub fn open(control: impl AsRef<Path>) -> Result<Self, LooseStoreError> {
        Self::open_with_injector(control, Arc::new(NoFailures))
    }

    pub fn open_with_injector(
        control: impl AsRef<Path>,
        injector: Arc<dyn FailureInjector>,
    ) -> Result<Self, LooseStoreError> {
        let control = control.as_ref().to_path_buf();
        ensure_directory(&control, "create control directory")?;
        reject_link(&control)?;
        let control_handle = platform::open_directory(&control)
            .map_err(|error| LooseStoreError::io("pin repository control directory", error))?;
        let objects = control.join("objects");
        let loose = objects.join("loose");
        let temporary = objects.join("tmp");
        let quarantine = objects.join("quarantine");
        let objects_handle = open_or_create_directory(&control_handle, "objects")?;
        let loose_handle = open_or_create_directory(&objects_handle, "loose")?;
        let temporary_handle = open_or_create_directory(&objects_handle, "tmp")?;
        let quarantine_handle = open_or_create_directory(&objects_handle, "quarantine")?;
        ensure_handle_filesystems(
            &control_handle,
            [
                &objects_handle,
                &loose_handle,
                &temporary_handle,
                &quarantine_handle,
            ],
        )?;
        probe_publication_capabilities(&temporary_handle)?;
        let incident = marker_may_exist(&quarantine_handle) || marker_may_exist(&temporary_handle);
        Ok(Self {
            control,
            loose,
            temporary,
            quarantine,
            _control_handle: control_handle,
            _objects_handle: objects_handle,
            loose_handle,
            temporary_handle,
            quarantine_handle,
            injector,
            incident: AtomicBool::new(incident),
        })
    }

    /// Canonically verifies and durably publishes one immutable logical object.
    pub fn put_canonical(
        &self,
        payload: &[u8],
        algorithm: HashAlgorithm,
    ) -> Result<(ObjectId, PutLooseOutcome), LooseStoreError> {
        let object = AnyObject::decode(payload, CanonicalLimits::bulk())
            .map_err(LooseStoreError::InvalidObject)?;
        let id = object
            .id(algorithm)
            .map_err(LooseStoreError::InvalidObject)?;
        let outcome = self.put(&id, payload)?;
        Ok((id, outcome))
    }

    /// Publishes one object while requiring agreement with a caller-computed ID.
    pub fn put(&self, id: &ObjectId, payload: &[u8]) -> Result<PutLooseOutcome, LooseStoreError> {
        if self.incident.load(Ordering::Acquire) {
            return Err(LooseStoreError::ReadOnlyIncident);
        }
        let object = AnyObject::decode_verified(payload, id, CanonicalLimits::bulk())
            .map_err(LooseStoreError::InvalidObject)?;
        let frame = encode_frame(id, &object, payload)?;
        self.injector.check(FailurePoint::AfterEncode)?;

        let (temporary_name, mut temporary_file) = self.create_temporary()?;
        self.injector.check(FailurePoint::AfterTempCreate)?;
        if let Err(error) = write_complete(&mut temporary_file, &frame) {
            return Err(LooseStoreError::io("write temporary record", error));
        }
        self.injector.check(FailurePoint::AfterFrameWrite)?;
        temporary_file
            .sync_all()
            .map_err(|error| LooseStoreError::io("sync temporary record", error))?;
        self.injector.check(FailurePoint::AfterTempSync)?;

        let expected_identity = platform::file_identity(&temporary_file)
            .map_err(|error| LooseStoreError::io("inspect temporary record", error))?;
        verify_open_file(&mut temporary_file, Some(id), None)?;
        self.injector.check(FailurePoint::AfterTempVerify)?;

        let (parent, final_name) = self.create_fanout(id)?;
        self.injector.check(FailurePoint::AfterDirectoryCreate)?;
        self.injector.check(FailurePoint::BeforePublish)?;

        match platform::rename_no_replace_at(
            &self.temporary_handle,
            &temporary_name,
            &parent,
            &final_name,
        ) {
            Ok(()) => {
                self.injector.check(FailurePoint::AfterPublish)?;
                let published = open_regular_at(&parent, &final_name, "open published record")?;
                let identity_matches = platform::same_file(&expected_identity, &published)
                    .map_err(|error| {
                        LooseStoreError::io("verify published record identity", error)
                    })?;
                if !identity_matches {
                    self.incident.store(true, Ordering::Release);
                    let _ = self.persist_incident();
                    return Err(LooseStoreError::CollisionIncident);
                }
                platform::sync_handle(&self.temporary_handle)
                    .map_err(|error| LooseStoreError::io("sync temporary directory", error))?;
                platform::sync_handle(&parent)
                    .map_err(|error| LooseStoreError::io("sync final object directory", error))?;
                self.injector.check(FailurePoint::AfterParentSync)?;
                Ok(PutLooseOutcome::New)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => self.resolve_existing(
                id,
                &frame,
                &temporary_name,
                temporary_file,
                &parent,
                &final_name,
            ),
            Err(error) => Err(LooseStoreError::io(
                "publish object without replacement",
                error,
            )),
        }
    }

    /// Returns verified canonical payload bytes, never physical framing bytes.
    pub fn read(&self, id: &ObjectId) -> Result<VerifiedReader, LooseStoreError> {
        self.read_audited(id)
            .map_err(|_| LooseStoreError::Unavailable)
    }

    /// Detailed physical verification for an already-authorized local auditor.
    pub fn read_audited(&self, id: &ObjectId) -> Result<VerifiedReader, LooseStoreError> {
        let (parent, name) = self.open_fanout(id).map_err(unavailable_io)?;
        let mut file =
            open_regular_at(&parent, &name, "open loose record").map_err(unavailable_io)?;
        let verified = verify_open_file(&mut file, Some(id), None)?;
        Ok(VerifiedReader {
            cursor: Cursor::new(verified.payload.clone()),
            object: verified.object,
        })
    }

    /// Reads an object while also enforcing a typed caller's expected kind.
    pub fn read_typed(
        &self,
        id: &ObjectId,
        expected_kind: ObjectKind,
    ) -> Result<VerifiedReader, LooseStoreError> {
        self.read_typed_audited(id, expected_kind)
            .map_err(|_| LooseStoreError::Unavailable)
    }

    /// Typed detailed verification for an already-authorized local auditor.
    pub fn read_typed_audited(
        &self,
        id: &ObjectId,
        expected_kind: ObjectKind,
    ) -> Result<VerifiedReader, LooseStoreError> {
        let reader = self.read_audited(id)?;
        if reader.object.decoded().kind() != expected_kind {
            return Err(LooseStoreError::MetadataMismatch);
        }
        Ok(reader)
    }

    /// Fully verifies and decodes one object into the shared storage model.
    pub fn read_object(&self, id: &ObjectId) -> Result<StoredObject, LooseStoreError> {
        self.read_object_audited(id)
            .map_err(|_| LooseStoreError::Unavailable)
    }

    /// Detailed decoded-object verification for an authorized local auditor.
    pub fn read_object_audited(&self, id: &ObjectId) -> Result<StoredObject, LooseStoreError> {
        let (parent, name) = self.open_fanout(id).map_err(unavailable_io)?;
        let mut file =
            open_regular_at(&parent, &name, "open loose record").map_err(unavailable_io)?;
        let verified = verify_open_file(&mut file, Some(id), None)?;
        StoredObject::new(id.clone(), verified.payload.to_vec(), verified.object)
            .map_err(LooseStoreError::InvalidObject)
    }

    /// A verified presence test. Corrupt, missing, and quarantined objects are all
    /// false at this authorization-neutral physical boundary.
    #[must_use]
    pub fn contains(&self, id: &ObjectId) -> bool {
        self.read(id).is_ok()
    }

    /// Inventories recovery remnants without deleting, adopting, or repairing any.
    pub fn inventory_audited(&self) -> Result<Vec<InventoryEntry>, LooseStoreError> {
        let mut entries = Vec::new();
        inventory_flat(
            &self.control,
            &self.temporary,
            InventoryEntryKind::StaleTemporary,
            &mut entries,
        )?;
        inventory_flat(
            &self.control,
            &self.quarantine,
            InventoryEntryKind::Quarantined,
            &mut entries,
        )?;
        inventory_loose(&self.control, &self.loose, &mut entries)?;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(entries)
    }

    #[must_use]
    pub fn is_read_only_incident(&self) -> bool {
        self.incident.load(Ordering::Acquire)
    }

    #[cfg(all(test, unix))]
    fn path_for(&self, id: &ObjectId) -> PathBuf {
        let digest = hex::encode(id.digest());
        self.loose
            .join(id.format_version().to_string())
            .join(format!("{:02x}", id.algorithm() as u64))
            .join(&digest[..2])
            .join(format!("{}.rgl", &digest[2..]))
    }

    fn create_temporary(&self) -> Result<(String, File), LooseStoreError> {
        for _ in 0..TEMP_ATTEMPTS {
            let mut random = [0_u8; 16];
            fill_random(&mut random)
                .map_err(|error| LooseStoreError::io("obtain temporary-name entropy", error))?;
            let name = format!("put-{}", hex::encode(random));
            match platform::create_file_at(&self.temporary_handle, &name) {
                Ok(file) => return Ok((name, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(LooseStoreError::io("create temporary record", error)),
            }
        }
        Err(LooseStoreError::io(
            "allocate unique temporary record",
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "temporary namespace exhausted",
            ),
        ))
    }

    fn create_fanout(
        &self,
        id: &ObjectId,
    ) -> Result<(platform::DirectoryHandle, String), LooseStoreError> {
        let (components, name) = path_components(id);
        let mut current = self
            .loose_handle
            .try_clone()
            .map_err(|error| LooseStoreError::io("clone loose directory handle", error))?;
        for component in components {
            let created = platform::create_directory_at(&current, &component)
                .map_err(|error| LooseStoreError::io("create fanout directory", error))?;
            let child = platform::open_directory_at(&current, &component)
                .map_err(|error| LooseStoreError::io("pin fanout directory", error))?;
            if created {
                platform::sync_handle(&child)
                    .map_err(|error| LooseStoreError::io("sync new fanout directory", error))?;
                platform::sync_handle(&current)
                    .map_err(|error| LooseStoreError::io("sync fanout parent", error))?;
            }
            current = child;
        }
        Ok((current, name))
    }

    fn open_fanout(
        &self,
        id: &ObjectId,
    ) -> Result<(platform::DirectoryHandle, String), LooseStoreError> {
        let (components, name) = path_components(id);
        let mut current = self
            .loose_handle
            .try_clone()
            .map_err(|error| LooseStoreError::io("clone loose directory handle", error))?;
        for component in components {
            current = platform::open_directory_at(&current, &component)
                .map_err(|error| LooseStoreError::io("open fanout directory", error))?;
        }
        Ok((current, name))
    }

    fn resolve_existing(
        &self,
        id: &ObjectId,
        candidate: &[u8],
        temporary_name: &str,
        mut temporary_file: File,
        final_parent: &platform::DirectoryHandle,
        final_name: &str,
    ) -> Result<PutLooseOutcome, LooseStoreError> {
        let existing_matches =
            match open_regular_at(final_parent, final_name, "open existing object") {
                Ok(mut file) => {
                    let valid = verify_open_file(&mut file, Some(id), None).is_ok();
                    let mut bytes = Vec::new();
                    let readable = file.seek(SeekFrom::Start(0)).is_ok()
                        && file
                            .take(ABSOLUTE_RECORD_MAX + 1)
                            .read_to_end(&mut bytes)
                            .is_ok();
                    valid && readable && bytes == candidate
                }
                Err(_) => false,
            };
        if existing_matches {
            drop(temporary_file);
            platform::remove_file_at(&self.temporary_handle, temporary_name)
                .map_err(|error| LooseStoreError::io("remove duplicate temporary record", error))?;
            platform::sync_handle(&self.temporary_handle)
                .map_err(|error| LooseStoreError::io("sync temporary directory", error))?;
            platform::sync_handle(final_parent)
                .map_err(|error| LooseStoreError::io("sync existing object directory", error))?;
            return Ok(PutLooseOutcome::AlreadyPresent);
        }

        // Reverify the retained candidate before quarantining it as evidence.
        verify_open_file(&mut temporary_file, Some(id), None)?;
        drop(temporary_file);
        self.incident.store(true, Ordering::Release);
        let _ = self.persist_incident();
        self.quarantine_candidate(temporary_name)?;
        platform::sync_handle(&self.temporary_handle)
            .map_err(|error| LooseStoreError::io("sync temporary directory", error))?;
        platform::sync_handle(&self.quarantine_handle)
            .map_err(|error| LooseStoreError::io("sync quarantine directory", error))?;
        Err(LooseStoreError::CollisionIncident)
    }

    fn quarantine_candidate(&self, temporary_name: &str) -> Result<(), LooseStoreError> {
        for _ in 0..TEMP_ATTEMPTS {
            let mut random = [0_u8; 16];
            fill_random(&mut random)
                .map_err(|error| LooseStoreError::io("obtain quarantine-name entropy", error))?;
            let name = format!("record-{}.rgl", hex::encode(random));
            match platform::rename_no_replace_at(
                &self.temporary_handle,
                temporary_name,
                &self.quarantine_handle,
                &name,
            ) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(LooseStoreError::io("quarantine collision candidate", error));
                }
            }
        }
        Err(LooseStoreError::io(
            "allocate unique quarantine record",
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "quarantine namespace exhausted",
            ),
        ))
    }

    fn persist_incident(&self) -> Result<(), LooseStoreError> {
        if persist_marker(&self.quarantine_handle).is_ok() {
            return Ok(());
        }
        persist_marker(&self.temporary_handle)
    }
}

fn marker_may_exist(directory: &platform::DirectoryHandle) -> bool {
    !matches!(
        platform::open_file_at(directory, "INCIDENT"),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    )
}

fn persist_marker(directory: &platform::DirectoryHandle) -> Result<(), LooseStoreError> {
    match platform::create_file_at(directory, "INCIDENT") {
        Ok(mut marker) => {
            write_complete(&mut marker, b"RGIT-LOOSE-INCIDENT\nversion=0\n")
                .map_err(|error| LooseStoreError::io("write incident marker", error))?;
            marker
                .sync_all()
                .map_err(|error| LooseStoreError::io("sync incident marker", error))?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(LooseStoreError::io("create incident marker", error)),
    }
    platform::sync_handle(directory)
        .map_err(|error| LooseStoreError::io("sync incident marker directory", error))
}

struct VerifiedRecord {
    payload: Arc<[u8]>,
    object: AnyObject,
}

fn encode_frame(
    id: &ObjectId,
    object: &AnyObject,
    payload: &[u8],
) -> Result<Vec<u8>, LooseStoreError> {
    let limit = payload_limit(object.decoded().kind());
    if payload.len() > limit {
        return Err(LooseStoreError::AllocationLimit);
    }
    let mut frame = Vec::with_capacity(8 + 1 + 35 + 10 + 10 + 10 + payload.len() + CHECKSUM_LEN);
    frame.extend_from_slice(MAGIC);
    push_varint(&mut frame, 0);
    frame.extend_from_slice(&id.to_bytes());
    push_varint(&mut frame, object.decoded().kind() as u64);
    push_varint(&mut frame, object.decoded().schema_version());
    push_varint(&mut frame, payload.len() as u64);
    frame.extend_from_slice(payload);
    let mut checksum = Sha256::new();
    checksum.update(CHECKSUM_DOMAIN);
    checksum.update(&frame);
    frame.extend_from_slice(&checksum.finalize());
    Ok(frame)
}

fn path_components(id: &ObjectId) -> ([String; 3], String) {
    let digest = hex::encode(id.digest());
    (
        [
            id.format_version().to_string(),
            format!("{:02x}", id.algorithm() as u64),
            digest[..2].to_owned(),
        ],
        format!("{}.rgl", &digest[2..]),
    )
}

fn probe_publication_capabilities(
    directory: &platform::DirectoryHandle,
) -> Result<(), LooseStoreError> {
    let mut random = [0_u8; 16];
    fill_random(&mut random)
        .map_err(|error| LooseStoreError::io("obtain capability-probe entropy", error))?;
    let stem = hex::encode(random);
    let source = format!("probe-{stem}-source");
    let occupied = format!("probe-{stem}-occupied");
    let published = format!("probe-{stem}-published");
    let mut source_file = platform::create_file_at(directory, &source)
        .map_err(|error| LooseStoreError::io("create capability probe", error))?;
    write_complete(&mut source_file, b"probe")
        .map_err(|error| LooseStoreError::io("write capability probe", error))?;
    source_file
        .sync_all()
        .map_err(|error| LooseStoreError::io("sync capability probe", error))?;
    drop(source_file);
    let occupied_file = platform::create_file_at(directory, &occupied)
        .map_err(|error| LooseStoreError::io("create occupied probe target", error))?;
    occupied_file
        .sync_all()
        .map_err(|error| LooseStoreError::io("sync occupied probe target", error))?;
    drop(occupied_file);
    let collision = platform::rename_no_replace_at(directory, &source, directory, &occupied);
    if !collision.is_err_and(|error| error.kind() == io::ErrorKind::AlreadyExists) {
        return Err(LooseStoreError::UnsupportedPlatform);
    }
    platform::rename_no_replace_at(directory, &source, directory, &published)
        .map_err(|_| LooseStoreError::UnsupportedPlatform)?;
    platform::sync_handle(directory).map_err(|_| LooseStoreError::UnsupportedPlatform)?;
    for name in [&occupied, &published] {
        platform::remove_file_at(directory, name)
            .map_err(|error| LooseStoreError::io("remove capability probe", error))?;
    }
    platform::sync_handle(directory).map_err(|_| LooseStoreError::UnsupportedPlatform)
}

fn verify_open_file(
    file: &mut File,
    expected_id: Option<&ObjectId>,
    actual_path: Option<&Path>,
) -> Result<VerifiedRecord, LooseStoreError> {
    let metadata = file
        .metadata()
        .map_err(|error| LooseStoreError::io("inspect loose record", error))?;
    if !metadata.file_type().is_file() || metadata.len() > ABSOLUTE_RECORD_MAX {
        return Err(LooseStoreError::AllocationLimit);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(LooseStoreError::PathMismatch);
        }
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| LooseStoreError::io("rewind loose record", error))?;
    parse_record_stream(file, metadata.len(), expected_id, actual_path)
}

fn parse_record_stream(
    reader: &mut impl Read,
    observed_length: u64,
    expected_id: Option<&ObjectId>,
    actual_path: Option<&Path>,
) -> Result<VerifiedRecord, LooseStoreError> {
    let mut checksum = Sha256::new();
    checksum.update(CHECKSUM_DOMAIN);
    let mut consumed = 0_u64;
    let mut magic = [0_u8; 8];
    read_hashed(reader, &mut magic, &mut checksum, &mut consumed)?;
    if &magic != MAGIC {
        return Err(LooseStoreError::InvalidFrame);
    }
    if read_varint_stream(reader, &mut checksum, &mut consumed)? != 0 {
        return Err(LooseStoreError::UnsupportedFormat);
    }
    let format = read_varint_stream(reader, &mut checksum, &mut consumed)?;
    let algorithm = read_varint_stream(reader, &mut checksum, &mut consumed)?;
    let digest_length = read_varint_stream(reader, &mut checksum, &mut consumed)?;
    if format != 0 || !matches!(algorithm, 0x12 | 0x1e) || digest_length != 32 {
        return Err(LooseStoreError::UnsupportedFormat);
    }
    let mut digest = [0_u8; 32];
    read_hashed(reader, &mut digest, &mut checksum, &mut consumed)?;
    let mut id_bytes = Vec::with_capacity(35);
    push_varint(&mut id_bytes, format);
    push_varint(&mut id_bytes, algorithm);
    push_varint(&mut id_bytes, digest_length);
    id_bytes.extend_from_slice(&digest);
    let id = ObjectId::from_bytes(&id_bytes).map_err(|_| LooseStoreError::InvalidFrame)?;
    let kind_number = read_varint_stream(reader, &mut checksum, &mut consumed)?;
    let kind = ObjectKind::try_from(kind_number).map_err(|_| LooseStoreError::UnsupportedFormat)?;
    let schema = read_varint_stream(reader, &mut checksum, &mut consumed)?;
    if schema != 0 {
        return Err(LooseStoreError::UnsupportedFormat);
    }
    let payload_length_u64 = read_varint_stream(reader, &mut checksum, &mut consumed)?;
    if payload_length_u64 > payload_limit(kind) as u64 {
        return Err(LooseStoreError::AllocationLimit);
    }
    let expected_length = consumed
        .checked_add(payload_length_u64)
        .and_then(|length| length.checked_add(CHECKSUM_LEN as u64))
        .ok_or(LooseStoreError::InvalidFrame)?;
    if expected_length != observed_length {
        return Err(LooseStoreError::InvalidFrame);
    }
    let payload_length =
        usize::try_from(payload_length_u64).map_err(|_| LooseStoreError::AllocationLimit)?;
    let mut payload = vec![0_u8; payload_length];
    read_hashed(reader, &mut payload, &mut checksum, &mut consumed)?;
    let mut physical_checksum = [0_u8; CHECKSUM_LEN];
    reader
        .read_exact(&mut physical_checksum)
        .map_err(|_| LooseStoreError::InvalidFrame)?;
    if checksum
        .finalize()
        .as_slice()
        .ct_eq(&physical_checksum)
        .unwrap_u8()
        != 1
    {
        return Err(LooseStoreError::Checksum);
    }
    validate_logical_record(id, kind, schema, &payload, expected_id, actual_path)
}

fn read_hashed(
    reader: &mut impl Read,
    output: &mut [u8],
    checksum: &mut Sha256,
    consumed: &mut u64,
) -> Result<(), LooseStoreError> {
    reader
        .read_exact(output)
        .map_err(|_| LooseStoreError::InvalidFrame)?;
    checksum.update(&*output);
    *consumed = consumed
        .checked_add(output.len() as u64)
        .ok_or(LooseStoreError::InvalidFrame)?;
    Ok(())
}

fn read_varint_stream(
    reader: &mut impl Read,
    checksum: &mut Sha256,
    consumed: &mut u64,
) -> Result<u64, LooseStoreError> {
    let mut encoded = [0_u8; 10];
    for index in 0..encoded.len() {
        read_hashed(reader, &mut encoded[index..=index], checksum, consumed)?;
        let byte = encoded[index];
        if index == 9 && byte > 1 {
            return Err(LooseStoreError::InvalidVarint);
        }
        if byte & 0x80 == 0 {
            let mut at = 0;
            let value = read_varint(&encoded[..=index], &mut at)?;
            if at != index + 1 {
                return Err(LooseStoreError::InvalidVarint);
            }
            return Ok(value);
        }
    }
    Err(LooseStoreError::InvalidVarint)
}

fn validate_logical_record(
    id: ObjectId,
    kind: ObjectKind,
    schema: u64,
    payload: &[u8],
    expected_id: Option<&ObjectId>,
    actual_path: Option<&Path>,
) -> Result<VerifiedRecord, LooseStoreError> {
    let object = AnyObject::decode_verified(payload, &id, limits_for(kind))
        .map_err(LooseStoreError::InvalidObject)?;
    if object.decoded().kind() != kind || object.decoded().schema_version() != schema {
        return Err(LooseStoreError::MetadataMismatch);
    }
    if let Some(expected) = expected_id {
        if expected != &id {
            return Err(LooseStoreError::MetadataMismatch);
        }
    }
    if let Some(path) = actual_path {
        let digest_hex = hex::encode(id.digest());
        let suffix = format!("{}.rgl", &digest_hex[2..]);
        let components = [
            id.format_version().to_string(),
            format!("{:02x}", id.algorithm() as u64),
            digest_hex[..2].to_owned(),
            suffix,
        ];
        let tail: Vec<_> = path.components().rev().take(4).collect();
        if tail.len() != 4
            || tail[0].as_os_str() != components[3].as_str()
            || tail[1].as_os_str() != components[2].as_str()
            || tail[2].as_os_str() != components[1].as_str()
            || tail[3].as_os_str() != components[0].as_str()
        {
            return Err(LooseStoreError::PathMismatch);
        }
    }
    Ok(VerifiedRecord {
        payload: Arc::from(payload),
        object,
    })
}

fn payload_limit(kind: ObjectKind) -> usize {
    match kind {
        ObjectKind::Chunk | ObjectKind::Blob => BULK_MAX_ENCODED_BYTES,
        _ => METADATA_MAX_ENCODED_BYTES,
    }
}

fn limits_for(kind: ObjectKind) -> CanonicalLimits {
    match kind {
        ObjectKind::Chunk | ObjectKind::Blob => CanonicalLimits::bulk(),
        _ => CanonicalLimits::metadata(),
    }
}

fn push_varint(output: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(input: &[u8], at: &mut usize) -> Result<u64, LooseStoreError> {
    let start = *at;
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = *input.get(*at).ok_or(LooseStoreError::InvalidVarint)?;
        *at += 1;
        if shift == 63 && byte > 1 {
            return Err(LooseStoreError::InvalidVarint);
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            let mut canonical = Vec::new();
            push_varint(&mut canonical, value);
            if canonical.len() != *at - start {
                return Err(LooseStoreError::InvalidVarint);
            }
            return Ok(value);
        }
    }
    Err(LooseStoreError::InvalidVarint)
}

fn write_complete(writer: &mut impl Write, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        match writer.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "zero-progress write",
                ));
            }
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    writer.flush()
}

fn ensure_directory(path: &Path, operation: &'static str) -> Result<bool, LooseStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(false)
        }
        Ok(_) => Err(LooseStoreError::PathMismatch),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => Ok(true),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    reject_link(path)?;
                    Ok(false)
                }
                Err(error) => Err(LooseStoreError::io(operation, error)),
            }
        }
        Err(error) => Err(LooseStoreError::io(operation, error)),
    }
}

fn open_or_create_directory(
    parent: &platform::DirectoryHandle,
    name: &str,
) -> Result<platform::DirectoryHandle, LooseStoreError> {
    let created = platform::create_directory_at(parent, name)
        .map_err(|error| LooseStoreError::io("create repository directory", error))?;
    let child = platform::open_directory_at(parent, name)
        .map_err(|error| LooseStoreError::io("pin repository directory", error))?;
    if created {
        platform::sync_handle(&child)
            .map_err(|error| LooseStoreError::io("sync new repository directory", error))?;
        platform::sync_handle(parent)
            .map_err(|error| LooseStoreError::io("sync repository directory parent", error))?;
    }
    Ok(child)
}

#[cfg(unix)]
fn ensure_handle_filesystems<'a>(
    root: &platform::DirectoryHandle,
    children: impl IntoIterator<Item = &'a platform::DirectoryHandle>,
) -> Result<(), LooseStoreError> {
    use std::os::unix::fs::MetadataExt;
    let device = root
        .metadata()
        .map_err(|error| LooseStoreError::io("inspect repository filesystem", error))?
        .dev();
    for child in children {
        if child
            .metadata()
            .map_err(|error| LooseStoreError::io("inspect object filesystem", error))?
            .dev()
            != device
        {
            return Err(LooseStoreError::UnsupportedPlatform);
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_handle_filesystems<'a>(
    _: &platform::DirectoryHandle,
    _: impl IntoIterator<Item = &'a platform::DirectoryHandle>,
) -> Result<(), LooseStoreError> {
    Err(LooseStoreError::UnsupportedPlatform)
}

fn reject_link(path: &Path) -> Result<(), LooseStoreError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| LooseStoreError::io("inspect control path", error))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        Err(LooseStoreError::PathMismatch)
    } else {
        Ok(())
    }
}

#[cfg(all(test, unix))]
fn open_regular_nofollow(path: &Path, operation: &'static str) -> Result<File, LooseStoreError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(o_nofollow());
    }
    let file = options
        .open(path)
        .map_err(|error| LooseStoreError::io(operation, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| LooseStoreError::io(operation, error))?;
    if !metadata.file_type().is_file() {
        return Err(LooseStoreError::PathMismatch);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(LooseStoreError::PathMismatch);
        }
    }
    Ok(file)
}

fn open_regular_at(
    parent: &platform::DirectoryHandle,
    name: &str,
    operation: &'static str,
) -> Result<File, LooseStoreError> {
    let file = platform::open_file_at(parent, name)
        .map_err(|error| LooseStoreError::io(operation, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| LooseStoreError::io(operation, error))?;
    if !metadata.file_type().is_file() {
        return Err(LooseStoreError::PathMismatch);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(LooseStoreError::PathMismatch);
        }
    }
    Ok(file)
}

fn inventory_flat(
    root: &Path,
    directory: &Path,
    kind: InventoryEntryKind,
    output: &mut Vec<InventoryEntry>,
) -> Result<(), LooseStoreError> {
    for entry in fs::read_dir(directory)
        .map_err(|error| LooseStoreError::io("inventory object directory", error))?
    {
        let entry = entry.map_err(|error| LooseStoreError::io("inventory object entry", error))?;
        output.push(InventoryEntry {
            kind,
            relative_path: entry
                .path()
                .strip_prefix(root)
                .map_err(|_| LooseStoreError::PathMismatch)?
                .to_path_buf(),
        });
    }
    Ok(())
}

fn inventory_loose(
    root: &Path,
    directory: &Path,
    output: &mut Vec<InventoryEntry>,
) -> Result<(), LooseStoreError> {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current)
            .map_err(|error| LooseStoreError::io("inventory loose directory", error))?
        {
            let entry =
                entry.map_err(|error| LooseStoreError::io("inventory loose entry", error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| LooseStoreError::io("inspect loose entry", error))?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else {
                output.push(InventoryEntry {
                    kind: if file_type.is_file() {
                        InventoryEntryKind::UnindexedOrphan
                    } else {
                        InventoryEntryKind::Unexpected
                    },
                    relative_path: entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| LooseStoreError::PathMismatch)?
                        .to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

fn unavailable_io(_: LooseStoreError) -> LooseStoreError {
    LooseStoreError::Unavailable
}

#[cfg(all(test, target_os = "macos"))]
const fn o_nofollow() -> i32 {
    0x0000_0100
}

#[cfg(all(test, unix, not(target_os = "macos")))]
const fn o_nofollow() -> i32 {
    0x0002_0000
}

#[cfg(unix)]
fn fill_random(output: &mut [u8]) -> io::Result<()> {
    File::open("/dev/urandom")?.read_exact(output)
}

#[cfg(windows)]
fn fill_random(output: &mut [u8]) -> io::Result<()> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            output.as_mut_ptr(),
            output.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other("operating-system CSPRNG failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgit_objects::HashAlgorithm;
    #[cfg(unix)]
    use std::{
        collections::BTreeSet,
        process::Command,
        sync::{Barrier, Mutex},
        thread,
    };

    const VECTOR: &str = "524749544c4f4f5300001e20fcb8cf563145b1628a69e25e3a775d0d11cf3ae40cab63cc3bf94af1bfcbb166010044a40001010002a20050000000000000000000000000000000000158230012200000000000000000000000000000000000000000000000000000000000000000034361626306967a7dc69492425c9d81d219dd62ab37434c8e3690f00686fd6d7c47059e06";

    #[cfg(unix)]
    struct TestDirectory(PathBuf);

    #[cfg(unix)]
    impl TestDirectory {
        fn new() -> Self {
            let mut random = [0_u8; 16];
            fill_random(&mut random).unwrap();
            let path =
                std::env::temp_dir().join(format!("rgit-loose-test-{}", hex::encode(random)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    #[cfg(unix)]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn vector_parts() -> (ObjectId, Vec<u8>, AnyObject) {
        let bytes = hex::decode(VECTOR).unwrap();
        let verified = parse_test_record(&bytes).unwrap();
        let id = verified.object.id(HashAlgorithm::Blake3_256).unwrap();
        (id, verified.payload.to_vec(), verified.object)
    }

    fn repair_checksum(bytes: &mut [u8]) {
        let split = bytes.len() - CHECKSUM_LEN;
        let mut checksum = Sha256::new();
        checksum.update(CHECKSUM_DOMAIN);
        checksum.update(&bytes[..split]);
        bytes[split..].copy_from_slice(&checksum.finalize());
    }

    fn parse_test_record(bytes: &[u8]) -> Result<VerifiedRecord, LooseStoreError> {
        parse_record_stream(&mut Cursor::new(bytes), bytes.len() as u64, None, None)
    }

    #[test]
    fn committed_vector_decodes_and_reencodes_exactly() {
        let bytes = hex::decode(VECTOR).unwrap();
        let verified = parse_test_record(&bytes).unwrap();
        let id = verified.object.id(HashAlgorithm::Blake3_256).unwrap();
        assert_eq!(
            encode_frame(&id, &verified.object, &verified.payload).unwrap(),
            bytes
        );
    }

    struct FragmentedReader {
        bytes: Cursor<Vec<u8>>,
        maximum: usize,
        interrupt_next: bool,
    }

    impl Read for FragmentedReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.interrupt_next {
                self.interrupt_next = false;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            self.interrupt_next = true;
            let maximum = output.len().min(self.maximum);
            self.bytes.read(&mut output[..maximum])
        }
    }

    #[test]
    fn streaming_parser_handles_fragmentation_and_eintr_at_every_byte() {
        let bytes = hex::decode(VECTOR).unwrap();
        for maximum in 1..=bytes.len() {
            let mut reader = FragmentedReader {
                bytes: Cursor::new(bytes.clone()),
                maximum,
                interrupt_next: maximum % 2 == 0,
            };
            let verified =
                parse_record_stream(&mut reader, bytes.len() as u64, None, None).unwrap();
            assert_eq!(verified.payload.len(), 68);
        }
    }

    #[test]
    fn rejects_nonminimal_varint_and_corruption() {
        let original = hex::decode(VECTOR).unwrap();
        let mut nonminimal = original.clone();
        nonminimal.splice(8..9, [0x80, 0]);
        assert!(matches!(
            parse_test_record(&nonminimal),
            Err(LooseStoreError::InvalidVarint)
        ));
        let mut checksum = original;
        *checksum.last_mut().unwrap() ^= 1;
        assert!(matches!(
            parse_test_record(&checksum),
            Err(LooseStoreError::Checksum)
        ));
    }

    #[test]
    fn one_variable_frame_mutations_fail_closed() {
        let original = hex::decode(VECTOR).unwrap();
        for (offset, value, repair) in [
            (0, b'X', false),
            (8, 1, false),
            (9, 1, false),
            (10, 0x13, false),
            (11, 31, false),
            (11, 33, false),
            (12, original[12] ^ 1, true),
            (44, 2, true),
            (45, 1, true),
            (46, 67, false),
            (46, 69, false),
            (47, 0xa5, true),
            (114, original[114] ^ 1, true),
            (115, original[115] ^ 1, false),
        ] {
            let mut changed = original.clone();
            changed[offset] = value;
            if repair {
                repair_checksum(&mut changed);
            }
            assert!(
                parse_test_record(&changed).is_err(),
                "accepted mutation at {offset}"
            );
        }
        let mut trailing = original;
        trailing.push(0);
        assert!(parse_test_record(&trailing).is_err());
    }

    #[test]
    fn rejects_overflow_unterminated_and_oversized_lengths() {
        let original = hex::decode(VECTOR).unwrap();
        for encoded in [
            vec![0x80; 11],
            vec![0xff; 9].into_iter().chain([2]).collect(),
        ] {
            let mut changed = original.clone();
            changed.splice(46..47, encoded);
            assert!(matches!(
                parse_test_record(&changed),
                Err(LooseStoreError::InvalidVarint)
            ));
        }
        let mut oversized = original;
        let mut encoded = Vec::new();
        push_varint(&mut encoded, (BULK_MAX_ENCODED_BYTES as u64) + 1);
        oversized.splice(46..47, encoded);
        assert!(matches!(
            parse_test_record(&oversized),
            Err(LooseStoreError::AllocationLimit)
        ));
    }

    #[test]
    fn every_truncation_is_rejected() {
        let bytes = hex::decode(VECTOR).unwrap();
        for length in 0..bytes.len() {
            assert!(
                parse_test_record(&bytes[..length]).is_err(),
                "accepted {length}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_filesystem_round_trip_dedup_reopen_and_inventory() {
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let (id, payload, _) = vector_parts();
        let (computed, outcome) = store
            .put_canonical(&payload, HashAlgorithm::Blake3_256)
            .unwrap();
        assert_eq!(computed, id);
        assert_eq!(outcome, PutLooseOutcome::New);
        assert_eq!(
            store.put(&id, &payload).unwrap(),
            PutLooseOutcome::AlreadyPresent
        );
        assert!(store.contains(&id));
        let mut reader = store.read(&id).unwrap();
        let mut observed = Vec::new();
        reader.read_to_end(&mut observed).unwrap();
        assert_eq!(observed, payload);
        assert_eq!(reader.object().decoded().kind(), ObjectKind::Chunk);
        assert!(store.read_typed(&id, ObjectKind::Chunk).is_ok());
        assert!(matches!(
            store.read_typed(&id, ObjectKind::Blob),
            Err(LooseStoreError::Unavailable)
        ));
        assert!(matches!(
            store.read_typed_audited(&id, ObjectKind::Blob),
            Err(LooseStoreError::MetadataMismatch)
        ));
        drop(store);
        let reopened = LooseObjectStore::open(&directory.0).unwrap();
        assert!(reopened.contains(&id));
        assert!(
            reopened
                .inventory_audited()
                .unwrap()
                .iter()
                .any(|entry| entry.kind == InventoryEntryKind::UnindexedOrphan)
        );
    }

    #[cfg(unix)]
    #[test]
    fn derived_path_and_permissions_match_contract() {
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let (id, payload, _) = vector_parts();
        store.put(&id, &payload).unwrap();
        assert!(store.path_for(&id).ends_with(
            "0/1e/fc/b8cf563145b1628a69e25e3a775d0d11cf3ae40cab63cc3bf94af1bfcbb166.rgl"
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.path_for(&id))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&store.temporary).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn wrong_path_and_symlink_are_never_followed() {
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let (id, payload, object) = vector_parts();
        let frame = encode_frame(&id, &object, &payload).unwrap();
        let wrong = store.loose.join("0/1e/00/wrong.rgl");
        fs::create_dir_all(wrong.parent().unwrap()).unwrap();
        fs::write(&wrong, &frame).unwrap();
        let mut file = open_regular_nofollow(&wrong, "test").unwrap();
        assert!(matches!(
            verify_open_file(&mut file, Some(&id), Some(&wrong)),
            Err(LooseStoreError::PathMismatch)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = directory.0.join("record-link");
            symlink(&wrong, &link).unwrap();
            assert!(open_regular_nofollow(&link, "test").is_err());
        }
    }

    #[cfg(unix)]
    #[derive(Debug)]
    struct FailOnce(FailurePoint);

    #[cfg(unix)]
    impl FailureInjector for FailOnce {
        fn check(&self, point: FailurePoint) -> Result<(), LooseStoreError> {
            if point == self.0 {
                Err(LooseStoreError::InjectedFailure)
            } else {
                Ok(())
            }
        }
    }

    #[cfg(unix)]
    #[derive(Debug)]
    struct ExitAt(FailurePoint);

    #[cfg(unix)]
    impl FailureInjector for ExitAt {
        fn check(&self, point: FailurePoint) -> Result<(), LooseStoreError> {
            if point == self.0 {
                std::process::exit(91);
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    fn point_name(point: FailurePoint) -> &'static str {
        match point {
            FailurePoint::AfterEncode => "after-encode",
            FailurePoint::AfterTempCreate => "after-temp-create",
            FailurePoint::AfterFrameWrite => "after-frame-write",
            FailurePoint::AfterTempSync => "after-temp-sync",
            FailurePoint::AfterTempVerify => "after-temp-verify",
            FailurePoint::AfterDirectoryCreate => "after-directory-create",
            FailurePoint::BeforePublish => "before-publish",
            FailurePoint::AfterPublish => "after-publish",
            FailurePoint::AfterParentSync => "after-parent-sync",
        }
    }

    #[cfg(unix)]
    fn parse_point(name: &str) -> FailurePoint {
        [
            FailurePoint::AfterEncode,
            FailurePoint::AfterTempCreate,
            FailurePoint::AfterFrameWrite,
            FailurePoint::AfterTempSync,
            FailurePoint::AfterTempVerify,
            FailurePoint::AfterDirectoryCreate,
            FailurePoint::BeforePublish,
            FailurePoint::AfterPublish,
            FailurePoint::AfterParentSync,
        ]
        .into_iter()
        .find(|point| point_name(*point) == name)
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess crash helper"]
    fn crash_child() {
        let control = std::env::var_os("RGIT_CRASH_CONTROL").unwrap();
        let point = parse_point(&std::env::var("RGIT_CRASH_POINT").unwrap());
        let store = LooseObjectStore::open_with_injector(control, Arc::new(ExitAt(point))).unwrap();
        let (id, payload, _) = vector_parts();
        let _ = store.put(&id, &payload);
        panic!("failure point was not reached");
    }

    #[cfg(unix)]
    #[test]
    fn process_termination_at_every_boundary_has_recoverable_layout() {
        let points = [
            FailurePoint::AfterEncode,
            FailurePoint::AfterTempCreate,
            FailurePoint::AfterFrameWrite,
            FailurePoint::AfterTempSync,
            FailurePoint::AfterTempVerify,
            FailurePoint::AfterDirectoryCreate,
            FailurePoint::BeforePublish,
            FailurePoint::AfterPublish,
            FailurePoint::AfterParentSync,
        ];
        for point in points {
            let directory = TestDirectory::new();
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact", "loose::tests::crash_child"])
                .env("RGIT_CRASH_CONTROL", &directory.0)
                .env("RGIT_CRASH_POINT", point_name(point))
                .status()
                .unwrap();
            assert_eq!(
                status.code(),
                Some(91),
                "child did not terminate at {point:?}"
            );
            let reopened = LooseObjectStore::open(&directory.0).unwrap();
            let (id, _, _) = vector_parts();
            let published = reopened.path_for(&id).exists();
            assert_eq!(
                published,
                matches!(
                    point,
                    FailurePoint::AfterPublish | FailurePoint::AfterParentSync
                ),
                "wrong publication boundary for {point:?}"
            );
            let first = reopened.inventory_audited().unwrap();
            let second = reopened.inventory_audited().unwrap();
            assert_eq!(
                first, second,
                "inventory was not idempotent after {point:?}"
            );
            let temporary_count = first
                .iter()
                .filter(|entry| entry.kind == InventoryEntryKind::StaleTemporary)
                .count();
            let orphan_count = first
                .iter()
                .filter(|entry| entry.kind == InventoryEntryKind::UnindexedOrphan)
                .count();
            if matches!(
                point,
                FailurePoint::AfterPublish | FailurePoint::AfterParentSync
            ) {
                assert_eq!((temporary_count, orphan_count), (0, 1));
            } else if point == FailurePoint::AfterEncode {
                assert_eq!((temporary_count, orphan_count), (0, 0));
            } else {
                assert_eq!((temporary_count, orphan_count), (1, 0));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_permission_loss_fails_before_publication() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let temporary = directory.0.join("objects/tmp");
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o500)).unwrap();
        let (id, payload, _) = vector_parts();
        let result = store.put(&id, &payload);
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(result, Err(LooseStoreError::Io { .. })));
        assert!(!store.path_for(&id).exists());
    }

    #[cfg(unix)]
    #[test]
    fn prepublication_failpoints_never_expose_final_object() {
        let points = [
            FailurePoint::AfterEncode,
            FailurePoint::AfterTempCreate,
            FailurePoint::AfterFrameWrite,
            FailurePoint::AfterTempSync,
            FailurePoint::AfterTempVerify,
            FailurePoint::AfterDirectoryCreate,
            FailurePoint::BeforePublish,
        ];
        for point in points {
            let directory = TestDirectory::new();
            let store =
                LooseObjectStore::open_with_injector(&directory.0, Arc::new(FailOnce(point)))
                    .unwrap();
            let (id, payload, _) = vector_parts();
            assert!(matches!(
                store.put(&id, &payload),
                Err(LooseStoreError::InjectedFailure)
            ));
            assert!(
                !store.path_for(&id).exists(),
                "{point:?} published a final path"
            );
            assert!(!store.contains(&id));
        }
    }

    #[cfg(unix)]
    #[test]
    fn postrename_failpoints_are_reported_as_inventory_orphans() {
        for point in [FailurePoint::AfterPublish, FailurePoint::AfterParentSync] {
            let directory = TestDirectory::new();
            let store =
                LooseObjectStore::open_with_injector(&directory.0, Arc::new(FailOnce(point)))
                    .unwrap();
            let (id, payload, _) = vector_parts();
            assert!(matches!(
                store.put(&id, &payload),
                Err(LooseStoreError::InjectedFailure)
            ));
            assert!(store.path_for(&id).exists());
            assert!(
                store
                    .inventory_audited()
                    .unwrap()
                    .iter()
                    .any(|entry| entry.kind == InventoryEntryKind::UnindexedOrphan)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn collision_quarantines_candidate_and_enters_incident_mode() {
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let (id, payload, _) = vector_parts();
        store.put(&id, &payload).unwrap();
        let final_path = store.path_for(&id);
        let mut corrupt = fs::read(&final_path).unwrap();
        *corrupt.last_mut().unwrap() ^= 1;
        fs::write(&final_path, corrupt).unwrap();
        assert!(matches!(
            store.put(&id, &payload),
            Err(LooseStoreError::CollisionIncident)
        ));
        assert!(store.is_read_only_incident());
        assert!(matches!(
            store.put(&id, &payload),
            Err(LooseStoreError::ReadOnlyIncident)
        ));
        assert!(
            store
                .inventory_audited()
                .unwrap()
                .iter()
                .any(|entry| entry.kind == InventoryEntryKind::Quarantined)
        );
        drop(store);
        let reopened = LooseObjectStore::open(&directory.0).unwrap();
        assert!(reopened.is_read_only_incident());
        assert!(matches!(
            reopened.put(&id, &payload),
            Err(LooseStoreError::ReadOnlyIncident)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_reads_collapse_corruption_while_audited_reads_retain_detail() {
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let (id, payload, _) = vector_parts();
        store.put(&id, &payload).unwrap();
        let path = store.path_for(&id);
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(path, bytes).unwrap();
        assert!(matches!(store.read(&id), Err(LooseStoreError::Unavailable)));
        assert!(matches!(
            store.read_typed(&id, ObjectKind::Chunk),
            Err(LooseStoreError::Unavailable)
        ));
        assert!(matches!(
            store.read_object(&id),
            Err(LooseStoreError::Unavailable)
        ));
        assert!(matches!(
            store.read_audited(&id),
            Err(LooseStoreError::Checksum)
        ));
        assert!(matches!(
            store.read_typed_audited(&id, ObjectKind::Chunk),
            Err(LooseStoreError::Checksum)
        ));
        assert!(matches!(
            store.read_object_audited(&id),
            Err(LooseStoreError::Checksum)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn pinned_loose_directory_defeats_hostile_path_swap() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let original = directory.0.join("objects/loose");
        let pinned = directory.0.join("objects/pinned-loose");
        let outside = directory.0.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::rename(&original, &pinned).unwrap();
        symlink(&outside, &original).unwrap();
        let (id, payload, _) = vector_parts();
        store.put(&id, &payload).unwrap();
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        let digest = hex::encode(id.digest());
        assert!(
            pinned
                .join(format!("0/1e/{}/{}.rgl", &digest[..2], &digest[2..]))
                .is_file()
        );
    }

    #[cfg(unix)]
    #[test]
    fn startup_rejects_symlinked_object_directory() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new();
        let outside = directory.0.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, directory.0.join("objects")).unwrap();
        assert!(LooseObjectStore::open(&directory.0).is_err());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn pinned_startup_root_defeats_root_path_swap() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new();
        let root = directory.0.join("control");
        let pinned = directory.0.join("pinned-control");
        let outside = directory.0.join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        let handle = platform::open_directory(&root).unwrap();
        fs::rename(&root, &pinned).unwrap();
        symlink(&outside, &root).unwrap();
        let _objects = open_or_create_directory(&handle, "objects").unwrap();
        assert!(pinned.join("objects").is_dir());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[derive(Debug)]
    struct SubstitutePublished {
        final_path: PathBuf,
    }

    #[cfg(unix)]
    impl FailureInjector for SubstitutePublished {
        fn check(&self, point: FailurePoint) -> Result<(), LooseStoreError> {
            if point == FailurePoint::AfterPublish {
                fs::rename(&self.final_path, self.final_path.with_extension("replaced")).unwrap();
                fs::write(&self.final_path, b"hostile substitute").unwrap();
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    #[test]
    fn postrename_substitution_persists_incident_across_restart() {
        let directory = TestDirectory::new();
        let (id, payload, _) = vector_parts();
        let bootstrap = LooseObjectStore::open(&directory.0).unwrap();
        let final_path = bootstrap.path_for(&id);
        drop(bootstrap);
        let store = LooseObjectStore::open_with_injector(
            &directory.0,
            Arc::new(SubstitutePublished { final_path }),
        )
        .unwrap();
        assert!(matches!(
            store.put(&id, &payload),
            Err(LooseStoreError::CollisionIncident)
        ));
        assert!(store.is_read_only_incident());
        drop(store);
        let reopened = LooseObjectStore::open(&directory.0).unwrap();
        assert!(reopened.is_read_only_incident());
        assert!(matches!(
            reopened.put(&id, &payload),
            Err(LooseStoreError::ReadOnlyIncident)
        ));
    }

    #[cfg(unix)]
    #[derive(Debug)]
    struct HardlinkTemporary {
        temporary: PathBuf,
    }

    #[cfg(unix)]
    impl FailureInjector for HardlinkTemporary {
        fn check(&self, point: FailurePoint) -> Result<(), LooseStoreError> {
            if point == FailurePoint::AfterTempCreate {
                let source = fs::read_dir(&self.temporary)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("put-")
                    })
                    .unwrap();
                fs::hard_link(source, self.temporary.join("hostile-hardlink")).unwrap();
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    #[test]
    fn temporary_hardlink_is_rejected_before_publication() {
        let directory = TestDirectory::new();
        let temporary = directory.0.join("objects/tmp");
        let store = LooseObjectStore::open_with_injector(
            &directory.0,
            Arc::new(HardlinkTemporary {
                temporary: temporary.clone(),
            }),
        )
        .unwrap();
        let (id, payload, _) = vector_parts();
        assert!(matches!(
            store.put(&id, &payload),
            Err(LooseStoreError::PathMismatch)
        ));
        assert!(!store.path_for(&id).exists());
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_identical_writers_publish_once_without_replacement() {
        let directory = TestDirectory::new();
        let store = Arc::new(LooseObjectStore::open(&directory.0).unwrap());
        let (id, payload, _) = vector_parts();
        let barrier = Arc::new(Barrier::new(8));
        let results = Arc::new(Mutex::new(Vec::new()));
        thread::scope(|scope| {
            for _ in 0..8 {
                let store = Arc::clone(&store);
                let id = id.clone();
                let payload = payload.clone();
                let barrier = Arc::clone(&barrier);
                let results = Arc::clone(&results);
                scope.spawn(move || {
                    barrier.wait();
                    results
                        .lock()
                        .unwrap()
                        .push(store.put(&id, &payload).unwrap());
                });
            }
        });
        let observed: BTreeSet<_> = results.lock().unwrap().iter().copied().collect();
        assert_eq!(
            observed,
            BTreeSet::from([PutLooseOutcome::New, PutLooseOutcome::AlreadyPresent])
        );
        assert!(store.contains(&id));
    }

    struct ShortWriter {
        bytes: Vec<u8>,
        maximum: usize,
        fail_after: Option<usize>,
    }

    impl Write for ShortWriter {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if self
                .fail_after
                .is_some_and(|limit| self.bytes.len() >= limit)
            {
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "simulated ENOSPC",
                ));
            }
            let amount = input.len().min(self.maximum);
            self.bytes.extend_from_slice(&input[..amount]);
            Ok(amount)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn complete_writer_handles_short_writes_zero_progress_and_enospc() {
        let input = b"a bounded frame";
        let mut short = ShortWriter {
            bytes: Vec::new(),
            maximum: 2,
            fail_after: None,
        };
        write_complete(&mut short, input).unwrap();
        assert_eq!(short.bytes, input);
        let mut zero = ShortWriter {
            bytes: Vec::new(),
            maximum: 0,
            fail_after: None,
        };
        assert_eq!(
            write_complete(&mut zero, input).unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
        for offset in 0..input.len() {
            let mut full = ShortWriter {
                bytes: Vec::new(),
                maximum: 1,
                fail_after: Some(offset),
            };
            assert_eq!(
                write_complete(&mut full, input).unwrap_err().kind(),
                io::ErrorKind::StorageFull
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn errors_and_debug_output_do_not_disclose_paths_or_ids() {
        let directory = TestDirectory::new();
        let store = LooseObjectStore::open(&directory.0).unwrap();
        let (id, _, _) = vector_parts();
        assert!(!format!("{store:?}").contains(directory.0.to_str().unwrap()));
        let error = store.read(&id).unwrap_err();
        let rendered = format!("{error:?}");
        assert!(!rendered.contains(&id.to_string()));
        assert!(!rendered.contains(directory.0.to_str().unwrap()));
    }
}
