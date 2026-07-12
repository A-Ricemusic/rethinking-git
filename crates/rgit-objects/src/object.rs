use std::collections::BTreeSet;

use serde_json::{Map as JsonMap, Value as JsonValue};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

use crate::types::{oid, oid_array, optional, text};
use crate::{
    ActorId, ChangeId, DeviceId, HashAlgorithm, LineId, OPERATION_SCHEMA_VERSION_1, ObjectId,
    ObjectKind, PathSegment, PolicyRef, PortablePath, ReferenceKey, SCHEMA_VERSION_0, Signature,
    SignaturePurpose, TypedObjectRef, Value, WallTime,
};

pub const MAX_INLINE_BLOB_BYTES: usize = 64 * 1024;
/// Maximum plaintext bytes carried by one schema-0 chunk.
pub const MAX_CHUNK_BYTES: usize = crate::BULK_MAX_BYTE_STRING_BYTES;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ObjectError {
    #[error(transparent)]
    Canonical(#[from] crate::canonical::CanonicalError),
    #[error("object text field is not Unicode NFC")]
    NonNormalizedText,
    #[error("object violates schema invariant: {0}")]
    Invalid(&'static str),
    #[error("manifest entries are duplicated or collide under case folding")]
    ManifestCollision,
}

/// Implemented by every immutable schema-0 logical object.
pub trait CanonicalObject {
    const KIND: ObjectKind;
    const SCHEMA_VERSION: u64 = SCHEMA_VERSION_0;
    fn canonical_value(&self) -> Result<Value, ObjectError>;
    fn validate(&self) -> Result<(), ObjectError> {
        Ok(())
    }
    /// Resource profile used for the canonical representation of this kind.
    fn canonical_limits() -> crate::CanonicalLimits {
        match Self::KIND {
            ObjectKind::Chunk | ObjectKind::Blob => crate::CanonicalLimits::bulk(),
            _ => crate::CanonicalLimits::metadata(),
        }
    }
    fn encode(&self) -> Result<Vec<u8>, ObjectError> {
        self.encode_with_limits(Self::canonical_limits())
    }
    /// Encode with an explicitly selected resource profile.
    fn encode_with_limits(&self, limits: crate::CanonicalLimits) -> Result<Vec<u8>, ObjectError> {
        self.validate()?;
        Ok(self.canonical_value()?.encode_with_limits(limits)?)
    }
    fn id(&self, algorithm: HashAlgorithm) -> Result<ObjectId, ObjectError> {
        Ok(ObjectId::from_payload(
            Self::KIND as u64,
            Self::SCHEMA_VERSION,
            &self.encode()?,
            algorithm,
        ))
    }
    fn debug_json(&self) -> Result<String, ObjectError> {
        serde_json::to_string_pretty(&value_json(&self.canonical_value()?))
            .map_err(|_| ObjectError::Invalid("JSON rendering failed"))
    }
}

/// Canonical projection and profile-0 preimage for an object carrying signatures.
///
/// The unsigned projection is the complete schema object with only its signature
/// field omitted. It is not itself a storable object because it does not satisfy
/// the kind's production schema.
pub trait SignedObject: CanonicalObject {
    fn unsigned_value(&self) -> Result<Value, ObjectError>;
    fn signatures(&self) -> &[Signature];

    fn unsigned_encode(&self) -> Result<Vec<u8>, ObjectError> {
        self.validate()?;
        Ok(self
            .unsigned_value()?
            .encode_with_limits(Self::canonical_limits())?)
    }

    fn signing_preimage(&self, signature: &Signature) -> Result<Vec<u8>, ObjectError> {
        if !self.signatures().contains(signature) {
            return Err(ObjectError::Invalid(
                "signature metadata is not carried by this object",
            ));
        }
        Ok(crate::signing_preimage(
            signature.algorithm(),
            signature.purpose(),
            signature.signer(),
            signature.signing_key_id(),
            Self::KIND,
            Self::SCHEMA_VERSION,
            &self.unsigned_encode()?,
        ))
    }
}

fn header(kind: ObjectKind, policy: &PolicyRef) -> Vec<(u64, Value)> {
    header_version(kind, SCHEMA_VERSION_0, policy)
}
fn header_version(kind: ObjectKind, schema: u64, policy: &PolicyRef) -> Vec<(u64, Value)> {
    vec![
        (0, Value::Unsigned(kind as u64)),
        (1, Value::Unsigned(schema)),
        (2, policy.value()),
    ]
}
fn optional_header(kind: ObjectKind, policy: Option<&PolicyRef>) -> Vec<(u64, Value)> {
    let mut result = vec![
        (0, Value::Unsigned(kind as u64)),
        (1, Value::Unsigned(SCHEMA_VERSION_0)),
    ];
    optional(&mut result, 2, policy.map(PolicyRef::value));
    result
}
fn ensure_nfc(values: &[&str]) -> Result<(), ObjectError> {
    if values.iter().all(|value| value.nfc().eq(value.chars())) {
        Ok(())
    } else {
        Err(ObjectError::NonNormalizedText)
    }
}
fn ensure_signature_purpose(
    signature: &Signature,
    expected: SignaturePurpose,
) -> Result<(), ObjectError> {
    if signature.purpose() == expected {
        Ok(())
    } else {
        Err(ObjectError::Invalid(
            "signature purpose does not match object kind",
        ))
    }
}
fn ensure_signature_set(
    signatures: &[Signature],
    expected: SignaturePurpose,
) -> Result<(), ObjectError> {
    if signatures.is_empty() {
        return Err(ObjectError::Invalid(
            "signed object requires at least one signature",
        ));
    }
    for signature in signatures {
        ensure_signature_purpose(signature, expected)?;
    }
    if signatures.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ObjectError::Invalid(
            "signatures must be canonically sorted and duplicate-free",
        ));
    }
    Ok(())
}
fn value_json(value: &Value) -> JsonValue {
    match value {
        Value::Unsigned(n) => JsonValue::from(*n),
        Value::Signed(n) => JsonValue::from(*n),
        Value::Bytes(bytes) => JsonValue::String(format!("hex:{}", hex::encode(bytes))),
        Value::Text(s) => JsonValue::String(s.clone()),
        Value::Array(v) => JsonValue::Array(v.iter().map(value_json).collect()),
        Value::Map(entries) => {
            let mut map = JsonMap::new();
            for (key, value) in entries {
                map.insert(key.to_string(), value_json(value));
            }
            JsonValue::Object(map)
        }
        Value::Bool(v) => JsonValue::Bool(*v),
        Value::Null => JsonValue::Null,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub policy_ref: PolicyRef,
    pub bytes: Vec<u8>,
}
impl CanonicalObject for Chunk {
    const KIND: ObjectKind = ObjectKind::Chunk;
    fn validate(&self) -> Result<(), ObjectError> {
        if self.bytes.len() > MAX_CHUNK_BYTES {
            return Err(ObjectError::Invalid("chunk exceeds schema-0 4 MiB maximum"));
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.push((3, Value::Bytes(self.bytes.clone())));
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkRef {
    pub id: ObjectId,
    pub plaintext_length: u64,
}
impl ChunkRef {
    fn value(&self) -> Value {
        Value::Map(vec![
            (0, oid(&self.id)),
            (1, Value::Unsigned(self.plaintext_length)),
        ])
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlobContent {
    Inline(Vec<u8>),
    Chunks(Vec<ChunkRef>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkProfile {
    /// Registry value for the content-defined chunking algorithm.
    pub algorithm: ChunkAlgorithm,
    pub version: u64,
    pub min_size: u64,
    pub target_size: u64,
    pub max_size: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ChunkAlgorithm {
    FastCdc = 0,
}
impl ChunkProfile {
    #[must_use]
    pub const fn fastcdc_v0() -> Self {
        Self {
            algorithm: ChunkAlgorithm::FastCdc,
            version: 0,
            min_size: crate::FASTCDC_V0_MIN_SIZE as u64,
            target_size: crate::FASTCDC_V0_TARGET_SIZE as u64,
            max_size: crate::FASTCDC_V0_MAX_SIZE as u64,
        }
    }
    fn value(&self) -> Value {
        Value::Map(vec![
            (0, Value::Unsigned(self.algorithm as u64)),
            (1, Value::Unsigned(self.version)),
            (2, Value::Unsigned(self.min_size)),
            (3, Value::Unsigned(self.target_size)),
            (4, Value::Unsigned(self.max_size)),
        ])
    }
    fn validate(&self) -> Result<(), ObjectError> {
        if self != &Self::fastcdc_v0() {
            return Err(ObjectError::Invalid("invalid chunk profile size bounds"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub policy_ref: PolicyRef,
    pub byte_length: u64,
    pub content: BlobContent,
    pub chunk_profile: Option<ChunkProfile>,
    pub content_hint: Option<String>,
}
impl CanonicalObject for Blob {
    const KIND: ObjectKind = ObjectKind::Blob;
    fn validate(&self) -> Result<(), ObjectError> {
        if let Some(hint) = &self.content_hint {
            ensure_nfc(&[hint])?;
        }
        match &self.content {
            BlobContent::Inline(_) if self.chunk_profile.is_some() => Err(ObjectError::Invalid(
                "inline blob must not declare a chunk profile",
            )),
            BlobContent::Inline(bytes) if bytes.len() > MAX_INLINE_BLOB_BYTES => {
                Err(ObjectError::Invalid("inline blob exceeds 64 KiB"))
            }
            BlobContent::Inline(bytes) if bytes.len() as u64 != self.byte_length => {
                Err(ObjectError::Invalid("inline length mismatch"))
            }
            BlobContent::Chunks(_) if self.chunk_profile.is_none() => Err(ObjectError::Invalid(
                "chunked blob requires a chunk profile",
            )),
            BlobContent::Chunks(_) if self.byte_length <= MAX_INLINE_BLOB_BYTES as u64 => Err(
                ObjectError::Invalid("blob at or below 64 KiB must use inline representation"),
            ),
            BlobContent::Chunks(chunks)
                if chunks.is_empty()
                    || chunks.iter().any(|chunk| {
                        chunk.plaintext_length == 0
                            || chunk.plaintext_length
                                > self.chunk_profile.as_ref().map_or(0, |p| p.max_size)
                    })
                    || chunks
                        .iter()
                        .try_fold(0_u64, |sum, chunk| sum.checked_add(chunk.plaintext_length))
                        .filter(|sum| *sum == self.byte_length)
                        .is_none() =>
            {
                Err(ObjectError::Invalid(
                    "chunk lengths do not equal blob length",
                ))
            }
            _ => {
                if let Some(profile) = &self.chunk_profile {
                    profile.validate()?;
                }
                Ok(())
            }
        }
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.push((3, Value::Unsigned(self.byte_length)));
        match &self.content {
            BlobContent::Inline(bytes) => m.push((4, Value::Bytes(bytes.clone()))),
            BlobContent::Chunks(chunks) => m.push((
                5,
                Value::Array(chunks.iter().map(ChunkRef::value).collect()),
            )),
        }
        optional(&mut m, 6, self.content_hint.as_deref().map(text));
        optional(
            &mut m,
            7,
            self.chunk_profile.as_ref().map(ChunkProfile::value),
        );
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum FileMode {
    Regular = 0,
    Executable = 1,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestTarget {
    File { blob: ObjectId, mode: FileMode },
    Directory { manifest: ObjectId },
    Symlink { target_blob: ObjectId },
    Subproject { subproject: ObjectId },
    SecretRef { secret_ref: ObjectId },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    pub name: PathSegment,
    pub target: ManifestTarget,
    pub policy_ref: PolicyRef,
}
impl ManifestEntry {
    fn value(&self) -> Value {
        let (kind, target, mode) = match &self.target {
            ManifestTarget::File { blob, mode } => (0, blob, Some(*mode as u64)),
            ManifestTarget::Directory { manifest } => (1, manifest, None),
            ManifestTarget::Symlink { target_blob } => (2, target_blob, None),
            ManifestTarget::Subproject { subproject } => (3, subproject, None),
            ManifestTarget::SecretRef { secret_ref } => (4, secret_ref, None),
        };
        let mut m = vec![
            (0, Value::from(&self.name)),
            (1, Value::Unsigned(kind)),
            (2, oid(target)),
            (4, self.policy_ref.value()),
        ];
        optional(&mut m, 3, mode.map(Value::Unsigned));
        Value::Map(m)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub policy_ref: PolicyRef,
    pub entries: Vec<ManifestEntry>,
}
impl CanonicalObject for Manifest {
    const KIND: ObjectKind = ObjectKind::Manifest;
    fn validate(&self) -> Result<(), ObjectError> {
        let mut names = BTreeSet::new();
        let mut folded = BTreeSet::new();
        for entry in &self.entries {
            entry
                .name
                .ensure_portable()
                .map_err(|_| ObjectError::Invalid("manifest entry is not portable"))?;
            if !names.insert(entry.name.as_str()) || !folded.insert(entry.name.portable_case_fold())
            {
                return Err(ObjectError::ManifestCollision);
            }
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by_key(|entry| entry.name.canonical_bytes());
        let mut m = header(Self::KIND, &self.policy_ref);
        m.push((
            3,
            Value::Array(entries.into_iter().map(ManifestEntry::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subproject {
    pub policy_ref: PolicyRef,
    pub system_kind: String,
    pub repository_identity: Vec<u8>,
    pub revision: Vec<u8>,
    pub native_projection: Option<ObjectId>,
}
impl CanonicalObject for Subproject {
    const KIND: ObjectKind = ObjectKind::Subproject;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[&self.system_kind])
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, text(&self.system_kind)),
            (4, Value::Bytes(self.repository_identity.clone())),
            (5, Value::Bytes(self.revision.clone())),
        ]);
        optional(&mut m, 6, self.native_projection.as_ref().map(oid));
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretRef {
    pub policy_ref: PolicyRef,
    pub provider_kind: String,
    pub locator: String,
    pub exact_version: Option<String>,
    pub value_schema_id: Vec<u8>,
    pub materialization_target: String,
    pub required_capability: String,
    pub encrypted_development_value: Option<ObjectId>,
}
impl CanonicalObject for SecretRef {
    const KIND: ObjectKind = ObjectKind::SecretRef;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[
            &self.provider_kind,
            &self.locator,
            &self.materialization_target,
            &self.required_capability,
        ])?;
        if let Some(v) = &self.exact_version {
            ensure_nfc(&[v])?;
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, text(&self.provider_kind)),
            (4, text(&self.locator)),
            (6, Value::Bytes(self.value_schema_id.clone())),
            (7, text(&self.materialization_target)),
            (8, text(&self.required_capability)),
        ]);
        optional(&mut m, 5, self.exact_version.as_deref().map(text));
        optional(
            &mut m,
            9,
            self.encrypted_development_value.as_ref().map(oid),
        );
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub policy_ref: PolicyRef,
    pub root_manifest: ObjectId,
    pub parents: Vec<ObjectId>,
    pub change_id: ChangeId,
    pub author: ActorId,
    pub device: DeviceId,
    pub logical_time: u64,
    pub wall_time: WallTime,
    pub message_blob: Option<ObjectId>,
}
impl CanonicalObject for Snapshot {
    const KIND: ObjectKind = ObjectKind::Snapshot;
    fn validate(&self) -> Result<(), ObjectError> {
        let unique: BTreeSet<_> = self.parents.iter().collect();
        if unique.len() == self.parents.len() {
            Ok(())
        } else {
            Err(ObjectError::Invalid(
                "snapshot parents must be duplicate-free",
            ))
        }
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, oid(&self.root_manifest)),
            (4, oid_array(&self.parents)),
            (5, self.change_id.into()),
            (6, self.author.into()),
            (7, self.device.into()),
            (8, Value::Unsigned(self.logical_time)),
            (9, self.wall_time.value()),
        ]);
        optional(&mut m, 10, self.message_blob.as_ref().map(oid));
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ChangeState {
    Open = 0,
    Abandoned = 1,
    Landed = 2,
    Superseded = 3,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRevision {
    pub policy_ref: PolicyRef,
    pub change_id: ChangeId,
    pub previous_revision: Option<ObjectId>,
    pub title_blob: ObjectId,
    pub description_blob: ObjectId,
    pub base_snapshot: ObjectId,
    pub current_snapshot: ObjectId,
    pub target_line: LineId,
    pub observed_generation: u64,
    pub owner: ActorId,
    pub author: ActorId,
    pub state: ChangeState,
    pub review_policy: PolicyRef,
    pub landing_policy: PolicyRef,
}
impl CanonicalObject for ChangeRevision {
    const KIND: ObjectKind = ObjectKind::ChangeRevision;
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, self.change_id.into()),
            (5, oid(&self.title_blob)),
            (6, oid(&self.description_blob)),
            (7, oid(&self.base_snapshot)),
            (8, oid(&self.current_snapshot)),
            (9, self.target_line.into()),
            (10, Value::Unsigned(self.observed_generation)),
            (11, self.owner.into()),
            (12, self.author.into()),
            (13, Value::Unsigned(self.state as u64)),
            (14, self.review_policy.value()),
            (15, self.landing_policy.value()),
        ]);
        optional(&mut m, 4, self.previous_revision.as_ref().map(oid));
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineState {
    pub policy_ref: PolicyRef,
    pub line_id: LineId,
    pub display_name: String,
    pub head_snapshot: ObjectId,
    pub generation: u64,
    pub previous_state: Option<ObjectId>,
    pub integration_policy: PolicyRef,
    pub approval_policy: PolicyRef,
    pub release_policy: PolicyRef,
    pub visibility_policy: PolicyRef,
    pub transaction_operation: ObjectId,
    pub signature: Signature,
}
impl CanonicalObject for LineState {
    const KIND: ObjectKind = ObjectKind::LineState;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[&self.display_name])?;
        if (self.generation == 0) == self.previous_state.is_some() {
            return Err(ObjectError::Invalid(
                "only generation zero may omit previous line state",
            ));
        }
        ensure_signature_purpose(&self.signature, SignaturePurpose::LineState)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((13, self.signature.value()));
        Ok(Value::Map(m))
    }
}
impl LineState {
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, self.line_id.into()),
            (4, text(&self.display_name)),
            (5, oid(&self.head_snapshot)),
            (6, Value::Unsigned(self.generation)),
            (8, self.integration_policy.value()),
            (9, self.approval_policy.value()),
            (10, self.release_policy.value()),
            (11, self.visibility_policy.value()),
            (12, oid(&self.transaction_operation)),
        ]);
        optional(&mut m, 7, self.previous_state.as_ref().map(oid));
        m
    }

    /// Checks both the operation ID and its line-advance declaration before a
    /// caller publishes this state through compare-and-swap.
    pub fn validate_transaction(&self, operation: &Operation) -> Result<(), ObjectError> {
        if operation.id(self.transaction_operation.algorithm())? != self.transaction_operation {
            return Err(ObjectError::Invalid(
                "line state transaction operation ID does not match operation",
            ));
        }
        let matches = operation
            .actions
            .iter()
            .filter_map(|action| match action {
                OperationAction::LineAdvance(declaration) if declaration.matches(self) => Some(()),
                _ => None,
            })
            .count();
        if matches != 1 {
            return Err(ObjectError::Invalid(
                "operation must contain exactly one matching line advance declaration",
            ));
        }
        Ok(())
    }

    /// Schema-1 counterpart of [`Self::validate_transaction`].
    pub fn validate_transaction_v1(&self, operation: &OperationV1) -> Result<(), ObjectError> {
        if operation.id(self.transaction_operation.algorithm())? != self.transaction_operation {
            return Err(ObjectError::Invalid(
                "line state transaction operation ID does not match operation",
            ));
        }
        let matches = operation
            .actions
            .iter()
            .filter_map(|action| match action {
                OperationActionV1::LineAdvance(declaration) if declaration.matches(self) => {
                    Some(())
                }
                _ => None,
            })
            .count();
        if matches != 1 {
            return Err(ObjectError::Invalid(
                "operation must contain exactly one matching line advance declaration",
            ));
        }
        Ok(())
    }
}
impl SignedObject for LineState {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        std::slice::from_ref(&self.signature)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictRegion {
    pub start: u64,
    pub end: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub policy_ref: PolicyRef,
    pub base: TypedObjectRef,
    pub left: TypedObjectRef,
    pub right: TypedObjectRef,
    pub path: PortablePath,
    pub conflict_kind: ConflictKind,
    pub merge_driver: String,
    pub merge_driver_version: String,
    pub regions: Vec<ConflictRegion>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ConflictKind {
    Content = 0,
    AddAdd = 1,
    ModifyDelete = 2,
    TypeChange = 3,
}
impl CanonicalObject for Conflict {
    const KIND: ObjectKind = ObjectKind::Conflict;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[&self.merge_driver, &self.merge_driver_version])?;
        let mut previous_end = None;
        for region in &self.regions {
            if region.start >= region.end || previous_end.is_some_and(|end| region.start < end) {
                return Err(ObjectError::Invalid(
                    "conflict regions must be nonempty, ordered, and nonoverlapping",
                ));
            }
            previous_end = Some(region.end);
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, self.base.value()),
            (4, self.left.value()),
            (5, self.right.value()),
            (6, self.path.value()),
            (7, Value::Unsigned(self.conflict_kind as u64)),
            (8, text(&self.merge_driver)),
            (9, text(&self.merge_driver_version)),
            (
                10,
                Value::Array(
                    self.regions
                        .iter()
                        .map(|r| {
                            Value::Map(vec![
                                (0, Value::Unsigned(r.start)),
                                (1, Value::Unsigned(r.end)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]);
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineAdvanceDeclaration {
    pub policy_ref: PolicyRef,
    pub line_id: LineId,
    pub display_name: String,
    pub head_snapshot: ObjectId,
    pub generation: u64,
    pub previous_state: Option<ObjectId>,
    pub integration_policy: PolicyRef,
    pub approval_policy: PolicyRef,
    pub release_policy: PolicyRef,
    pub visibility_policy: PolicyRef,
}
impl LineAdvanceDeclaration {
    fn value(&self) -> Value {
        let mut m = vec![
            (0, self.policy_ref.value()),
            (1, self.line_id.into()),
            (2, text(&self.display_name)),
            (3, oid(&self.head_snapshot)),
            (4, Value::Unsigned(self.generation)),
            (6, self.integration_policy.value()),
            (7, self.approval_policy.value()),
            (8, self.release_policy.value()),
            (9, self.visibility_policy.value()),
        ];
        optional(&mut m, 5, self.previous_state.as_ref().map(oid));
        Value::Map(m)
    }
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[&self.display_name])?;
        if (self.generation == 0) == self.previous_state.is_some() {
            return Err(ObjectError::Invalid(
                "only generation zero line advance may omit previous state",
            ));
        }
        Ok(())
    }
    #[must_use]
    pub fn matches(&self, state: &LineState) -> bool {
        self.policy_ref == state.policy_ref
            && self.line_id == state.line_id
            && self.display_name == state.display_name
            && self.head_snapshot == state.head_snapshot
            && self.generation == state.generation
            && self.previous_state == state.previous_state
            && self.integration_policy == state.integration_policy
            && self.approval_policy == state.approval_policy
            && self.release_policy == state.release_policy
            && self.visibility_policy == state.visibility_policy
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationAction {
    Transition {
        before: Option<TypedObjectRef>,
        after: Option<TypedObjectRef>,
    },
    LineAdvance(Box<LineAdvanceDeclaration>),
}
impl OperationAction {
    fn value(&self) -> Value {
        match self {
            Self::Transition { before, after } => {
                let mut m = vec![(0, Value::Unsigned(0))];
                optional(&mut m, 1, before.as_ref().map(TypedObjectRef::value));
                optional(&mut m, 2, after.as_ref().map(TypedObjectRef::value));
                Value::Map(m)
            }
            Self::LineAdvance(declaration) => {
                Value::Map(vec![(0, Value::Unsigned(1)), (3, declaration.value())])
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation {
    pub policy_ref: PolicyRef,
    pub parents: Vec<ObjectId>,
    pub actor: ActorId,
    pub device: DeviceId,
    pub logical_time: u64,
    pub wall_time: WallTime,
    pub actions: Vec<OperationAction>,
    pub inverse_payloads: Vec<ObjectId>,
    pub public_envelope: Option<ObjectId>,
    pub private_payload: Option<ObjectId>,
    pub signature: Signature,
    pub client_implementation: String,
}
impl CanonicalObject for Operation {
    const KIND: ObjectKind = ObjectKind::Operation;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[&self.client_implementation])?;
        let unique: BTreeSet<_> = self.parents.iter().collect();
        if unique.len() != self.parents.len() {
            return Err(ObjectError::Invalid(
                "operation parents must be duplicate-free",
            ));
        }
        for action in &self.actions {
            match action {
                OperationAction::Transition { before, after } => {
                    if before.is_none() && after.is_none() {
                        return Err(ObjectError::Invalid(
                            "operation transition requires a before or after reference",
                        ));
                    }
                    if after
                        .as_ref()
                        .is_some_and(|reference| reference.kind == ObjectKind::LineState)
                    {
                        return Err(ObjectError::Invalid(
                            "generic operation transition cannot install a line state",
                        ));
                    }
                }
                OperationAction::LineAdvance(declaration) => declaration.validate()?,
            }
        }
        ensure_signature_purpose(&self.signature, SignaturePurpose::Operation)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((12, self.signature.value()));
        Ok(Value::Map(m))
    }
}
impl Operation {
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, oid_array(&self.parents)),
            (4, self.actor.into()),
            (5, self.device.into()),
            (6, Value::Unsigned(self.logical_time)),
            (7, self.wall_time.value()),
            (
                8,
                Value::Array(self.actions.iter().map(OperationAction::value).collect()),
            ),
            (9, oid_array(&self.inverse_payloads)),
            (13, text(&self.client_implementation)),
        ]);
        optional(&mut m, 10, self.public_envelope.as_ref().map(oid));
        optional(&mut m, 11, self.private_payload.as_ref().map(oid));
        m
    }
}
impl SignedObject for Operation {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        std::slice::from_ref(&self.signature)
    }
}

/// Closed action registry for Operation schema 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationActionV1 {
    /// A compare-and-swap transition bound to one exact mutable key.
    BoundTransition {
        key: ReferenceKey,
        before: Option<TypedObjectRef>,
        after: TypedObjectRef,
    },
    LineAdvance(Box<LineAdvanceDeclaration>),
}

impl OperationActionV1 {
    fn value(&self) -> Value {
        match self {
            Self::BoundTransition { key, before, after } => {
                let mut fields = vec![
                    (0, Value::Unsigned(2)),
                    (1, key.value()),
                    (3, after.value()),
                ];
                optional(&mut fields, 2, before.as_ref().map(TypedObjectRef::value));
                Value::Map(fields)
            }
            Self::LineAdvance(declaration) => {
                Value::Map(vec![(0, Value::Unsigned(1)), (3, declaration.value())])
            }
        }
    }
}

/// Key-bound Operation schema 1.
///
/// Schema-0 [`Operation`] remains frozen and independently decodable. New
/// reference publications use this type so the signature commits to the exact
/// mutable key as well as its before/after object IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationV1 {
    pub policy_ref: PolicyRef,
    pub parents: Vec<ObjectId>,
    pub actor: ActorId,
    pub device: DeviceId,
    pub logical_time: u64,
    pub wall_time: WallTime,
    pub actions: Vec<OperationActionV1>,
    pub inverse_payloads: Vec<ObjectId>,
    pub public_envelope: Option<ObjectId>,
    pub private_payload: Option<ObjectId>,
    pub signature: Signature,
    pub client_implementation: String,
}

impl OperationV1 {
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut fields = header_version(Self::KIND, OPERATION_SCHEMA_VERSION_1, &self.policy_ref);
        fields.extend([
            (3, oid_array(&self.parents)),
            (4, self.actor.into()),
            (5, self.device.into()),
            (6, Value::Unsigned(self.logical_time)),
            (7, self.wall_time.value()),
            (
                8,
                Value::Array(self.actions.iter().map(OperationActionV1::value).collect()),
            ),
            (9, oid_array(&self.inverse_payloads)),
            (13, text(&self.client_implementation)),
        ]);
        optional(&mut fields, 10, self.public_envelope.as_ref().map(oid));
        optional(&mut fields, 11, self.private_payload.as_ref().map(oid));
        fields
    }
}

impl CanonicalObject for OperationV1 {
    const KIND: ObjectKind = ObjectKind::Operation;
    const SCHEMA_VERSION: u64 = OPERATION_SCHEMA_VERSION_1;

    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[&self.client_implementation])?;
        let parents: BTreeSet<_> = self.parents.iter().collect();
        if parents.len() != self.parents.len() {
            return Err(ObjectError::Invalid(
                "operation parents must be duplicate-free",
            ));
        }
        let mut keys = BTreeSet::new();
        for action in &self.actions {
            let key = match action {
                OperationActionV1::BoundTransition { key, before, after } => {
                    if matches!(key, ReferenceKey::Line(_) | ReferenceKey::OperationHead) {
                        return Err(ObjectError::Invalid(
                            "bound transition key requires a dedicated operation action",
                        ));
                    }
                    if before
                        .as_ref()
                        .is_some_and(|reference| reference.kind != key.expected_kind())
                        || after.kind != key.expected_kind()
                    {
                        return Err(ObjectError::Invalid(
                            "bound transition object kind does not match its reference key",
                        ));
                    }
                    key.clone()
                }
                OperationActionV1::LineAdvance(declaration) => {
                    declaration.validate()?;
                    ReferenceKey::Line(declaration.line_id)
                }
            };
            if !keys.insert(key) {
                return Err(ObjectError::Invalid(
                    "operation actions must have unique reference keys",
                ));
            }
        }
        ensure_signature_purpose(&self.signature, SignaturePurpose::Operation)
    }

    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut fields = self.unsigned_map();
        fields.push((12, self.signature.value()));
        Ok(Value::Map(fields))
    }
}

impl SignedObject for OperationV1 {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }

    fn signatures(&self) -> &[Signature] {
        std::slice::from_ref(&self.signature)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum MarkerKind {
    Release = 0,
    Deployment = 1,
    Review = 2,
    Policy = 3,
    Bookmark = 4,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub policy_ref: PolicyRef,
    pub marker_kind: MarkerKind,
    pub target: TypedObjectRef,
    pub issuer: ActorId,
    pub issue_time: WallTime,
    pub typed_payload: Vec<u8>,
    pub signature: Signature,
}
impl CanonicalObject for Marker {
    const KIND: ObjectKind = ObjectKind::Marker;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_signature_purpose(&self.signature, SignaturePurpose::Marker)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((8, self.signature.value()));
        Ok(Value::Map(m))
    }
}
impl Marker {
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, Value::Unsigned(self.marker_kind as u64)),
            (4, self.target.value()),
            (5, self.issuer.into()),
            (6, self.issue_time.value()),
            (7, Value::Bytes(self.typed_payload.clone())),
        ]);
        m
    }
}
impl SignedObject for Marker {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        std::slice::from_ref(&self.signature)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub policy_ref: PolicyRef,
    pub source_line: LineId,
    pub source_generation: u64,
    pub source_snapshot: ObjectId,
    pub audience_policy: PolicyRef,
    pub projection_rules: ObjectId,
    pub projected_root: ObjectId,
    pub projection_proof: ObjectId,
    pub version_identifier: String,
    pub release_notes_blob: Option<ObjectId>,
    pub build_provenance: Vec<ObjectId>,
    pub artifacts: Vec<ObjectId>,
    pub policy_decision_evidence: Vec<ObjectId>,
    pub issue_time: WallTime,
    pub signatures: Vec<Signature>,
}
impl CanonicalObject for Release {
    const KIND: ObjectKind = ObjectKind::Release;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_nfc(&[&self.version_identifier])?;
        ensure_signature_set(&self.signatures, SignaturePurpose::Release)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            16,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl Release {
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, &self.policy_ref);
        m.extend([
            (3, self.source_line.into()),
            (4, Value::Unsigned(self.source_generation)),
            (5, oid(&self.source_snapshot)),
            (6, self.audience_policy.value()),
            (7, oid(&self.projection_rules)),
            (8, oid(&self.projected_root)),
            (9, oid(&self.projection_proof)),
            (10, text(&self.version_identifier)),
            (12, oid_array(&self.build_provenance)),
            (13, oid_array(&self.artifacts)),
            (14, oid_array(&self.policy_decision_evidence)),
            (15, self.issue_time.value()),
        ]);
        optional(&mut m, 11, self.release_notes_blob.as_ref().map(oid));
        m
    }
}
impl SignedObject for Release {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum PrincipalKind {
    Actor = 0,
    Group = 1,
    Role = 2,
    Service = 3,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub kind: PrincipalKind,
    pub identifier: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u64)]
pub enum Capability {
    Discover = 0,
    Read = 1,
    Materialize = 2,
    Derive = 3,
    Review = 4,
    Integrate = 5,
    Release = 6,
    Administer = 7,
    Audit = 8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub principal_index: u64,
    pub capabilities: Vec<Capability>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum RedactionMode {
    Omit = 0,
    OpaquePlaceholder = 1,
    TypedSummary = 2,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Omitted only for the repository root policy.
    pub policy_ref: Option<PolicyRef>,
    pub policy_id: crate::PolicyId,
    pub version_sequence: u64,
    pub previous_version: Option<ObjectId>,
    pub principals: Vec<Principal>,
    pub grants: Vec<Grant>,
    pub redaction_mode: RedactionMode,
    /// Registered, non-executable derivation rule plus its canonical parameters.
    pub derivation_rule: DerivationRule,
    pub declassification_requirements: Vec<ObjectId>,
    pub key_epoch: u64,
    pub key_envelope_set: ObjectId,
    pub administrators: Vec<ActorId>,
    pub activation_constraints: Vec<u8>,
    pub signatures: Vec<Signature>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum DerivationRule {
    NoDerivation = 0,
    SamePolicy = 1,
    ExplicitEvidence = 2,
}
impl CanonicalObject for Policy {
    const KIND: ObjectKind = ObjectKind::Policy;
    fn validate(&self) -> Result<(), ObjectError> {
        ensure_signature_set(&self.signatures, SignaturePurpose::Policy)?;
        if (self.version_sequence == 0) == self.previous_version.is_some() {
            return Err(ObjectError::Invalid(
                "only policy version zero may omit a previous version",
            ));
        }
        if self
            .principals
            .iter()
            .any(|principal| principal.identifier.is_empty())
        {
            return Err(ObjectError::Invalid(
                "principal identifier must not be empty",
            ));
        }
        if self
            .grants
            .iter()
            .any(|grant| grant.principal_index >= self.principals.len() as u64)
        {
            return Err(ObjectError::Invalid("grant references an absent principal"));
        }
        if self.grants.iter().any(|grant| {
            let unique: BTreeSet<_> = grant.capabilities.iter().collect();
            unique.len() != grant.capabilities.len()
        }) {
            return Err(ObjectError::Invalid(
                "grant capabilities contain duplicates",
            ));
        }
        let unique_requirements: BTreeSet<_> = self.declassification_requirements.iter().collect();
        if unique_requirements.len() != self.declassification_requirements.len() {
            return Err(ObjectError::Invalid(
                "declassification requirements must be duplicate-free",
            ));
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            15,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl Policy {
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = optional_header(Self::KIND, self.policy_ref.as_ref());
        m.extend([
            (3, self.policy_id.into()),
            (4, Value::Unsigned(self.version_sequence)),
            (
                6,
                Value::Array(
                    self.principals
                        .iter()
                        .map(|p| {
                            Value::Map(vec![
                                (0, Value::Unsigned(p.kind as u64)),
                                (1, Value::Bytes(p.identifier.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                7,
                Value::Array(
                    self.grants
                        .iter()
                        .map(|g| {
                            let mut capabilities: Vec<_> =
                                g.capabilities.iter().map(|c| *c as u64).collect();
                            capabilities.sort_unstable();
                            Value::Map(vec![
                                (0, Value::Unsigned(g.principal_index)),
                                (
                                    1,
                                    Value::Array(
                                        capabilities.into_iter().map(Value::Unsigned).collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (8, Value::Unsigned(self.redaction_mode as u64)),
            (9, Value::Unsigned(self.derivation_rule as u64)),
            (10, oid_array(&self.declassification_requirements)),
            (11, Value::Unsigned(self.key_epoch)),
            (12, oid(&self.key_envelope_set)),
            (
                13,
                Value::Array(
                    self.administrators
                        .iter()
                        .copied()
                        .map(Value::from)
                        .collect(),
                ),
            ),
            (14, Value::Bytes(self.activation_constraints.clone())),
        ]);
        optional(&mut m, 5, self.previous_version.as_ref().map(oid));
        m
    }
}
impl SignedObject for Policy {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}
