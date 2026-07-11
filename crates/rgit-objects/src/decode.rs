use std::collections::BTreeSet;

use thiserror::Error;

use crate::{CanonicalLimits, HashAlgorithm, ObjectId, ObjectKind, Value, decode_canonical};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedObject {
    kind: ObjectKind,
    schema_version: u64,
    value: Value,
}

/// A closed dispatch enum for every schema-0 object kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnyObject {
    Chunk(DecodedObject),
    Blob(DecodedObject),
    SecretRef(DecodedObject),
    Manifest(DecodedObject),
    Subproject(DecodedObject),
    Snapshot(DecodedObject),
    ChangeRevision(DecodedObject),
    LineState(DecodedObject),
    Conflict(DecodedObject),
    Operation(DecodedObject),
    Marker(DecodedObject),
    Release(DecodedObject),
    Policy(DecodedObject),
    RepositoryRoot(DecodedObject),
    Identity(DecodedObject),
    GroupMembership(DecodedObject),
    KeyEnvelopeSet(DecodedObject),
    ChangeRelation(DecodedObject),
    ConflictResolution(DecodedObject),
    ReviewEvidence(DecodedObject),
    ApprovalEvidence(DecodedObject),
    CiEvidence(DecodedObject),
    PolicyDecisionEvidence(DecodedObject),
    ProjectionRules(DecodedObject),
    ProjectionProof(DecodedObject),
    BuildProvenance(DecodedObject),
    Artifact(DecodedObject),
    OperationPayload(DecodedObject),
    View(DecodedObject),
    Migration(DecodedObject),
    Ruleset(DecodedObject),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceEdge {
    pub role: ReferenceRole,
    pub expected_kind: Option<ObjectKind>,
    pub id: ObjectId,
}

/// The schema field that gives an object reference its meaning.
///
/// Keeping this closed prevents graph consumers from having to infer security-
/// sensitive edge semantics from an untrusted numeric field or array position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceRole {
    ObjectPolicy,
    ManifestEntryPolicy,
    ReviewPolicy,
    LandingPolicy,
    IntegrationPolicy,
    ApprovalPolicy,
    ReleasePolicy,
    VisibilityPolicy,
    AudiencePolicy,
    BlobChunk,
    ManifestEntryTarget,
    SnapshotRootManifest,
    SnapshotParent,
    SnapshotMessage,
    ChangePreviousRevision,
    ChangeTitle,
    ChangeDescription,
    ChangeBaseSnapshot,
    ChangeCurrentSnapshot,
    LineHeadSnapshot,
    LinePreviousState,
    LineTransactionOperation,
    SubprojectNativeProjection,
    SecretDevelopmentValue,
    ConflictBase,
    ConflictLeft,
    ConflictRight,
    OperationParent,
    OperationBefore,
    OperationAfter,
    OperationLinePolicy,
    OperationLineHeadSnapshot,
    OperationLinePreviousState,
    OperationLineIntegrationPolicy,
    OperationLineApprovalPolicy,
    OperationLineReleasePolicy,
    OperationLineVisibilityPolicy,
    OperationInversePayload,
    OperationPublicEnvelope,
    OperationPrivatePayload,
    MarkerTarget,
    ReleaseSourceSnapshot,
    ReleaseProjectionRules,
    ReleaseProjectionProof,
    ReleaseProjectedRoot,
    ReleaseNotes,
    ReleaseBuildProvenance,
    ReleaseArtifact,
    ReleasePolicyEvidence,
    PolicyPreviousVersion,
    PolicyDeclassificationRequirement,
    PolicyKeyEnvelopeSet,
    RepositoryRootPolicy,
    RepositoryTrustedIdentity,
    RepositoryBootstrapKeyEnvelopeSet,
    RepositoryGenesisOperation,
    RepositoryInitialLineState,
    IdentityPrevious,
    IdentityKeyNotBefore,
    IdentityKeyNotAfter,
    IdentityActivationOperation,
    IdentityNotAfterOperation,
    MembershipPrevious,
    MembershipActivationOperation,
    MembershipNotAfterOperation,
    ChangeRelationSource,
    ChangeRelationResult,
    ChangeRelationProvenance,
    ChangeRelationOperation,
    ResolutionConflict,
    ResolutionResult,
    ResolutionProvenance,
    EvidenceTarget,
    EvidenceSnapshot,
    EvidenceRuleset,
    EvidenceRelated,
    CiRunnerIdentity,
    CiBuildProvenance,
    ProjectionRulesPrevious,
    ProjectionSourceSnapshot,
    ProjectionRules,
    ProjectionAudiencePolicy,
    ProjectionManifest,
    BuildSnapshot,
    BuildRuleset,
    BuildIdentity,
    BuildInput,
    BuildOutput,
    ArtifactBlob,
    PayloadReference,
    ViewPolicy,
    ViewLineState,
    ViewManifest,
    MigrationOldObject,
    MigrationNewObject,
    MigrationToolIdentity,
    RulesetPrevious,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeObjectError {
    #[error(transparent)]
    Canonical(#[from] crate::canonical::CanonicalError),
    #[error("top-level object is not a map")]
    NotMap,
    #[error("missing, duplicated, or wrongly typed field {0}")]
    Field(u64),
    #[error("unknown object kind {0}")]
    Kind(u64),
    #[error("unsupported schema version {0}")]
    Schema(u64),
    #[error("unknown field {0} for this schema")]
    UnknownField(u64),
    #[error("invalid object ID in field {0}")]
    ObjectId(u64),
    #[error("object ID digest does not match canonical object")]
    Digest,
}

impl AnyObject {
    pub fn decode(bytes: &[u8], limits: CanonicalLimits) -> Result<Self, DecodeObjectError> {
        // A caller may tighten admission limits, but cannot use this API to
        // relax schema 0 beyond the largest (Chunk/Blob) resource profile.
        let admission_limits = limits.restricted_by(CanonicalLimits::bulk());
        let value = decode_canonical(bytes, admission_limits)?;
        let map = as_map(&value)?;
        let kind_number = unsigned(field(map, 0)?, 0)?;
        let kind = ObjectKind::try_from(kind_number).map_err(DecodeObjectError::Kind)?;
        let schema = unsigned(field(map, 1)?, 1)?;
        if schema != 0 {
            return Err(DecodeObjectError::Schema(schema));
        }
        let kind_limits = match kind {
            ObjectKind::Chunk | ObjectKind::Blob => CanonicalLimits::bulk(),
            _ => CanonicalLimits::metadata(),
        };
        // Enforce the kind-specific ceiling even when initial parsing needed
        // the broader bulk profile in order to discover the kind.
        value.encode_with_limits(limits.restricted_by(kind_limits))?;
        validate_fields(kind, map)?;
        let decoded = DecodedObject {
            kind,
            schema_version: schema,
            value,
        };
        Ok(match kind {
            ObjectKind::Chunk => Self::Chunk(decoded),
            ObjectKind::Blob => Self::Blob(decoded),
            ObjectKind::SecretRef => Self::SecretRef(decoded),
            ObjectKind::Manifest => Self::Manifest(decoded),
            ObjectKind::Subproject => Self::Subproject(decoded),
            ObjectKind::Snapshot => Self::Snapshot(decoded),
            ObjectKind::ChangeRevision => Self::ChangeRevision(decoded),
            ObjectKind::LineState => Self::LineState(decoded),
            ObjectKind::Conflict => Self::Conflict(decoded),
            ObjectKind::Operation => Self::Operation(decoded),
            ObjectKind::Marker => Self::Marker(decoded),
            ObjectKind::Release => Self::Release(decoded),
            ObjectKind::Policy => Self::Policy(decoded),
            ObjectKind::RepositoryRoot => Self::RepositoryRoot(decoded),
            ObjectKind::Identity => Self::Identity(decoded),
            ObjectKind::GroupMembership => Self::GroupMembership(decoded),
            ObjectKind::KeyEnvelopeSet => Self::KeyEnvelopeSet(decoded),
            ObjectKind::ChangeRelation => Self::ChangeRelation(decoded),
            ObjectKind::ConflictResolution => Self::ConflictResolution(decoded),
            ObjectKind::ReviewEvidence => Self::ReviewEvidence(decoded),
            ObjectKind::ApprovalEvidence => Self::ApprovalEvidence(decoded),
            ObjectKind::CiEvidence => Self::CiEvidence(decoded),
            ObjectKind::PolicyDecisionEvidence => Self::PolicyDecisionEvidence(decoded),
            ObjectKind::ProjectionRules => Self::ProjectionRules(decoded),
            ObjectKind::ProjectionProof => Self::ProjectionProof(decoded),
            ObjectKind::BuildProvenance => Self::BuildProvenance(decoded),
            ObjectKind::Artifact => Self::Artifact(decoded),
            ObjectKind::OperationPayload => Self::OperationPayload(decoded),
            ObjectKind::View => Self::View(decoded),
            ObjectKind::Migration => Self::Migration(decoded),
            ObjectKind::Ruleset => Self::Ruleset(decoded),
        })
    }

    pub fn decode_verified(
        bytes: &[u8],
        expected: &ObjectId,
        limits: CanonicalLimits,
    ) -> Result<Self, DecodeObjectError> {
        let object = Self::decode(bytes, limits)?;
        if &object.id(expected.algorithm())? != expected {
            return Err(DecodeObjectError::Digest);
        }
        Ok(object)
    }
    #[must_use]
    pub fn decoded(&self) -> &DecodedObject {
        match self {
            Self::Chunk(v)
            | Self::Blob(v)
            | Self::SecretRef(v)
            | Self::Manifest(v)
            | Self::Subproject(v)
            | Self::Snapshot(v)
            | Self::ChangeRevision(v)
            | Self::LineState(v)
            | Self::Conflict(v)
            | Self::Operation(v)
            | Self::Marker(v)
            | Self::Release(v)
            | Self::Policy(v) => v,
            Self::RepositoryRoot(v)
            | Self::Identity(v)
            | Self::GroupMembership(v)
            | Self::KeyEnvelopeSet(v)
            | Self::ChangeRelation(v)
            | Self::ConflictResolution(v)
            | Self::ReviewEvidence(v)
            | Self::ApprovalEvidence(v)
            | Self::CiEvidence(v)
            | Self::PolicyDecisionEvidence(v)
            | Self::ProjectionRules(v)
            | Self::ProjectionProof(v)
            | Self::BuildProvenance(v)
            | Self::Artifact(v)
            | Self::OperationPayload(v)
            | Self::View(v)
            | Self::Migration(v)
            | Self::Ruleset(v) => v,
        }
    }
    pub fn id(&self, algorithm: HashAlgorithm) -> Result<ObjectId, DecodeObjectError> {
        let d = self.decoded();
        let limits = match d.kind {
            ObjectKind::Chunk | ObjectKind::Blob => CanonicalLimits::bulk(),
            _ => CanonicalLimits::metadata(),
        };
        let bytes = d.value.encode_with_limits(limits)?;
        Ok(ObjectId::from_payload(
            d.kind as u64,
            d.schema_version,
            &bytes,
            algorithm,
        ))
    }
    pub fn references(&self) -> Result<Vec<ReferenceEdge>, DecodeObjectError> {
        references(self.decoded())
    }
}

impl DecodedObject {
    #[must_use]
    pub const fn kind(&self) -> ObjectKind {
        self.kind
    }
    #[must_use]
    pub const fn schema_version(&self) -> u64 {
        self.schema_version
    }
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.value
    }
}

fn validate_fields(kind: ObjectKind, map: &[(u64, Value)]) -> Result<(), DecodeObjectError> {
    let (required, optional): (&[u64], &[u64]) = match kind {
        ObjectKind::Chunk => (&[0, 1, 2, 3], &[]),
        ObjectKind::Blob => (&[0, 1, 2, 3], &[4, 5, 6, 7]),
        ObjectKind::SecretRef => (&[0, 1, 2, 3, 4, 6, 7, 8], &[5, 9]),
        ObjectKind::Manifest => (&[0, 1, 2, 3], &[]),
        ObjectKind::Subproject => (&[0, 1, 2, 3, 4, 5], &[6]),
        ObjectKind::Snapshot => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], &[10]),
        ObjectKind::ChangeRevision => (&[0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], &[4]),
        ObjectKind::LineState => (&[0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13], &[7]),
        ObjectKind::Conflict => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], &[]),
        ObjectKind::Operation => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13], &[10, 11]),
        ObjectKind::Marker => (&[0, 1, 2, 3, 4, 5, 6, 7, 8], &[]),
        ObjectKind::Release => (
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14, 15, 16],
            &[11],
        ),
        ObjectKind::Policy => (&[0, 1, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], &[2, 5]),
        ObjectKind::RepositoryRoot => (&[0, 1, 3, 4, 5, 6, 7, 8, 9, 10], &[]),
        ObjectKind::Identity => (&[0, 1, 3, 4, 5, 7, 8, 9, 10, 13], &[2, 6, 11, 12]),
        ObjectKind::GroupMembership => (&[0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 12], &[6, 11]),
        ObjectKind::KeyEnvelopeSet => (&[0, 1, 3, 4, 5], &[2]),
        ObjectKind::ChangeRelation => (&[0, 1, 2, 3, 4, 5, 6, 7], &[]),
        ObjectKind::ConflictResolution => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], &[]),
        ObjectKind::ReviewEvidence
        | ObjectKind::ApprovalEvidence
        | ObjectKind::PolicyDecisionEvidence => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], &[]),
        ObjectKind::CiEvidence => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14], &[15]),
        ObjectKind::ProjectionRules => (&[0, 1, 2, 3, 5, 6], &[4]),
        ObjectKind::ProjectionProof => (&[0, 1, 2, 3, 4, 5, 6, 7, 8], &[]),
        ObjectKind::BuildProvenance => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], &[]),
        ObjectKind::Artifact => (&[0, 1, 2, 3, 4, 5, 6], &[7, 8]),
        ObjectKind::OperationPayload => (&[0, 1, 2, 3, 4, 5, 6], &[]),
        ObjectKind::View => (&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], &[]),
        ObjectKind::Migration => (&[0, 1, 2, 3, 4, 5, 6, 7, 8], &[]),
        ObjectKind::Ruleset => (&[0, 1, 2, 3, 4, 6, 7], &[5]),
    };
    for key in required {
        field(map, *key)?;
    }
    for (key, _) in map {
        if !required.contains(key) && !optional.contains(key) {
            return Err(DecodeObjectError::UnknownField(*key));
        }
    }
    // Common policy references and all object ID byte fields are structurally checked here.
    if !matches!(
        kind,
        ObjectKind::Policy
            | ObjectKind::RepositoryRoot
            | ObjectKind::Identity
            | ObjectKind::KeyEnvelopeSet
    ) || map.iter().any(|(k, _)| *k == 2)
    {
        policy_ref(field(map, 2)?, 2)?;
    }
    validate_shapes(kind, map)
}

fn validate_shapes(kind: ObjectKind, map: &[(u64, Value)]) -> Result<(), DecodeObjectError> {
    match kind {
        ObjectKind::Chunk => {
            if bytes(field(map, 3)?, 3)?.len() > crate::MAX_CHUNK_BYTES {
                return Err(DecodeObjectError::Field(3));
            }
        }
        ObjectKind::Blob => {
            let byte_length = unsigned(field(map, 3)?, 3)?;
            if map.iter().filter(|(k, _)| *k == 4 || *k == 5).count() != 1 {
                return Err(DecodeObjectError::Field(4));
            }
            if let Some(inline) = map.iter().find(|(k, _)| *k == 4).map(|(_, v)| v) {
                if bytes(inline, 4)?.len() > crate::MAX_INLINE_BLOB_BYTES
                    || bytes(inline, 4)?.len() as u64 != byte_length
                    || map.iter().any(|(k, _)| *k == 7)
                {
                    return Err(DecodeObjectError::Field(4));
                }
            } else {
                if byte_length <= crate::MAX_INLINE_BLOB_BYTES as u64 {
                    return Err(DecodeObjectError::Field(5));
                }
                let mut total = 0_u64;
                let chunk_values = array(field(map, 5)?, 5)?;
                if chunk_values.is_empty() {
                    return Err(DecodeObjectError::Field(5));
                }
                for chunk in chunk_values {
                    let chunk = exact_map(chunk, &[0, 1], &[], 5)?;
                    oid(field(chunk, 0)?, 5)?;
                    let length = unsigned(field(chunk, 1)?, 5)?;
                    if length == 0 || length > crate::FASTCDC_V0_MAX_SIZE as u64 {
                        return Err(DecodeObjectError::Field(5));
                    }
                    total = total
                        .checked_add(length)
                        .ok_or(DecodeObjectError::Field(5))?;
                }
                if total != byte_length {
                    return Err(DecodeObjectError::Field(5));
                }
                let profile = exact_map(field(map, 7)?, &[0, 1, 2, 3, 4], &[], 7)?;
                if unsigned(field(profile, 0)?, 7)? != 0 || unsigned(field(profile, 1)?, 7)? != 0 {
                    return Err(DecodeObjectError::Field(7));
                }
                let min = unsigned(field(profile, 2)?, 7)?;
                let target = unsigned(field(profile, 3)?, 7)?;
                let max = unsigned(field(profile, 4)?, 7)?;
                if min != crate::FASTCDC_V0_MIN_SIZE as u64
                    || target != crate::FASTCDC_V0_TARGET_SIZE as u64
                    || max != crate::FASTCDC_V0_MAX_SIZE as u64
                {
                    return Err(DecodeObjectError::Field(7));
                }
            }
            if let Some(hint) = map.iter().find(|(k, _)| *k == 6).map(|(_, v)| v) {
                text(hint, 6)?;
            }
        }
        ObjectKind::Manifest => {
            let mut previous = None;
            let mut folded_names = BTreeSet::new();
            for entry in array(field(map, 3)?, 3)? {
                let entry = exact_map(entry, &[0, 1, 2, 4], &[3], 3)?;
                let name = text(field(entry, 0)?, 3)?;
                let segment = crate::PathSegment::new_portable(name)
                    .map_err(|_| DecodeObjectError::Field(3))?;
                if !folded_names.insert(segment.portable_case_fold()) {
                    return Err(DecodeObjectError::Field(3));
                }
                let encoded = Value::Text(name.to_owned()).encode()?;
                if previous.as_ref().is_some_and(|old| old >= &encoded) {
                    return Err(DecodeObjectError::Field(3));
                }
                previous = Some(encoded);
                let kind = unsigned(field(entry, 1)?, 3)?;
                if kind > 4 || (kind == 0) != entry.iter().any(|(k, _)| *k == 3) {
                    return Err(DecodeObjectError::Field(3));
                }
                if kind == 0 && unsigned(field(entry, 3)?, 3)? > 1 {
                    return Err(DecodeObjectError::Field(3));
                }
                oid(field(entry, 2)?, 3)?;
                policy_ref(field(entry, 4)?, 3)?;
            }
        }
        ObjectKind::Snapshot => {
            oid(field(map, 3)?, 3)?;
            let parents = oid_values(field(map, 4)?, 4)?;
            ensure_unique_ids(&parents, 4)?;
            fixed_bytes(field(map, 5)?, 5, 16)?;
            fixed_bytes(field(map, 6)?, 6, 16)?;
            fixed_bytes(field(map, 7)?, 7, 16)?;
            unsigned(field(map, 8)?, 8)?;
            wall_time(field(map, 9)?, 9)?;
            if let Some(message) = map.iter().find(|(k, _)| *k == 10).map(|(_, v)| v) {
                oid(message, 10)?;
            }
        }
        ObjectKind::ChangeRevision => {
            fixed_bytes(field(map, 3)?, 3, 16)?;
            if let Some(previous) = map.iter().find(|(k, _)| *k == 4).map(|(_, v)| v) {
                oid(previous, 4)?;
            }
            for k in [5, 6, 7, 8] {
                oid(field(map, k)?, k)?;
            }
            fixed_bytes(field(map, 9)?, 9, 16)?;
            unsigned(field(map, 10)?, 10)?;
            fixed_bytes(field(map, 11)?, 11, 16)?;
            fixed_bytes(field(map, 12)?, 12, 16)?;
            if unsigned(field(map, 13)?, 13)? > 3 {
                return Err(DecodeObjectError::Field(13));
            }
            policy_ref(field(map, 14)?, 14)?;
            policy_ref(field(map, 15)?, 15)?;
        }
        ObjectKind::LineState => {
            fixed_bytes(field(map, 3)?, 3, 16)?;
            text(field(map, 4)?, 4)?;
            oid(field(map, 5)?, 5)?;
            let generation = unsigned(field(map, 6)?, 6)?;
            let has_previous = map.iter().any(|(k, _)| *k == 7);
            if (generation == 0) == has_previous {
                return Err(DecodeObjectError::Field(7));
            }
            if let Some(previous) = map.iter().find(|(k, _)| *k == 7).map(|(_, v)| v) {
                oid(previous, 7)?;
            }
            oid(field(map, 12)?, 12)?;
            for k in [8, 9, 10, 11] {
                policy_ref(field(map, k)?, k)?;
            }
            signature(field(map, 13)?, 13, crate::SignaturePurpose::LineState)?;
        }
        ObjectKind::Conflict => {
            typed_ref(field(map, 3)?, 3)?;
            typed_ref(field(map, 4)?, 4)?;
            typed_ref(field(map, 5)?, 5)?;
            let segments = array(field(map, 6)?, 6)?
                .iter()
                .map(|segment| {
                    crate::PathSegment::new_portable(text(segment, 6)?)
                        .map_err(|_| DecodeObjectError::Field(6))
                })
                .collect::<Result<Vec<_>, _>>()?;
            crate::PortablePath::new(segments).map_err(|_| DecodeObjectError::Field(6))?;
            if unsigned(field(map, 7)?, 7)? > 3 {
                return Err(DecodeObjectError::Field(7));
            }
            text(field(map, 8)?, 8)?;
            text(field(map, 9)?, 9)?;
            let mut previous_end = None;
            for region in array(field(map, 10)?, 10)? {
                let region = exact_map(region, &[0, 1], &[], 10)?;
                let start = unsigned(field(region, 0)?, 10)?;
                let end = unsigned(field(region, 1)?, 10)?;
                if start >= end || previous_end.is_some_and(|previous| start < previous) {
                    return Err(DecodeObjectError::Field(10));
                }
                previous_end = Some(end);
            }
        }
        ObjectKind::Operation => {
            let parents = oid_values(field(map, 3)?, 3)?;
            ensure_unique_ids(&parents, 3)?;
            fixed_bytes(field(map, 4)?, 4, 16)?;
            fixed_bytes(field(map, 5)?, 5, 16)?;
            unsigned(field(map, 6)?, 6)?;
            wall_time(field(map, 7)?, 7)?;
            for action in array(field(map, 8)?, 8)? {
                let action_map = as_map(action)?;
                match unsigned(field(action_map, 0)?, 8)? {
                    0 => {
                        let action = exact_map(action, &[0], &[1, 2], 8)?;
                        let before = action.iter().find(|(k, _)| *k == 1).map(|(_, v)| v);
                        let after = action.iter().find(|(k, _)| *k == 2).map(|(_, v)| v);
                        if before.is_none() && after.is_none() {
                            return Err(DecodeObjectError::Field(8));
                        }
                        if let Some(value) = before {
                            typed_ref(value, 8)?;
                        }
                        if let Some(value) = after {
                            let kind = typed_ref_kind(value, 8)?;
                            if kind == ObjectKind::LineState {
                                return Err(DecodeObjectError::Field(8));
                            }
                        }
                    }
                    1 => {
                        let action = exact_map(action, &[0, 3], &[], 8)?;
                        line_advance(field(action, 3)?, 8)?;
                    }
                    _ => return Err(DecodeObjectError::Field(8)),
                }
            }
            oid_array(field(map, 9)?, 9)?;
            if let Some(value) = map.iter().find(|(k, _)| *k == 10).map(|(_, v)| v) {
                oid(value, 10)?;
            }
            if let Some(value) = map.iter().find(|(k, _)| *k == 11).map(|(_, v)| v) {
                oid(value, 11)?;
            }
            signature(field(map, 12)?, 12, crate::SignaturePurpose::Operation)?;
            text(field(map, 13)?, 13)?;
        }
        ObjectKind::Marker => {
            let marker = unsigned(field(map, 3)?, 3)?;
            if marker > 4 {
                return Err(DecodeObjectError::Field(3));
            }
            typed_ref(field(map, 4)?, 4)?;
            fixed_bytes(field(map, 5)?, 5, 16)?;
            wall_time(field(map, 6)?, 6)?;
            bytes(field(map, 7)?, 7)?;
            signature(field(map, 8)?, 8, crate::SignaturePurpose::Marker)?;
        }
        ObjectKind::Release => {
            fixed_bytes(field(map, 3)?, 3, 16)?;
            unsigned(field(map, 4)?, 4)?;
            for k in [5, 7, 8] {
                oid(field(map, k)?, k)?;
            }
            text(field(map, 10)?, 10)?;
            policy_ref(field(map, 6)?, 6)?;
            oid(field(map, 9)?, 9)?;
            for k in [12, 13, 14] {
                oid_array(field(map, k)?, k)?;
            }
            wall_time(field(map, 15)?, 15)?;
            let signatures = array(field(map, 16)?, 16)?;
            if signatures.is_empty() {
                return Err(DecodeObjectError::Field(16));
            }
            for value in signatures {
                signature(value, 16, crate::SignaturePurpose::Release)?;
            }
            ensure_sorted_unique_values(signatures, 16)?;
        }
        ObjectKind::Policy => {
            fixed_bytes(field(map, 3)?, 3, 16)?;
            let version = unsigned(field(map, 4)?, 4)?;
            let previous = map.iter().find(|(k, _)| *k == 5).map(|(_, v)| v);
            if (version == 0) == previous.is_some() {
                return Err(DecodeObjectError::Field(5));
            }
            if let Some(value) = previous {
                oid(value, 5)?;
            }
            let principals = array(field(map, 6)?, 6)?;
            for principal in principals {
                let principal = exact_map(principal, &[0, 1], &[], 6)?;
                if unsigned(field(principal, 0)?, 6)? > 3
                    || bytes(field(principal, 1)?, 6)?.is_empty()
                {
                    return Err(DecodeObjectError::Field(6));
                }
            }
            for grant in array(field(map, 7)?, 7)? {
                let grant = exact_map(grant, &[0, 1], &[], 7)?;
                if unsigned(field(grant, 0)?, 7)? >= principals.len() as u64 {
                    return Err(DecodeObjectError::Field(7));
                }
                let mut previous_capability = None;
                for capability in array(field(grant, 1)?, 7)? {
                    let capability = unsigned(capability, 7)?;
                    if capability > 8
                        || previous_capability.is_some_and(|previous| previous >= capability)
                    {
                        return Err(DecodeObjectError::Field(7));
                    }
                    previous_capability = Some(capability);
                }
            }
            oid(field(map, 12)?, 12)?;
            let mode = unsigned(field(map, 8)?, 8)?;
            if mode > 2 {
                return Err(DecodeObjectError::Field(8));
            }
            if unsigned(field(map, 9)?, 9)? > 2 {
                return Err(DecodeObjectError::Field(9));
            }
            let requirements = oid_values(field(map, 10)?, 10)?;
            ensure_unique_ids(&requirements, 10)?;
            unsigned(field(map, 11)?, 11)?;
            for administrator in array(field(map, 13)?, 13)? {
                fixed_bytes(administrator, 13, 16)?;
            }
            bytes(field(map, 14)?, 14)?;
            let signatures = array(field(map, 15)?, 15)?;
            if signatures.is_empty() {
                return Err(DecodeObjectError::Field(15));
            }
            for value in signatures {
                signature(value, 15, crate::SignaturePurpose::Policy)?;
            }
            ensure_sorted_unique_values(signatures, 15)?;
        }
        ObjectKind::SecretRef => {
            for k in [3, 4, 7, 8] {
                text(field(map, k)?, k)?;
            }
            bytes(field(map, 6)?, 6)?;
            if let Some(value) = map.iter().find(|(k, _)| *k == 5).map(|(_, v)| v) {
                text(value, 5)?;
            }
            if let Some(value) = map.iter().find(|(k, _)| *k == 9).map(|(_, v)| v) {
                oid(value, 9)?;
            }
        }
        ObjectKind::Subproject => {
            text(field(map, 3)?, 3)?;
            bytes(field(map, 4)?, 4)?;
            bytes(field(map, 5)?, 5)?;
            if let Some(value) = map.iter().find(|(k, _)| *k == 6).map(|(_, v)| v) {
                oid(value, 6)?;
            }
        }
        ObjectKind::RepositoryRoot => {
            fixed_bytes(field(map, 3)?, 3, 16)?;
            oid(field(map, 4)?, 4)?;
            let identities = oid_values(field(map, 5)?, 5)?;
            ensure_sorted_unique_ids(&identities, 5)?;
            if identities.is_empty() {
                return Err(DecodeObjectError::Field(5));
            }
            oid(field(map, 6)?, 6)?;
            oid(field(map, 7)?, 7)?;
            let lines = oid_values(field(map, 8)?, 8)?;
            ensure_sorted_unique_ids(&lines, 8)?;
            if lines.is_empty() {
                return Err(DecodeObjectError::Field(8));
            }
            if unsigned(field(map, 9)?, 9)? > 1 {
                return Err(DecodeObjectError::Field(9));
            }
            signature_set(field(map, 10)?, 10, crate::SignaturePurpose::RepositoryRoot)?;
        }
        ObjectKind::Identity => {
            if unsigned(field(map, 3)?, 3)? > 1 {
                return Err(DecodeObjectError::Field(3));
            }
            fixed_bytes(field(map, 4)?, 4, 16)?;
            let version = unsigned(field(map, 5)?, 5)?;
            let previous = optional_field(map, 6);
            let activation = optional_field(map, 11);
            let has_policy = optional_field(map, 2).is_some();
            if (version == 0) != (previous.is_none() && activation.is_none() && !has_policy) {
                return Err(DecodeObjectError::Field(5));
            }
            if version > 0 && (previous.is_none() || activation.is_none() || !has_policy) {
                return Err(DecodeObjectError::Field(5));
            }
            if let Some(value) = previous {
                oid(value, 6)?;
            }
            let signing = array(field(map, 7)?, 7)?;
            if signing.is_empty() {
                return Err(DecodeObjectError::Field(7));
            }
            let encryption = array(field(map, 8)?, 8)?;
            let mut key_ids = BTreeSet::new();
            for value in signing {
                let (algorithm, key_id) = public_key(value, 7)?;
                if algorithm != 0 || !key_ids.insert(key_id) {
                    return Err(DecodeObjectError::Field(7));
                }
            }
            for value in encryption {
                let (algorithm, key_id) = public_key(value, 8)?;
                if algorithm != 1 || !key_ids.insert(key_id) {
                    return Err(DecodeObjectError::Field(8));
                }
            }
            ensure_sorted_unique_values(signing, 7)?;
            ensure_sorted_unique_values(encryption, 8)?;
            fixed_bytes(field(map, 9)?, 9, 16)?;
            if unsigned(field(map, 10)?, 10)? > 2 {
                return Err(DecodeObjectError::Field(10));
            }
            if let Some(value) = activation {
                oid(value, 11)?;
            }
            if let Some(value) = optional_field(map, 12) {
                oid(value, 12)?;
            }
            signature_set(field(map, 13)?, 13, crate::SignaturePurpose::Identity)?;
        }
        ObjectKind::GroupMembership => {
            fixed_bytes(field(map, 3)?, 3, 16)?;
            fixed_bytes(field(map, 4)?, 4, 16)?;
            let version = unsigned(field(map, 5)?, 5)?;
            let previous = optional_field(map, 6);
            if (version == 0) == previous.is_some() {
                return Err(DecodeObjectError::Field(6));
            }
            if let Some(value) = previous {
                oid(value, 6)?;
            }
            principal(field(map, 7)?, 7)?;
            if unsigned(field(map, 8)?, 8)? > 1 {
                return Err(DecodeObjectError::Field(8));
            }
            fixed_bytes(field(map, 9)?, 9, 16)?;
            oid(field(map, 10)?, 10)?;
            if let Some(value) = optional_field(map, 11) {
                oid(value, 11)?;
            }
            signature_set(
                field(map, 12)?,
                12,
                crate::SignaturePurpose::GroupMembership,
            )?;
        }
        ObjectKind::KeyEnvelopeSet => {
            let epoch = unsigned(field(map, 3)?, 3)?;
            if (epoch == 0) != optional_field(map, 2).is_none() {
                return Err(DecodeObjectError::Field(3));
            }
            if unsigned(field(map, 4)?, 4)? != 0 {
                return Err(DecodeObjectError::Field(4));
            }
            let recipients = array(field(map, 5)?, 5)?;
            if recipients.is_empty() {
                return Err(DecodeObjectError::Field(5));
            }
            for value in recipients {
                let r = exact_map(value, &[0, 1, 2], &[], 5)?;
                principal(field(r, 0)?, 5)?;
                fixed_nonzero_bytes(field(r, 1)?, 5, 32)?;
                if bytes(field(r, 2)?, 5)?.is_empty() {
                    return Err(DecodeObjectError::Field(5));
                }
            }
            ensure_sorted_unique_values(recipients, 5)?;
        }
        ObjectKind::ChangeRelation => {
            if unsigned(field(map, 3)?, 3)? > 2 {
                return Err(DecodeObjectError::Field(3));
            }
            for key in [4, 5, 6] {
                let ids = oid_values(field(map, key)?, key)?;
                ensure_sorted_unique_ids(&ids, key)?;
                if key != 6 && ids.is_empty() {
                    return Err(DecodeObjectError::Field(key));
                }
            }
            oid(field(map, 7)?, 7)?;
        }
        ObjectKind::ConflictResolution => {
            oid(field(map, 3)?, 3)?;
            typed_ref(field(map, 4)?, 4)?;
            fixed_bytes(field(map, 5)?, 5, 16)?;
            fixed_bytes(field(map, 6)?, 6, 16)?;
            if unsigned(field(map, 7)?, 7)? > 4 {
                return Err(DecodeObjectError::Field(7));
            }
            let refs = array(field(map, 8)?, 8)?;
            for value in refs {
                typed_ref(value, 8)?;
            }
            ensure_sorted_unique_values(refs, 8)?;
            wall_time(field(map, 9)?, 9)?;
            signature_set(
                field(map, 10)?,
                10,
                crate::SignaturePurpose::ConflictResolution,
            )?;
        }
        ObjectKind::ReviewEvidence
        | ObjectKind::ApprovalEvidence
        | ObjectKind::CiEvidence
        | ObjectKind::PolicyDecisionEvidence => {
            typed_ref(field(map, 3)?, 3)?;
            oid(field(map, 4)?, 4)?;
            oid(field(map, 5)?, 5)?;
            fixed_bytes(field(map, 6)?, 6, 16)?;
            fixed_bytes(field(map, 7)?, 7, 16)?;
            if unsigned(field(map, 8)?, 8)? > 3 {
                return Err(DecodeObjectError::Field(8));
            }
            canonical_map_bytes(field(map, 9)?, 9)?;
            let related = array(field(map, 10)?, 10)?;
            for value in related {
                typed_ref(value, 10)?;
            }
            ensure_sorted_unique_values(related, 10)?;
            wall_time(field(map, 11)?, 11)?;
            let purpose = match kind {
                ObjectKind::ReviewEvidence => crate::SignaturePurpose::ReviewEvidence,
                ObjectKind::ApprovalEvidence => crate::SignaturePurpose::ApprovalEvidence,
                ObjectKind::CiEvidence => crate::SignaturePurpose::CiEvidence,
                ObjectKind::PolicyDecisionEvidence => {
                    crate::SignaturePurpose::PolicyDecisionEvidence
                }
                _ => unreachable!(),
            };
            signature_set(field(map, 12)?, 12, purpose)?;
            if kind == ObjectKind::CiEvidence {
                if text(field(map, 13)?, 13)?.is_empty() {
                    return Err(DecodeObjectError::Field(13));
                }
                oid(field(map, 14)?, 14)?;
                if let Some(v) = optional_field(map, 15) {
                    oid(v, 15)?;
                }
            }
        }
        ObjectKind::ProjectionRules => {
            let version = unsigned(field(map, 3)?, 3)?;
            let previous = optional_field(map, 4);
            if (version == 0) == previous.is_some() {
                return Err(DecodeObjectError::Field(4));
            }
            if let Some(v) = previous {
                oid(v, 4)?;
            }
            for rule in array(field(map, 5)?, 5)? {
                let r = exact_map(rule, &[0, 1], &[], 5)?;
                if unsigned(field(r, 0)?, 5)? > 2 {
                    return Err(DecodeObjectError::Field(5));
                }
                canonical_map_bytes(field(r, 1)?, 5)?;
            }
            if !boolean(field(map, 6)?, 6)? {
                return Err(DecodeObjectError::Field(6));
            }
        }
        ObjectKind::ProjectionProof => {
            if unsigned(field(map, 3)?, 3)? != 0 {
                return Err(DecodeObjectError::Field(3));
            }
            for key in [4, 5, 6, 7] {
                oid(field(map, key)?, key)?;
            }
            if bytes(field(map, 8)?, 8)?.is_empty() {
                return Err(DecodeObjectError::Field(8));
            }
        }
        ObjectKind::BuildProvenance => {
            for key in [3, 4, 5] {
                oid(field(map, key)?, key)?;
            }
            let inputs = array(field(map, 6)?, 6)?;
            for value in inputs {
                typed_ref(value, 6)?;
            }
            ensure_sorted_unique_values(inputs, 6)?;
            let outputs = oid_values(field(map, 7)?, 7)?;
            ensure_sorted_unique_ids(&outputs, 7)?;
            canonical_map_bytes(field(map, 8)?, 8)?;
            wall_time(field(map, 9)?, 9)?;
            signature_set(
                field(map, 10)?,
                10,
                crate::SignaturePurpose::BuildProvenance,
            )?;
        }
        ObjectKind::Artifact => {
            if unsigned(field(map, 3)?, 3)? > 3 {
                return Err(DecodeObjectError::Field(3));
            }
            if unsigned(field(map, 4)?, 4)? > 1 {
                return Err(DecodeObjectError::Field(4));
            }
            if bytes(field(map, 5)?, 5)?.len() != 32 {
                return Err(DecodeObjectError::Field(5));
            }
            unsigned(field(map, 6)?, 6)?;
            if optional_field(map, 7).is_none() && optional_field(map, 8).is_none() {
                return Err(DecodeObjectError::Field(7));
            }
            if let Some(v) = optional_field(map, 7) {
                text(v, 7)?;
            }
            if let Some(v) = optional_field(map, 8) {
                oid(v, 8)?;
            }
        }
        ObjectKind::OperationPayload => {
            if unsigned(field(map, 3)?, 3)? > 3 {
                return Err(DecodeObjectError::Field(3));
            }
            let refs = array(field(map, 4)?, 4)?;
            for value in refs {
                typed_ref(value, 4)?;
            }
            ensure_sorted_unique_values(refs, 4)?;
            if unsigned(field(map, 5)?, 5)? != 0 {
                return Err(DecodeObjectError::Field(5));
            }
            canonical_map_bytes(field(map, 6)?, 6)?;
        }
        ObjectKind::View => {
            fixed_bytes(field(map, 3)?, 3, 16)?;
            fixed_bytes(field(map, 4)?, 4, 16)?;
            let policies = oid_values(field(map, 5)?, 5)?;
            ensure_sorted_unique_ids(&policies, 5)?;
            let lines = array(field(map, 6)?, 6)?;
            let mut line_ids = BTreeSet::new();
            for value in lines {
                let line = exact_map(value, &[0, 1, 2], &[], 6)?;
                fixed_bytes(field(line, 0)?, 6, 16)?;
                if !line_ids.insert(bytes(field(line, 0)?, 6)?.to_vec()) {
                    return Err(DecodeObjectError::Field(6));
                }
                unsigned(field(line, 1)?, 6)?;
                oid(field(line, 2)?, 6)?;
            }
            ensure_sorted_unique_values(lines, 6)?;
            oid(field(map, 7)?, 7)?;
            canonical_map_bytes(field(map, 8)?, 8)?;
            signature_set(field(map, 9)?, 9, crate::SignaturePurpose::View)?;
        }
        ObjectKind::Migration => {
            let source = unsigned(field(map, 3)?, 3)?;
            let target = unsigned(field(map, 4)?, 4)?;
            if source > 1 || target > 1 || source == target {
                return Err(DecodeObjectError::Field(4));
            }
            let mappings = array(field(map, 5)?, 5)?;
            if mappings.is_empty() {
                return Err(DecodeObjectError::Field(5));
            }
            let mut old = BTreeSet::new();
            let mut new = BTreeSet::new();
            for value in mappings {
                let m = exact_map(value, &[0, 1], &[], 5)?;
                let a = oid(field(m, 0)?, 5)?;
                let b = oid(field(m, 1)?, 5)?;
                if !old.insert(a) || !new.insert(b) {
                    return Err(DecodeObjectError::Field(5));
                }
            }
            ensure_sorted_unique_values(mappings, 5)?;
            oid(field(map, 6)?, 6)?;
            wall_time(field(map, 7)?, 7)?;
            signature_set(field(map, 8)?, 8, crate::SignaturePurpose::Migration)?;
        }
        ObjectKind::Ruleset => {
            if unsigned(field(map, 3)?, 3)? > 4 {
                return Err(DecodeObjectError::Field(3));
            }
            let version = unsigned(field(map, 4)?, 4)?;
            let previous = optional_field(map, 5);
            if (version == 0) == previous.is_some() {
                return Err(DecodeObjectError::Field(5));
            }
            if let Some(v) = previous {
                oid(v, 5)?;
            }
            canonical_map_bytes(field(map, 6)?, 6)?;
            let mut prior = None;
            for value in array(field(map, 7)?, 7)? {
                let n = unsigned(value, 7)?;
                if !matches!(n, 20..=23) || prior.is_some_and(|p| p >= n) {
                    return Err(DecodeObjectError::Field(7));
                }
                prior = Some(n);
            }
        }
    }
    Ok(())
}

fn references(d: &DecodedObject) -> Result<Vec<ReferenceEdge>, DecodeObjectError> {
    let map = as_map(&d.value)?;
    let mut out = Vec::new();
    // Every policy reference is an edge to an exact policy version.
    for (key, role) in policy_fields(d.kind) {
        if let Some(v) = map.iter().find(|(k, _)| k == key).map(|(_, v)| v) {
            let p = as_map(v)?;
            out.push(ReferenceEdge {
                role: *role,
                expected_kind: Some(ObjectKind::Policy),
                id: oid(field(p, 1)?, *key)?,
            });
        }
    }
    macro_rules! add {
        ($key:expr, $role:expr, $expected:expr) => {
            if let Some(v) = map
                .iter()
                .find(|(k, _)| *k == $key)
                .map(|(_, v)| v)
                .filter(|value| !matches!(value, Value::Null))
            {
                out.push(ReferenceEdge {
                    role: $role,
                    expected_kind: $expected,
                    id: oid(v, $key)?,
                });
            }
        };
    }
    match d.kind {
        ObjectKind::Blob => {
            if let Some(chunks) = map.iter().find(|(k, _)| *k == 5).map(|(_, v)| v) {
                for chunk in array(chunks, 5)? {
                    let chunk = as_map(chunk)?;
                    out.push(ReferenceEdge {
                        role: ReferenceRole::BlobChunk,
                        expected_kind: Some(ObjectKind::Chunk),
                        id: oid(field(chunk, 0)?, 5)?,
                    });
                }
            }
        }
        ObjectKind::Manifest => {
            for entry in array(field(map, 3)?, 3)? {
                let entry = as_map(entry)?;
                let kind = unsigned(field(entry, 1)?, 3)?;
                let expected = match kind {
                    0 | 2 => ObjectKind::Blob,
                    1 => ObjectKind::Manifest,
                    3 => ObjectKind::Subproject,
                    4 => ObjectKind::SecretRef,
                    _ => return Err(DecodeObjectError::Field(3)),
                };
                out.push(ReferenceEdge {
                    role: ReferenceRole::ManifestEntryTarget,
                    expected_kind: Some(expected),
                    id: oid(field(entry, 2)?, 3)?,
                });
                let p = as_map(field(entry, 4)?)?;
                out.push(ReferenceEdge {
                    role: ReferenceRole::ManifestEntryPolicy,
                    expected_kind: Some(ObjectKind::Policy),
                    id: oid(field(p, 1)?, 3)?,
                });
            }
        }
        ObjectKind::Snapshot => {
            add!(
                3,
                ReferenceRole::SnapshotRootManifest,
                Some(ObjectKind::Manifest)
            );
            add!(10, ReferenceRole::SnapshotMessage, Some(ObjectKind::Blob));
            for value in array(field(map, 4)?, 4)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::SnapshotParent,
                    expected_kind: Some(ObjectKind::Snapshot),
                    id: oid(value, 4)?,
                });
            }
        }
        ObjectKind::ChangeRevision => {
            add!(
                4,
                ReferenceRole::ChangePreviousRevision,
                Some(ObjectKind::ChangeRevision)
            );
            add!(5, ReferenceRole::ChangeTitle, Some(ObjectKind::Blob));
            add!(6, ReferenceRole::ChangeDescription, Some(ObjectKind::Blob));
            add!(
                7,
                ReferenceRole::ChangeBaseSnapshot,
                Some(ObjectKind::Snapshot)
            );
            add!(
                8,
                ReferenceRole::ChangeCurrentSnapshot,
                Some(ObjectKind::Snapshot)
            );
        }
        ObjectKind::LineState => {
            add!(
                5,
                ReferenceRole::LineHeadSnapshot,
                Some(ObjectKind::Snapshot)
            );
            add!(
                7,
                ReferenceRole::LinePreviousState,
                Some(ObjectKind::LineState)
            );
            add!(
                12,
                ReferenceRole::LineTransactionOperation,
                Some(ObjectKind::Operation)
            );
        }
        ObjectKind::Subproject => add!(6, ReferenceRole::SubprojectNativeProjection, None),
        ObjectKind::SecretRef => add!(9, ReferenceRole::SecretDevelopmentValue, None),
        ObjectKind::Release => {
            add!(
                5,
                ReferenceRole::ReleaseSourceSnapshot,
                Some(ObjectKind::Snapshot)
            );
            add!(
                7,
                ReferenceRole::ReleaseProjectionRules,
                Some(ObjectKind::ProjectionRules)
            );
            add!(
                8,
                ReferenceRole::ReleaseProjectedRoot,
                Some(ObjectKind::Manifest)
            );
            add!(
                9,
                ReferenceRole::ReleaseProjectionProof,
                Some(ObjectKind::ProjectionProof)
            );
            add!(11, ReferenceRole::ReleaseNotes, Some(ObjectKind::Blob));
            for (key, role) in [
                (12, ReferenceRole::ReleaseBuildProvenance),
                (13, ReferenceRole::ReleaseArtifact),
                (14, ReferenceRole::ReleasePolicyEvidence),
            ] {
                let expected = match key {
                    12 => ObjectKind::BuildProvenance,
                    13 => ObjectKind::Artifact,
                    _ => ObjectKind::PolicyDecisionEvidence,
                };
                for value in array(field(map, key)?, key)? {
                    out.push(ReferenceEdge {
                        role,
                        expected_kind: Some(expected),
                        id: oid(value, key)?,
                    });
                }
            }
        }
        ObjectKind::Policy => {
            add!(
                5,
                ReferenceRole::PolicyPreviousVersion,
                Some(ObjectKind::Policy)
            );
            add!(
                12,
                ReferenceRole::PolicyKeyEnvelopeSet,
                Some(ObjectKind::KeyEnvelopeSet)
            );
            for value in array(field(map, 10)?, 10)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::PolicyDeclassificationRequirement,
                    expected_kind: Some(ObjectKind::PolicyDecisionEvidence),
                    id: oid(value, 10)?,
                });
            }
        }
        ObjectKind::Conflict => {
            for (k, role) in [
                (3, ReferenceRole::ConflictBase),
                (4, ReferenceRole::ConflictLeft),
                (5, ReferenceRole::ConflictRight),
            ] {
                let r = as_map(field(map, k)?)?;
                out.push(ReferenceEdge {
                    role,
                    expected_kind: Some(
                        ObjectKind::try_from(unsigned(field(r, 0)?, k)?)
                            .map_err(DecodeObjectError::Kind)?,
                    ),
                    id: oid(field(r, 1)?, k)?,
                });
            }
        }
        ObjectKind::Marker => {
            let r = as_map(field(map, 4)?)?;
            out.push(ReferenceEdge {
                role: ReferenceRole::MarkerTarget,
                expected_kind: Some(
                    ObjectKind::try_from(unsigned(field(r, 0)?, 4)?)
                        .map_err(DecodeObjectError::Kind)?,
                ),
                id: oid(field(r, 1)?, 4)?,
            });
        }
        ObjectKind::Operation => {
            for value in array(field(map, 3)?, 3)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::OperationParent,
                    expected_kind: Some(ObjectKind::Operation),
                    id: oid(value, 3)?,
                });
            }
            for value in array(field(map, 9)?, 9)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::OperationInversePayload,
                    expected_kind: Some(ObjectKind::OperationPayload),
                    id: oid(value, 9)?,
                });
            }
            for action in array(field(map, 8)?, 8)? {
                let action = as_map(action)?;
                match unsigned(field(action, 0)?, 8)? {
                    0 => {
                        for (key, role) in [
                            (1, ReferenceRole::OperationBefore),
                            (2, ReferenceRole::OperationAfter),
                        ] {
                            if let Some(reference) = action
                                .iter()
                                .find(|(actual, _)| *actual == key)
                                .map(|(_, v)| v)
                            {
                                let reference = as_map(reference)?;
                                out.push(ReferenceEdge {
                                    role,
                                    expected_kind: Some(
                                        ObjectKind::try_from(unsigned(field(reference, 0)?, 8)?)
                                            .map_err(DecodeObjectError::Kind)?,
                                    ),
                                    id: oid(field(reference, 1)?, 8)?,
                                });
                            }
                        }
                    }
                    1 => {
                        let declaration = as_map(field(action, 3)?)?;
                        for (key, role) in [
                            (0, ReferenceRole::OperationLinePolicy),
                            (6, ReferenceRole::OperationLineIntegrationPolicy),
                            (7, ReferenceRole::OperationLineApprovalPolicy),
                            (8, ReferenceRole::OperationLineReleasePolicy),
                            (9, ReferenceRole::OperationLineVisibilityPolicy),
                        ] {
                            let policy = as_map(field(declaration, key)?)?;
                            out.push(ReferenceEdge {
                                role,
                                expected_kind: Some(ObjectKind::Policy),
                                id: oid(field(policy, 1)?, 8)?,
                            });
                        }
                        out.push(ReferenceEdge {
                            role: ReferenceRole::OperationLineHeadSnapshot,
                            expected_kind: Some(ObjectKind::Snapshot),
                            id: oid(field(declaration, 3)?, 8)?,
                        });
                        if let Some(previous) = declaration
                            .iter()
                            .find(|(key, _)| *key == 5)
                            .map(|(_, value)| value)
                        {
                            out.push(ReferenceEdge {
                                role: ReferenceRole::OperationLinePreviousState,
                                expected_kind: Some(ObjectKind::LineState),
                                id: oid(previous, 8)?,
                            });
                        }
                    }
                    _ => return Err(DecodeObjectError::Field(8)),
                }
            }
            add!(
                10,
                ReferenceRole::OperationPublicEnvelope,
                Some(ObjectKind::OperationPayload)
            );
            add!(
                11,
                ReferenceRole::OperationPrivatePayload,
                Some(ObjectKind::OperationPayload)
            );
        }
        ObjectKind::RepositoryRoot => {
            add!(
                4,
                ReferenceRole::RepositoryRootPolicy,
                Some(ObjectKind::Policy)
            );
            for value in array(field(map, 5)?, 5)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::RepositoryTrustedIdentity,
                    expected_kind: Some(ObjectKind::Identity),
                    id: oid(value, 5)?,
                });
            }
            add!(
                6,
                ReferenceRole::RepositoryBootstrapKeyEnvelopeSet,
                Some(ObjectKind::KeyEnvelopeSet)
            );
            add!(
                7,
                ReferenceRole::RepositoryGenesisOperation,
                Some(ObjectKind::Operation)
            );
            for value in array(field(map, 8)?, 8)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::RepositoryInitialLineState,
                    expected_kind: Some(ObjectKind::LineState),
                    id: oid(value, 8)?,
                });
            }
        }
        ObjectKind::Identity => {
            add!(
                6,
                ReferenceRole::IdentityPrevious,
                Some(ObjectKind::Identity)
            );
            for key in [7, 8] {
                for record in array(field(map, key)?, key)? {
                    let record = as_map(record)?;
                    if let Some(value) = optional_field(record, 3) {
                        out.push(ReferenceEdge {
                            role: ReferenceRole::IdentityKeyNotBefore,
                            expected_kind: Some(ObjectKind::Operation),
                            id: oid(value, key)?,
                        });
                    }
                    if let Some(value) = optional_field(record, 4) {
                        out.push(ReferenceEdge {
                            role: ReferenceRole::IdentityKeyNotAfter,
                            expected_kind: Some(ObjectKind::Operation),
                            id: oid(value, key)?,
                        });
                    }
                }
            }
            add!(
                11,
                ReferenceRole::IdentityActivationOperation,
                Some(ObjectKind::Operation)
            );
            add!(
                12,
                ReferenceRole::IdentityNotAfterOperation,
                Some(ObjectKind::Operation)
            );
        }
        ObjectKind::GroupMembership => {
            add!(
                6,
                ReferenceRole::MembershipPrevious,
                Some(ObjectKind::GroupMembership)
            );
            add!(
                10,
                ReferenceRole::MembershipActivationOperation,
                Some(ObjectKind::Operation)
            );
            add!(
                11,
                ReferenceRole::MembershipNotAfterOperation,
                Some(ObjectKind::Operation)
            );
        }
        ObjectKind::ChangeRelation => {
            for (key, role, expected) in [
                (
                    4,
                    ReferenceRole::ChangeRelationSource,
                    ObjectKind::ChangeRevision,
                ),
                (
                    5,
                    ReferenceRole::ChangeRelationResult,
                    ObjectKind::ChangeRevision,
                ),
                (
                    6,
                    ReferenceRole::ChangeRelationProvenance,
                    ObjectKind::Snapshot,
                ),
            ] {
                for value in array(field(map, key)?, key)? {
                    out.push(ReferenceEdge {
                        role,
                        expected_kind: Some(expected),
                        id: oid(value, key)?,
                    });
                }
            }
            add!(
                7,
                ReferenceRole::ChangeRelationOperation,
                Some(ObjectKind::Operation)
            );
        }
        ObjectKind::ConflictResolution => {
            add!(
                3,
                ReferenceRole::ResolutionConflict,
                Some(ObjectKind::Conflict)
            );
            let resolved = as_map(field(map, 4)?)?;
            out.push(ReferenceEdge {
                role: ReferenceRole::ResolutionResult,
                expected_kind: Some(
                    ObjectKind::try_from(unsigned(field(resolved, 0)?, 4)?)
                        .map_err(DecodeObjectError::Kind)?,
                ),
                id: oid(field(resolved, 1)?, 4)?,
            });
            for value in array(field(map, 8)?, 8)? {
                let r = as_map(value)?;
                out.push(ReferenceEdge {
                    role: ReferenceRole::ResolutionProvenance,
                    expected_kind: Some(
                        ObjectKind::try_from(unsigned(field(r, 0)?, 8)?)
                            .map_err(DecodeObjectError::Kind)?,
                    ),
                    id: oid(field(r, 1)?, 8)?,
                });
            }
        }
        ObjectKind::ReviewEvidence
        | ObjectKind::ApprovalEvidence
        | ObjectKind::CiEvidence
        | ObjectKind::PolicyDecisionEvidence => {
            let target = as_map(field(map, 3)?)?;
            out.push(ReferenceEdge {
                role: ReferenceRole::EvidenceTarget,
                expected_kind: Some(
                    ObjectKind::try_from(unsigned(field(target, 0)?, 3)?)
                        .map_err(DecodeObjectError::Kind)?,
                ),
                id: oid(field(target, 1)?, 3)?,
            });
            add!(
                4,
                ReferenceRole::EvidenceSnapshot,
                Some(ObjectKind::Snapshot)
            );
            add!(5, ReferenceRole::EvidenceRuleset, Some(ObjectKind::Ruleset));
            for value in array(field(map, 10)?, 10)? {
                let r = as_map(value)?;
                out.push(ReferenceEdge {
                    role: ReferenceRole::EvidenceRelated,
                    expected_kind: Some(
                        ObjectKind::try_from(unsigned(field(r, 0)?, 10)?)
                            .map_err(DecodeObjectError::Kind)?,
                    ),
                    id: oid(field(r, 1)?, 10)?,
                });
            }
            if d.kind == ObjectKind::CiEvidence {
                add!(
                    14,
                    ReferenceRole::CiRunnerIdentity,
                    Some(ObjectKind::Identity)
                );
                add!(
                    15,
                    ReferenceRole::CiBuildProvenance,
                    Some(ObjectKind::BuildProvenance)
                );
            }
        }
        ObjectKind::ProjectionRules => add!(
            4,
            ReferenceRole::ProjectionRulesPrevious,
            Some(ObjectKind::ProjectionRules)
        ),
        ObjectKind::ProjectionProof => {
            add!(
                4,
                ReferenceRole::ProjectionSourceSnapshot,
                Some(ObjectKind::Snapshot)
            );
            add!(
                5,
                ReferenceRole::ProjectionRules,
                Some(ObjectKind::ProjectionRules)
            );
            add!(
                6,
                ReferenceRole::ProjectionAudiencePolicy,
                Some(ObjectKind::Policy)
            );
            add!(
                7,
                ReferenceRole::ProjectionManifest,
                Some(ObjectKind::Manifest)
            );
        }
        ObjectKind::BuildProvenance => {
            add!(3, ReferenceRole::BuildSnapshot, Some(ObjectKind::Snapshot));
            add!(4, ReferenceRole::BuildRuleset, Some(ObjectKind::Ruleset));
            add!(5, ReferenceRole::BuildIdentity, Some(ObjectKind::Identity));
            for value in array(field(map, 6)?, 6)? {
                let r = as_map(value)?;
                out.push(ReferenceEdge {
                    role: ReferenceRole::BuildInput,
                    expected_kind: Some(
                        ObjectKind::try_from(unsigned(field(r, 0)?, 6)?)
                            .map_err(DecodeObjectError::Kind)?,
                    ),
                    id: oid(field(r, 1)?, 6)?,
                });
            }
            for value in array(field(map, 7)?, 7)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::BuildOutput,
                    expected_kind: Some(ObjectKind::Artifact),
                    id: oid(value, 7)?,
                });
            }
        }
        ObjectKind::Artifact => add!(8, ReferenceRole::ArtifactBlob, Some(ObjectKind::Blob)),
        ObjectKind::OperationPayload => {
            for value in array(field(map, 4)?, 4)? {
                let r = as_map(value)?;
                out.push(ReferenceEdge {
                    role: ReferenceRole::PayloadReference,
                    expected_kind: Some(
                        ObjectKind::try_from(unsigned(field(r, 0)?, 4)?)
                            .map_err(DecodeObjectError::Kind)?,
                    ),
                    id: oid(field(r, 1)?, 4)?,
                });
            }
        }
        ObjectKind::View => {
            for value in array(field(map, 5)?, 5)? {
                out.push(ReferenceEdge {
                    role: ReferenceRole::ViewPolicy,
                    expected_kind: Some(ObjectKind::Policy),
                    id: oid(value, 5)?,
                });
            }
            for value in array(field(map, 6)?, 6)? {
                let line = as_map(value)?;
                out.push(ReferenceEdge {
                    role: ReferenceRole::ViewLineState,
                    expected_kind: Some(ObjectKind::LineState),
                    id: oid(field(line, 2)?, 6)?,
                });
            }
            add!(7, ReferenceRole::ViewManifest, Some(ObjectKind::Manifest));
        }
        ObjectKind::Migration => {
            for value in array(field(map, 5)?, 5)? {
                let m = as_map(value)?;
                out.push(ReferenceEdge {
                    role: ReferenceRole::MigrationOldObject,
                    expected_kind: None,
                    id: oid(field(m, 0)?, 5)?,
                });
                out.push(ReferenceEdge {
                    role: ReferenceRole::MigrationNewObject,
                    expected_kind: None,
                    id: oid(field(m, 1)?, 5)?,
                });
            }
            add!(
                6,
                ReferenceRole::MigrationToolIdentity,
                Some(ObjectKind::Identity)
            );
        }
        ObjectKind::Ruleset => add!(5, ReferenceRole::RulesetPrevious, Some(ObjectKind::Ruleset)),
        _ => {}
    }
    // Schema-specific extraction should never surface the same semantic edge
    // twice. Keep this defensive boundary here so graph walkers cannot double
    // count authorization or reachability edges if field routing evolves.
    let mut unique = Vec::with_capacity(out.len());
    for edge in out {
        if !unique.contains(&edge) {
            unique.push(edge);
        }
    }
    Ok(unique)
}
fn policy_fields(kind: ObjectKind) -> &'static [(u64, ReferenceRole)] {
    match kind {
        ObjectKind::ChangeRevision => &[
            (2, ReferenceRole::ObjectPolicy),
            (14, ReferenceRole::ReviewPolicy),
            (15, ReferenceRole::LandingPolicy),
        ],
        ObjectKind::LineState => &[
            (2, ReferenceRole::ObjectPolicy),
            (8, ReferenceRole::IntegrationPolicy),
            (9, ReferenceRole::ApprovalPolicy),
            (10, ReferenceRole::ReleasePolicy),
            (11, ReferenceRole::VisibilityPolicy),
        ],
        ObjectKind::Release => &[
            (2, ReferenceRole::ObjectPolicy),
            (6, ReferenceRole::AudiencePolicy),
        ],
        _ => &[(2, ReferenceRole::ObjectPolicy)],
    }
}
fn as_map(v: &Value) -> Result<&[(u64, Value)], DecodeObjectError> {
    if let Value::Map(v) = v {
        Ok(v)
    } else {
        Err(DecodeObjectError::NotMap)
    }
}
fn field(m: &[(u64, Value)], k: u64) -> Result<&Value, DecodeObjectError> {
    m.iter()
        .find(|(key, _)| *key == k)
        .map(|(_, v)| v)
        .ok_or(DecodeObjectError::Field(k))
}
fn unsigned(v: &Value, k: u64) -> Result<u64, DecodeObjectError> {
    if let Value::Unsigned(n) = v {
        Ok(*n)
    } else {
        Err(DecodeObjectError::Field(k))
    }
}
fn bytes(v: &Value, k: u64) -> Result<&[u8], DecodeObjectError> {
    if let Value::Bytes(b) = v {
        Ok(b)
    } else {
        Err(DecodeObjectError::Field(k))
    }
}
fn text(v: &Value, k: u64) -> Result<&str, DecodeObjectError> {
    if let Value::Text(s) = v {
        Ok(s)
    } else {
        Err(DecodeObjectError::Field(k))
    }
}
fn array(v: &Value, k: u64) -> Result<&[Value], DecodeObjectError> {
    if let Value::Array(a) = v {
        Ok(a)
    } else {
        Err(DecodeObjectError::Field(k))
    }
}
fn fixed_bytes(v: &Value, k: u64, n: usize) -> Result<(), DecodeObjectError> {
    if bytes(v, k)?.len() == n {
        Ok(())
    } else {
        Err(DecodeObjectError::Field(k))
    }
}
fn oid(v: &Value, k: u64) -> Result<ObjectId, DecodeObjectError> {
    ObjectId::from_bytes(bytes(v, k)?).map_err(|_| DecodeObjectError::ObjectId(k))
}
fn oid_array(v: &Value, k: u64) -> Result<(), DecodeObjectError> {
    for x in array(v, k)? {
        oid(x, k)?;
    }
    Ok(())
}
fn policy_ref(v: &Value, k: u64) -> Result<(), DecodeObjectError> {
    let m = exact_map(v, &[0, 1], &[], k)?;
    fixed_bytes(field(m, 0)?, k, 16)?;
    oid(field(m, 1)?, k)?;
    Ok(())
}
fn typed_ref(v: &Value, k: u64) -> Result<(), DecodeObjectError> {
    typed_ref_kind(v, k).map(|_| ())
}
fn typed_ref_kind(v: &Value, k: u64) -> Result<ObjectKind, DecodeObjectError> {
    let m = exact_map(v, &[0, 1], &[], k)?;
    let kind = ObjectKind::try_from(unsigned(field(m, 0)?, k)?).map_err(DecodeObjectError::Kind)?;
    oid(field(m, 1)?, k)?;
    Ok(kind)
}
fn oid_values(v: &Value, k: u64) -> Result<Vec<ObjectId>, DecodeObjectError> {
    array(v, k)?.iter().map(|value| oid(value, k)).collect()
}
fn ensure_unique_ids(ids: &[ObjectId], key: u64) -> Result<(), DecodeObjectError> {
    for (index, id) in ids.iter().enumerate() {
        if ids[..index].contains(id) {
            return Err(DecodeObjectError::Field(key));
        }
    }
    Ok(())
}
fn ensure_sorted_unique_ids(ids: &[ObjectId], key: u64) -> Result<(), DecodeObjectError> {
    if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        Err(DecodeObjectError::Field(key))
    } else {
        Ok(())
    }
}
fn optional_field(map: &[(u64, Value)], key: u64) -> Option<&Value> {
    map.iter()
        .find(|(actual, _)| *actual == key)
        .map(|(_, value)| value)
}
fn boolean(value: &Value, key: u64) -> Result<bool, DecodeObjectError> {
    if let Value::Bool(value) = value {
        Ok(*value)
    } else {
        Err(DecodeObjectError::Field(key))
    }
}
fn fixed_nonzero_bytes(value: &Value, key: u64, length: usize) -> Result<(), DecodeObjectError> {
    fixed_bytes(value, key, length)?;
    if bytes(value, key)?.iter().all(|byte| *byte == 0) {
        Err(DecodeObjectError::Field(key))
    } else {
        Ok(())
    }
}
fn principal(value: &Value, key: u64) -> Result<(), DecodeObjectError> {
    let map = exact_map(value, &[0, 1], &[], key)?;
    if unsigned(field(map, 0)?, key)? > 3 || bytes(field(map, 1)?, key)?.is_empty() {
        Err(DecodeObjectError::Field(key))
    } else {
        Ok(())
    }
}
fn public_key(value: &Value, key: u64) -> Result<(u64, Vec<u8>), DecodeObjectError> {
    let map = exact_map(value, &[0, 1, 2], &[3, 4], key)?;
    let algorithm = unsigned(field(map, 0)?, key)?;
    if algorithm > 1 {
        return Err(DecodeObjectError::Field(key));
    }
    fixed_nonzero_bytes(field(map, 1)?, key, 32)?;
    if bytes(field(map, 2)?, key)?.len() != 32 {
        return Err(DecodeObjectError::Field(key));
    }
    for optional in [3, 4] {
        if let Some(value) = optional_field(map, optional) {
            oid(value, key)?;
        }
    }
    Ok((algorithm, bytes(field(map, 1)?, key)?.to_vec()))
}
fn canonical_map_bytes(value: &Value, key: u64) -> Result<(), DecodeObjectError> {
    let encoded = bytes(value, key)?;
    match crate::decode_canonical(encoded, CanonicalLimits::metadata()) {
        Ok(Value::Map(_)) => Ok(()),
        _ => Err(DecodeObjectError::Field(key)),
    }
}
fn signature_set(
    value: &Value,
    key: u64,
    purpose: crate::SignaturePurpose,
) -> Result<(), DecodeObjectError> {
    let values = array(value, key)?;
    if values.is_empty() {
        return Err(DecodeObjectError::Field(key));
    }
    for value in values {
        signature(value, key, purpose)?;
    }
    ensure_sorted_unique_values(values, key)
}
fn ensure_sorted_unique_values(values: &[Value], key: u64) -> Result<(), DecodeObjectError> {
    let encoded: Result<Vec<_>, _> = values.iter().map(Value::encode).collect();
    let encoded = encoded?;
    if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
        Err(DecodeObjectError::Field(key))
    } else {
        Ok(())
    }
}
fn line_advance(value: &Value, key: u64) -> Result<(), DecodeObjectError> {
    let map = exact_map(value, &[0, 1, 2, 3, 4, 6, 7, 8, 9], &[5], key)?;
    policy_ref(field(map, 0)?, key)?;
    fixed_bytes(field(map, 1)?, key, 16)?;
    text(field(map, 2)?, key)?;
    oid(field(map, 3)?, key)?;
    let generation = unsigned(field(map, 4)?, key)?;
    let previous = map.iter().find(|(actual, _)| *actual == 5).map(|(_, v)| v);
    if (generation == 0) == previous.is_some() {
        return Err(DecodeObjectError::Field(key));
    }
    if let Some(previous) = previous {
        oid(previous, key)?;
    }
    for policy_key in [6, 7, 8, 9] {
        policy_ref(field(map, policy_key)?, key)?;
    }
    Ok(())
}
fn exact_map<'a>(
    value: &'a Value,
    required: &[u64],
    optional: &[u64],
    field_key: u64,
) -> Result<&'a [(u64, Value)], DecodeObjectError> {
    let map = as_map(value)?;
    if required
        .iter()
        .any(|key| !map.iter().any(|(actual, _)| actual == key))
        || map
            .iter()
            .any(|(key, _)| !required.contains(key) && !optional.contains(key))
    {
        return Err(DecodeObjectError::Field(field_key));
    }
    Ok(map)
}
fn wall_time(value: &Value, key: u64) -> Result<(), DecodeObjectError> {
    let map = exact_map(value, &[0, 1], &[], key)?;
    for field_key in [0, 1] {
        if !matches!(
            field(map, field_key)?,
            Value::Signed(_) | Value::Unsigned(_)
        ) {
            return Err(DecodeObjectError::Field(key));
        }
    }
    Ok(())
}
fn signature(
    value: &Value,
    key: u64,
    expected_purpose: crate::SignaturePurpose,
) -> Result<(), DecodeObjectError> {
    let map = exact_map(value, &[0, 1, 2, 3, 4], &[], key)?;
    crate::SignatureAlgorithm::try_from(unsigned(field(map, 0)?, key)?)
        .map_err(|_| DecodeObjectError::Field(key))?;
    fixed_bytes(field(map, 1)?, key, 16)?;
    fixed_bytes(field(map, 2)?, key, 32)?;
    fixed_bytes(field(map, 3)?, key, 64)?;
    if bytes(field(map, 2)?, key)?.iter().all(|byte| *byte == 0)
        || bytes(field(map, 3)?, key)?.iter().all(|byte| *byte == 0)
    {
        return Err(DecodeObjectError::Field(key));
    }
    let purpose = crate::SignaturePurpose::try_from(unsigned(field(map, 4)?, key)?)
        .map_err(|_| DecodeObjectError::Field(key))?;
    if purpose != expected_purpose {
        return Err(DecodeObjectError::Field(key));
    }
    Ok(())
}
