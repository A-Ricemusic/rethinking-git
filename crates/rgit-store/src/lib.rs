//! Verified, authorization-neutral storage for immutable RGit objects.
//!
//! Every write is decoded and digest-verified before it becomes observable.
//! Authorization belongs above this crate; this layer intentionally exposes no
//! principal-dependent views.

mod error;
mod loose;
mod memory;
mod model;
mod platform;
mod sqlite;
#[cfg(unix)]
mod sqlite_vfs;
#[cfg(not(unix))]
mod sqlite_vfs {
    use std::{io, path::Path};

    pub(crate) const VFS_NAME: &str = "rgit-unsupported";
    pub(crate) struct PinnedSqliteRegistration;
    impl PinnedSqliteRegistration {
        pub(crate) fn register(_: i32, _: &Path) -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "writable SQLite VFS is not qualified on this platform",
            ))
        }
    }
}
mod store;

pub use error::LooseStoreError;
pub use error::{ClosureError, StoreError};
pub use loose::{
    FailureInjector, FailurePoint, InventoryEntry, InventoryEntryKind, LooseObjectStore,
    NoFailures, PutLooseOutcome, VerifiedReader,
};
pub use memory::MemoryStore;
pub use model::{
    Closure, ExpectedRef, ObjectPresence, Publication, PublicationObject, PutOutcome, RefUpdate,
    ReferenceKey, ReferenceState, StoredObject,
};
pub use sqlite::{
    NoTransactionFailures, SqliteFailurePoint, SqliteStore, SqliteStoreOptions,
    TransactionFailureInjector,
};
pub use store::{PublicationCandidate, PublicationValidator, Store};
