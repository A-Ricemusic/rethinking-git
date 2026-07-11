use rgit_objects::{DecodeObjectError, ObjectKind};
use thiserror::Error;

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
