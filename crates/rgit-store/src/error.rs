use rgit_objects::{DecodeObjectError, ObjectKind};
use thiserror::Error;

/// Typed failures from immutable loose-record storage.
///
/// The variants and their `Debug` representation intentionally contain neither
/// object identifiers nor repository paths. An auditor can correlate a failure
/// with separately access-controlled request context.
#[derive(Debug, Error)]
pub enum LooseStoreError {
    #[error("loose-record I/O failed during {operation}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("loose record has invalid framing")]
    InvalidFrame,
    #[error("loose record uses an unsupported format")]
    UnsupportedFormat,
    #[error("loose record contains a non-minimal or invalid varint")]
    InvalidVarint,
    #[error("loose record exceeds its allocation limit")]
    AllocationLimit,
    #[error("loose record checksum failed")]
    Checksum,
    #[error("loose record failed canonical object verification")]
    InvalidObject(#[source] DecodeObjectError),
    #[error("loose record metadata does not agree with its object")]
    MetadataMismatch,
    #[error("loose record is not at its derived location")]
    PathMismatch,
    #[error("loose object is unavailable")]
    Unavailable,
    #[error("immutable-object collision or corruption incident")]
    CollisionIncident,
    #[error("loose store is in fail-closed incident mode")]
    ReadOnlyIncident,
    #[error("publication was interrupted at an injected failure boundary")]
    InjectedFailure,
    #[error("platform does not provide required durable no-replace publication")]
    UnsupportedPlatform,
}

impl LooseStoreError {
    pub(crate) fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }
}

/// Failures at the verified storage boundary.
///
/// Display strings deliberately omit object and reference identifiers. Callers
/// may log structured request context only after applying their own disclosure
/// policy.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("object failed canonical decoding or digest verification")]
    InvalidObject(#[source] DecodeObjectError),
    #[error("object is not present")]
    NotPresent,
    #[error("object is quarantined")]
    Quarantined,
    #[error("object is promised but not materialized")]
    Promised,
    #[error("reference key is invalid")]
    InvalidReferenceKey,
    #[error("reference compare-and-swap precondition failed")]
    ReferenceConflict,
    #[error("publication contains duplicate reference updates")]
    DuplicateReferenceUpdate,
    #[error("publication target has the wrong object kind")]
    ReferenceKind,
    #[error("publication operation does not match requested reference transitions")]
    OperationMismatch,
    #[error("reference generation overflow")]
    GenerationOverflow,
    #[error("store revision overflow")]
    RevisionOverflow,
    #[error("publication was denied by validation")]
    PublicationDenied,
    #[error(transparent)]
    Closure(#[from] ClosureError),
    #[error("stored graph is invalid")]
    InvalidGraph,
    #[error("metadata database operation failed")]
    Database,
    #[error("metadata database is foreign, corrupt, or unsupported")]
    UnsupportedDatabase,
    #[error("metadata database requires a newer RGit binary")]
    UpgradeRequired,
    #[error("metadata database requires an explicit supported migration")]
    MigrationRequired,
    #[error("metadata writer is busy; retry the complete operation")]
    RetryableConflict,
    #[error("repository is in durable incident read-only mode")]
    IncidentReadOnly,
    #[error("metadata transaction stopped at an injected failure boundary")]
    InjectedTransactionFailure,
    #[error("durable object storage operation failed")]
    ObjectStorage,
}

impl From<DecodeObjectError> for StoreError {
    fn from(value: DecodeObjectError) -> Self {
        Self::InvalidObject(value)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ClosureError {
    #[error("closure contains a missing object")]
    Missing,
    #[error("closure contains a promised object")]
    Promised,
    #[error("closure contains a quarantined object")]
    Quarantined,
    #[error("closure edge expected {expected:?} but found {actual:?}")]
    WrongKind {
        expected: ObjectKind,
        actual: ObjectKind,
    },
}
