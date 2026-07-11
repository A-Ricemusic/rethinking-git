use std::collections::BTreeMap;

use rgit_objects::{CanonicalLimits, ObjectId, ReferenceEdge};

use crate::{
    Closure, ExpectedRef, ObjectPresence, Publication, ReferenceKey, ReferenceState, StoreError,
    StoredObject,
};

/// Immutable view passed to publication validators before state is committed.
pub struct PublicationCandidate<'a> {
    pub(crate) publication: &'a Publication,
    pub(crate) objects: &'a BTreeMap<ObjectId, StoredObject>,
    pub(crate) references: &'a BTreeMap<ReferenceKey, ReferenceState>,
}

impl PublicationCandidate<'_> {
    #[must_use]
    pub fn publication(&self) -> &Publication {
        self.publication
    }

    /// Looks up one known object without exposing physical-store enumeration.
    #[must_use]
    pub fn object(&self, id: &ObjectId) -> Option<&StoredObject> {
        self.objects.get(id)
    }

    /// Returns the staged state of one known reference key.
    #[must_use]
    pub fn reference(&self, key: &ReferenceKey) -> Option<&ReferenceState> {
        self.references.get(key)
    }
}

pub trait PublicationValidator: Send + Sync {
    fn validate(&self, candidate: &PublicationCandidate<'_>) -> Result<(), StoreError>;
}

impl<F> PublicationValidator for F
where
    F: for<'candidate, 'state> Fn(
            &'candidate PublicationCandidate<'state>,
        ) -> Result<(), StoreError>
        + Send
        + Sync,
{
    fn validate(&self, candidate: &PublicationCandidate<'_>) -> Result<(), StoreError> {
        self(candidate)
    }
}

/// Synchronous facade implemented by local and future durable stores.
pub trait Store: Send + Sync {
    fn put(&self, id: ObjectId, bytes: Vec<u8>) -> Result<crate::PutOutcome, StoreError>;
    fn get(&self, id: &ObjectId) -> Result<StoredObject, StoreError>;
    fn presence(&self, id: &ObjectId) -> Option<ObjectPresence>;
    fn mark_promised(&self, id: ObjectId) -> Result<(), StoreError>;
    fn quarantine(&self, id: &ObjectId) -> Result<(), StoreError>;
    fn reference(&self, key: &ReferenceKey) -> Option<ReferenceState>;
    fn compare_and_swap(
        &self,
        key: ReferenceKey,
        expected: ExpectedRef,
        target: ObjectId,
        operation: ObjectId,
        validator: &dyn PublicationValidator,
    ) -> Result<ReferenceState, StoreError>;
    fn publish(
        &self,
        publication: Publication,
        validator: &dyn PublicationValidator,
    ) -> Result<Vec<ReferenceState>, StoreError>;
    fn closure(&self, roots: &[ReferenceEdge]) -> Result<Closure, StoreError>;
    fn generation(&self, id: &ObjectId) -> Result<u64, StoreError>;
    fn is_reachable(&self, ancestor: &ObjectId, descendant: &ObjectId) -> Result<bool, StoreError>;

    fn decode_verified(
        &self,
        id: &ObjectId,
        bytes: &[u8],
    ) -> Result<rgit_objects::AnyObject, StoreError> {
        Ok(rgit_objects::AnyObject::decode_verified(
            bytes,
            id,
            CanonicalLimits::bulk(),
        )?)
    }
}
