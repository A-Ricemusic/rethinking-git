use std::{collections::BTreeMap, fmt, sync::Arc};

use rgit_objects::{AnyObject, ChangeId, LineId, ObjectId, ObjectKind, ReferenceEdge};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutOutcome {
    New,
    AlreadyPresent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectPresence {
    Present,
    Promised,
    Quarantined,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StoredObject {
    id: ObjectId,
    bytes: Arc<[u8]>,
    object: AnyObject,
    references: Arc<[ReferenceEdge]>,
}

impl fmt::Debug for StoredObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredObject")
            .field("kind", &self.kind())
            .field("byte_length", &self.bytes.len())
            .field("reference_count", &self.references.len())
            .finish_non_exhaustive()
    }
}

impl StoredObject {
    pub(crate) fn new(
        id: ObjectId,
        bytes: Vec<u8>,
        object: AnyObject,
    ) -> Result<Self, rgit_objects::DecodeObjectError> {
        let references = object.references()?.into();
        Ok(Self {
            id,
            bytes: bytes.into(),
            object,
            references,
        })
    }

    #[must_use]
    pub fn id(&self) -> &ObjectId {
        &self.id
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub fn object(&self) -> &AnyObject {
        &self.object
    }
    #[must_use]
    pub fn kind(&self) -> ObjectKind {
        self.object.decoded().kind()
    }
    #[must_use]
    pub fn references(&self) -> &[ReferenceEdge] {
        &self.references
    }
}

/// Closed namespaces for mutable repository references.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReferenceKey {
    Line(LineId),
    Change(ChangeId),
    OperationHead,
    Release(LineId),
    Marker([u8; 16]),
}

impl ReferenceKey {
    #[must_use]
    pub const fn expected_kind(&self) -> ObjectKind {
        match self {
            Self::Line(_) => ObjectKind::LineState,
            Self::Change(_) => ObjectKind::ChangeRevision,
            Self::OperationHead => ObjectKind::Operation,
            Self::Release(_) => ObjectKind::Release,
            Self::Marker(_) => ObjectKind::Marker,
        }
    }
}

impl fmt::Debug for ReferenceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Line(_) => "ReferenceKey::Line(<redacted>)",
            Self::Change(_) => "ReferenceKey::Change(<redacted>)",
            Self::OperationHead => "ReferenceKey::OperationHead",
            Self::Release(_) => "ReferenceKey::Release(<redacted>)",
            Self::Marker(_) => "ReferenceKey::Marker(<redacted>)",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReferenceState {
    pub target: ObjectId,
    pub generation: u64,
    pub operation: ObjectId,
}

impl fmt::Debug for ReferenceState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReferenceState")
            .field("target", &"<redacted>")
            .field("generation", &self.generation)
            .field("operation", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ExpectedRef {
    Absent,
    Exact(ReferenceState),
}

impl fmt::Debug for ExpectedRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => formatter.write_str("ExpectedRef::Absent"),
            Self::Exact(state) => formatter
                .debug_tuple("ExpectedRef::Exact")
                .field(state)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PublicationObject {
    pub id: ObjectId,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for PublicationObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicationObject")
            .field("id", &"<redacted>")
            .field("byte_length", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RefUpdate {
    pub key: ReferenceKey,
    pub expected: ExpectedRef,
    pub target: ObjectId,
}

impl fmt::Debug for RefUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefUpdate")
            .field("key", &self.key)
            .field("expected", &self.expected)
            .field("target", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Publication {
    pub objects: Vec<PublicationObject>,
    pub updates: Vec<RefUpdate>,
    pub operation: ObjectId,
}

impl fmt::Debug for Publication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Publication")
            .field("object_count", &self.objects.len())
            .field("updates", &self.updates)
            .field("operation", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Closure {
    objects: BTreeMap<ObjectId, StoredObject>,
}

impl fmt::Debug for Closure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Closure")
            .field("object_count", &self.objects.len())
            .finish()
    }
}

impl Closure {
    pub(crate) fn new(objects: BTreeMap<ObjectId, StoredObject>) -> Self {
        Self { objects }
    }
    #[must_use]
    pub fn objects(&self) -> &BTreeMap<ObjectId, StoredObject> {
        &self.objects
    }
}
