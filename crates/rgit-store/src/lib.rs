//! Verified, authorization-neutral storage for immutable RGit objects.
//!
//! Every write is decoded and digest-verified before it becomes observable.
//! Authorization belongs above this crate; this layer intentionally exposes no
//! principal-dependent views.

mod error;
mod memory;
mod model;
mod store;

pub use error::{ClosureError, StoreError};
pub use memory::MemoryStore;
pub use model::{
    Closure, ExpectedRef, ObjectPresence, Publication, PublicationObject, PutOutcome, RefUpdate,
    ReferenceKey, ReferenceState, StoredObject,
};
pub use store::{PublicationCandidate, PublicationValidator, Store};
