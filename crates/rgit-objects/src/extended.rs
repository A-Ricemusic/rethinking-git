//! Schema-0 registry objects 14 through 31.
//!
//! This module deliberately contains logical objects only. Storage encryption
//! envelopes are mutable physical records and therefore have no object kind.

use std::collections::BTreeSet;

use unicode_normalization::UnicodeNormalization;

use crate::{
    ActorId, CanonicalObject, DeviceId, HashAlgorithm, LineState, ObjectError, ObjectId,
    ObjectKind, Operation, Policy, PolicyRef, Principal, Signature, SignaturePurpose, SignedObject,
    TypedObjectRef, Value, WallTime,
};

const MAX_REGISTRY_ITEMS: usize = 65_536;

fn oid(id: &ObjectId) -> Value {
    Value::Bytes(id.to_bytes())
}
fn oid_array(ids: &[ObjectId]) -> Value {
    Value::Array(ids.iter().map(oid).collect())
}
fn header(kind: ObjectKind, policy: Option<&PolicyRef>) -> Vec<(u64, Value)> {
    let mut map = vec![(0, Value::Unsigned(kind as u64)), (1, Value::Unsigned(0))];
    if let Some(policy) = policy {
        map.push((2, policy.value()));
    }
    map
}
fn optional(map: &mut Vec<(u64, Value)>, key: u64, value: Option<Value>) {
    if let Some(value) = value {
        map.push((key, value));
    }
}
fn nfc(text: &str) -> Result<(), ObjectError> {
    if text.nfc().eq(text.chars()) {
        Ok(())
    } else {
        Err(ObjectError::NonNormalizedText)
    }
}
fn validate_canonical_map(bytes: &[u8], message: &'static str) -> Result<(), ObjectError> {
    match crate::decode_canonical(bytes, crate::CanonicalLimits::metadata()) {
        Ok(Value::Map(_)) => Ok(()),
        _ => Err(ObjectError::Invalid(message)),
    }
}
fn sorted_unique_ids(ids: &[ObjectId], what: &'static str) -> Result<(), ObjectError> {
    if ids.len() > MAX_REGISTRY_ITEMS {
        return Err(ObjectError::Invalid(
            "object reference set exceeds schema limit",
        ));
    }
    if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        Err(ObjectError::Invalid(what))
    } else {
        Ok(())
    }
}
fn sorted_unique_refs(values: &[TypedObjectRef], what: &'static str) -> Result<(), ObjectError> {
    if values.len() > MAX_REGISTRY_ITEMS {
        return Err(ObjectError::Invalid(
            "typed reference set exceeds schema limit",
        ));
    }
    let encoded = values
        .iter()
        .map(|value| value.value().encode())
        .collect::<Result<Vec<_>, _>>()?;
    if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
        Err(ObjectError::Invalid(what))
    } else {
        Ok(())
    }
}
fn signatures(values: &[Signature], purpose: SignaturePurpose) -> Result<(), ObjectError> {
    if values.is_empty() {
        return Err(ObjectError::Invalid("signed object requires signatures"));
    }
    if values
        .iter()
        .any(|signature| signature.purpose() != purpose)
    {
        return Err(ObjectError::Invalid(
            "signature purpose does not match object kind",
        ));
    }
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ObjectError::Invalid("signatures must be sorted and unique"));
    }
    Ok(())
}
fn principal_value(principal: &Principal) -> Value {
    Value::Map(vec![
        (0, Value::Unsigned(principal.kind as u64)),
        (1, Value::Bytes(principal.identifier.clone())),
    ])
}

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);
        impl $name {
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
            fn value(self) -> Value {
                Value::Bytes(self.0.to_vec())
            }
        }
    };
}
stable_id!(RepositoryId);
stable_id!(GroupId);
stable_id!(MembershipId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum FilesystemProfile {
    Portable = 0,
    Native = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRoot {
    pub repository_id: RepositoryId,
    pub root_policy: ObjectId,
    pub trusted_identities: Vec<ObjectId>,
    pub bootstrap_key_envelope_set: ObjectId,
    pub genesis_operation: ObjectId,
    pub initial_line_states: Vec<ObjectId>,
    pub filesystem_profile: FilesystemProfile,
    pub signatures: Vec<Signature>,
}
impl RepositoryRoot {
    pub fn try_new(value: Self) -> Result<Self, ObjectError> {
        value.validate()?;
        Ok(value)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut map = header(Self::KIND, None);
        map.extend([
            (3, self.repository_id.value()),
            (4, oid(&self.root_policy)),
            (5, oid_array(&self.trusted_identities)),
            (6, oid(&self.bootstrap_key_envelope_set)),
            (7, oid(&self.genesis_operation)),
            (8, oid_array(&self.initial_line_states)),
            (9, Value::Unsigned(self.filesystem_profile as u64)),
        ]);
        map
    }
}
impl CanonicalObject for RepositoryRoot {
    const KIND: ObjectKind = ObjectKind::RepositoryRoot;
    fn validate(&self) -> Result<(), ObjectError> {
        sorted_unique_ids(
            &self.trusted_identities,
            "trusted identities must be sorted and unique",
        )?;
        sorted_unique_ids(
            &self.initial_line_states,
            "initial line states must be sorted and unique",
        )?;
        if self.trusted_identities.is_empty() || self.initial_line_states.is_empty() {
            return Err(ObjectError::Invalid(
                "repository bootstrap sets must not be empty",
            ));
        }
        signatures(&self.signatures, SignaturePurpose::RepositoryRoot)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut map = self.unsigned_map();
        map.push((
            10,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(map))
    }
}
impl SignedObject for RepositoryRoot {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum IdentitySubjectKind {
    Actor = 0,
    Device = 1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum IdentityStatus {
    Active = 0,
    Suspended = 1,
    Revoked = 2,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum PublicKeyAlgorithm {
    Ed25519 = 0,
    X25519 = 1,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKeyRecord {
    pub algorithm: PublicKeyAlgorithm,
    pub key_id: [u8; 32],
    pub public_key: Vec<u8>,
    pub not_before: Option<ObjectId>,
    pub not_after: Option<ObjectId>,
}
impl PublicKeyRecord {
    fn value(&self) -> Value {
        let mut map = vec![
            (0, Value::Unsigned(self.algorithm as u64)),
            (1, Value::Bytes(self.key_id.to_vec())),
            (2, Value::Bytes(self.public_key.clone())),
        ];
        optional(&mut map, 3, self.not_before.as_ref().map(oid));
        optional(&mut map, 4, self.not_after.as_ref().map(oid));
        Value::Map(map)
    }
    fn validate(&self) -> Result<(), ObjectError> {
        if self.key_id == [0; 32] || self.public_key.len() != 32 {
            Err(ObjectError::Invalid(
                "identity key record is empty or reserved",
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub policy_ref: Option<PolicyRef>,
    pub subject_kind: IdentitySubjectKind,
    pub subject: [u8; 16],
    pub version: u64,
    pub previous: Option<ObjectId>,
    pub signing_keys: Vec<PublicKeyRecord>,
    pub encryption_keys: Vec<PublicKeyRecord>,
    pub issuer: ActorId,
    pub status: IdentityStatus,
    pub activation_operation: Option<ObjectId>,
    pub not_after_operation: Option<ObjectId>,
    pub signatures: Vec<Signature>,
}
impl Identity {
    pub fn try_new(value: Self) -> Result<Self, ObjectError> {
        value.validate()?;
        Ok(value)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut map = header(Self::KIND, self.policy_ref.as_ref());
        map.extend([
            (3, Value::Unsigned(self.subject_kind as u64)),
            (4, Value::Bytes(self.subject.to_vec())),
            (5, Value::Unsigned(self.version)),
            (
                7,
                Value::Array(
                    self.signing_keys
                        .iter()
                        .map(PublicKeyRecord::value)
                        .collect(),
                ),
            ),
            (
                8,
                Value::Array(
                    self.encryption_keys
                        .iter()
                        .map(PublicKeyRecord::value)
                        .collect(),
                ),
            ),
            (9, self.issuer.into()),
            (10, Value::Unsigned(self.status as u64)),
        ]);
        optional(&mut map, 6, self.previous.as_ref().map(oid));
        optional(&mut map, 11, self.activation_operation.as_ref().map(oid));
        optional(&mut map, 12, self.not_after_operation.as_ref().map(oid));
        map
    }
}
impl CanonicalObject for Identity {
    const KIND: ObjectKind = ObjectKind::Identity;
    fn validate(&self) -> Result<(), ObjectError> {
        let bootstrap = self.version == 0;
        if bootstrap
            != (self.previous.is_none()
                && self.activation_operation.is_none()
                && self.policy_ref.is_none())
        {
            return Err(ObjectError::Invalid(
                "only a version-zero bootstrap identity may omit policy, previous identity, and activation operation",
            ));
        }
        if !bootstrap
            && (self.policy_ref.is_none()
                || self.previous.is_none()
                || self.activation_operation.is_none())
        {
            return Err(ObjectError::Invalid(
                "nonbootstrap identity requires policy, previous identity, and activation operation",
            ));
        }
        if self.signing_keys.is_empty() {
            return Err(ObjectError::Invalid("identity requires a signing key"));
        }
        for key in self.signing_keys.iter().chain(&self.encryption_keys) {
            key.validate()?;
        }
        if self
            .signing_keys
            .iter()
            .any(|key| key.algorithm != PublicKeyAlgorithm::Ed25519)
            || self
                .encryption_keys
                .iter()
                .any(|key| key.algorithm != PublicKeyAlgorithm::X25519)
        {
            return Err(ObjectError::Invalid("key algorithm does not match key use"));
        }
        for keys in [&self.signing_keys, &self.encryption_keys] {
            let encoded = keys
                .iter()
                .map(|k| k.value().encode())
                .collect::<Result<Vec<_>, _>>()?;
            if encoded.windows(2).any(|p| p[0] >= p[1]) {
                return Err(ObjectError::Invalid(
                    "identity key records must be sorted and unique",
                ));
            }
        }
        let mut key_ids = BTreeSet::new();
        if self
            .signing_keys
            .iter()
            .chain(&self.encryption_keys)
            .any(|key| !key_ids.insert(key.key_id))
        {
            return Err(ObjectError::Invalid(
                "identity key IDs must be unique across signing and encryption keys",
            ));
        }
        signatures(&self.signatures, SignaturePurpose::Identity)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            13,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl SignedObject for Identity {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum MembershipState {
    Active = 0,
    Removed = 1,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMembership {
    pub policy_ref: PolicyRef,
    pub membership_id: MembershipId,
    pub group_id: GroupId,
    pub version: u64,
    pub previous: Option<ObjectId>,
    pub principal: Principal,
    pub state: MembershipState,
    pub issuer: ActorId,
    pub activation_operation: ObjectId,
    pub not_after_operation: Option<ObjectId>,
    pub signatures: Vec<Signature>,
}
impl GroupMembership {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, self.membership_id.value()),
            (4, self.group_id.value()),
            (5, Value::Unsigned(self.version)),
            (7, principal_value(&self.principal)),
            (8, Value::Unsigned(self.state as u64)),
            (9, self.issuer.into()),
            (10, oid(&self.activation_operation)),
        ]);
        optional(&mut m, 6, self.previous.as_ref().map(oid));
        optional(&mut m, 11, self.not_after_operation.as_ref().map(oid));
        m
    }
}
impl CanonicalObject for GroupMembership {
    const KIND: ObjectKind = ObjectKind::GroupMembership;
    fn validate(&self) -> Result<(), ObjectError> {
        if (self.version == 0) == self.previous.is_some() {
            return Err(ObjectError::Invalid(
                "only membership version zero may omit previous",
            ));
        }
        if self.principal.identifier.is_empty() {
            return Err(ObjectError::Invalid("membership principal is empty"));
        }
        signatures(&self.signatures, SignaturePurpose::GroupMembership)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            12,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl SignedObject for GroupMembership {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipientEnvelope {
    pub recipient: Principal,
    pub key_id: [u8; 32],
    pub envelope: Vec<u8>,
}
impl RecipientEnvelope {
    fn value(&self) -> Value {
        Value::Map(vec![
            (0, principal_value(&self.recipient)),
            (1, Value::Bytes(self.key_id.to_vec())),
            (2, Value::Bytes(self.envelope.clone())),
        ])
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEnvelopeSet {
    pub policy_ref: Option<PolicyRef>,
    pub epoch: u64,
    pub suite: KeyEnvelopeSuite,
    pub recipients: Vec<RecipientEnvelope>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum KeyEnvelopeSuite {
    X25519HkdfSha256Aes256Gcm = 0,
}
impl KeyEnvelopeSet {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
}
impl CanonicalObject for KeyEnvelopeSet {
    const KIND: ObjectKind = ObjectKind::KeyEnvelopeSet;
    fn validate(&self) -> Result<(), ObjectError> {
        if (self.epoch == 0) != self.policy_ref.is_none() {
            return Err(ObjectError::Invalid(
                "only bootstrap key envelope epoch zero may omit policy",
            ));
        }
        if self.recipients.is_empty() {
            return Err(ObjectError::Invalid("key envelope set has no recipients"));
        }
        for r in &self.recipients {
            if r.recipient.identifier.is_empty() || r.key_id == [0; 32] || r.envelope.is_empty() {
                return Err(ObjectError::Invalid("invalid recipient envelope"));
            }
        }
        let e = self
            .recipients
            .iter()
            .map(|r| r.value().encode())
            .collect::<Result<Vec<_>, _>>()?;
        if e.windows(2).any(|p| p[0] >= p[1]) {
            return Err(ObjectError::Invalid(
                "recipient envelopes must be sorted and unique",
            ));
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, self.policy_ref.as_ref());
        m.extend([
            (3, Value::Unsigned(self.epoch)),
            (4, Value::Unsigned(self.suite as u64)),
            (
                5,
                Value::Array(
                    self.recipients
                        .iter()
                        .map(RecipientEnvelope::value)
                        .collect(),
                ),
            ),
        ]);
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ChangeRelationKind {
    Split = 0,
    Combine = 1,
    Supersede = 2,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRelation {
    pub policy_ref: PolicyRef,
    pub relation_kind: ChangeRelationKind,
    pub sources: Vec<ObjectId>,
    pub results: Vec<ObjectId>,
    pub provenance: Vec<ObjectId>,
    pub creating_operation: ObjectId,
}
impl ChangeRelation {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
}
impl CanonicalObject for ChangeRelation {
    const KIND: ObjectKind = ObjectKind::ChangeRelation;
    fn validate(&self) -> Result<(), ObjectError> {
        if self.sources.is_empty() || self.results.is_empty() {
            return Err(ObjectError::Invalid(
                "change relation endpoints must not be empty",
            ));
        }
        sorted_unique_ids(&self.sources, "relation sources must be sorted and unique")?;
        sorted_unique_ids(&self.results, "relation results must be sorted and unique")?;
        sorted_unique_ids(
            &self.provenance,
            "relation provenance must be sorted and unique",
        )
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, Value::Unsigned(self.relation_kind as u64)),
            (4, oid_array(&self.sources)),
            (5, oid_array(&self.results)),
            (6, oid_array(&self.provenance)),
            (7, oid(&self.creating_operation)),
        ]);
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ResolutionKind {
    Left = 0,
    Right = 1,
    Base = 2,
    Manual = 3,
    Driver = 4,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictResolution {
    pub policy_ref: PolicyRef,
    pub conflict: ObjectId,
    pub resolved: TypedObjectRef,
    pub resolver: ActorId,
    pub device: DeviceId,
    pub resolution_kind: ResolutionKind,
    pub provenance: Vec<TypedObjectRef>,
    pub wall_time: WallTime,
    pub signatures: Vec<Signature>,
}
impl ConflictResolution {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, oid(&self.conflict)),
            (4, self.resolved.value()),
            (5, self.resolver.into()),
            (6, self.device.into()),
            (7, Value::Unsigned(self.resolution_kind as u64)),
            (
                8,
                Value::Array(self.provenance.iter().map(TypedObjectRef::value).collect()),
            ),
            (9, self.wall_time.value()),
        ]);
        m
    }
}
impl CanonicalObject for ConflictResolution {
    const KIND: ObjectKind = ObjectKind::ConflictResolution;
    fn validate(&self) -> Result<(), ObjectError> {
        sorted_unique_refs(
            &self.provenance,
            "resolution provenance must be sorted and unique",
        )?;
        signatures(&self.signatures, SignaturePurpose::ConflictResolution)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            10,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl SignedObject for ConflictResolution {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum EvidenceOutcome {
    Pass = 0,
    Fail = 1,
    Abstain = 2,
    Error = 3,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceFields {
    pub policy_ref: PolicyRef,
    pub target: TypedObjectRef,
    pub snapshot: ObjectId,
    pub ruleset: ObjectId,
    pub issuer: ActorId,
    pub device: DeviceId,
    pub outcome: EvidenceOutcome,
    pub constraints: Vec<u8>,
    pub related: Vec<TypedObjectRef>,
    pub wall_time: WallTime,
    pub signatures: Vec<Signature>,
}
fn evidence_map(kind: ObjectKind, v: &EvidenceFields) -> Vec<(u64, Value)> {
    let mut m = header(kind, Some(&v.policy_ref));
    m.extend([
        (3, v.target.value()),
        (4, oid(&v.snapshot)),
        (5, oid(&v.ruleset)),
        (6, v.issuer.into()),
        (7, v.device.into()),
        (8, Value::Unsigned(v.outcome as u64)),
        (9, Value::Bytes(v.constraints.clone())),
        (
            10,
            Value::Array(v.related.iter().map(TypedObjectRef::value).collect()),
        ),
        (11, v.wall_time.value()),
    ]);
    m
}
fn validate_evidence(v: &EvidenceFields, p: SignaturePurpose) -> Result<(), ObjectError> {
    sorted_unique_refs(
        &v.related,
        "related evidence references must be sorted and unique",
    )?;
    validate_canonical_map(
        &v.constraints,
        "evidence constraints must be a canonical CBOR map",
    )?;
    signatures(&v.signatures, p)
}
macro_rules! evidence_object {
    ($name:ident,$kind:ident,$purpose:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name(pub EvidenceFields);
        impl $name {
            pub fn try_new(v: EvidenceFields) -> Result<Self, ObjectError> {
                let out = Self(v);
                out.validate()?;
                Ok(out)
            }
            fn unsigned_map(&self) -> Vec<(u64, Value)> {
                evidence_map(Self::KIND, &self.0)
            }
        }
        impl CanonicalObject for $name {
            const KIND: ObjectKind = ObjectKind::$kind;
            fn validate(&self) -> Result<(), ObjectError> {
                validate_evidence(&self.0, SignaturePurpose::$purpose)
            }
            fn canonical_value(&self) -> Result<Value, ObjectError> {
                let mut m = self.unsigned_map();
                m.push((
                    12,
                    Value::Array(self.0.signatures.iter().map(Signature::value).collect()),
                ));
                Ok(Value::Map(m))
            }
        }
        impl SignedObject for $name {
            fn unsigned_value(&self) -> Result<Value, ObjectError> {
                Ok(Value::Map(self.unsigned_map()))
            }
            fn signatures(&self) -> &[Signature] {
                &self.0.signatures
            }
        }
    };
}
evidence_object!(ReviewEvidence, ReviewEvidence, ReviewEvidence);
evidence_object!(ApprovalEvidence, ApprovalEvidence, ApprovalEvidence);
evidence_object!(
    PolicyDecisionEvidence,
    PolicyDecisionEvidence,
    PolicyDecisionEvidence
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CiEvidence {
    pub fields: EvidenceFields,
    pub check_name: String,
    pub runner_identity: ObjectId,
    pub build_provenance: Option<ObjectId>,
}
impl CiEvidence {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = evidence_map(Self::KIND, &self.fields);
        m.extend([
            (13, Value::Text(self.check_name.clone())),
            (14, oid(&self.runner_identity)),
        ]);
        optional(&mut m, 15, self.build_provenance.as_ref().map(oid));
        m
    }
}
impl CanonicalObject for CiEvidence {
    const KIND: ObjectKind = ObjectKind::CiEvidence;
    fn validate(&self) -> Result<(), ObjectError> {
        nfc(&self.check_name)?;
        if self.check_name.is_empty() {
            return Err(ObjectError::Invalid("CI check name must not be empty"));
        }
        validate_evidence(&self.fields, SignaturePurpose::CiEvidence)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            12,
            Value::Array(
                self.fields
                    .signatures
                    .iter()
                    .map(Signature::value)
                    .collect(),
            ),
        ));
        m.sort_by_key(|(k, _)| *k);
        Ok(Value::Map(m))
    }
}
impl SignedObject for CiEvidence {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.fields.signatures
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionRule {
    pub rule_kind: ProjectionRuleKind,
    pub parameters: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ProjectionRuleKind {
    Include = 0,
    Exclude = 1,
    Redact = 2,
}
impl ProjectionRule {
    fn value(&self) -> Value {
        Value::Map(vec![
            (0, Value::Unsigned(self.rule_kind as u64)),
            (1, Value::Bytes(self.parameters.clone())),
        ])
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionRules {
    pub policy_ref: PolicyRef,
    pub version: u64,
    pub previous: Option<ObjectId>,
    pub rules: Vec<ProjectionRule>,
    pub default_fail: bool,
}
impl ProjectionRules {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
}
impl CanonicalObject for ProjectionRules {
    const KIND: ObjectKind = ObjectKind::ProjectionRules;
    fn validate(&self) -> Result<(), ObjectError> {
        if (self.version == 0) == self.previous.is_some() {
            return Err(ObjectError::Invalid(
                "only projection rules version zero may omit previous",
            ));
        }
        if !self.default_fail {
            return Err(ObjectError::Invalid(
                "schema-0 projection rules must fail closed",
            ));
        }
        for rule in &self.rules {
            validate_canonical_map(
                &rule.parameters,
                "projection rule parameters must be canonical CBOR maps",
            )?;
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, Value::Unsigned(self.version)),
            (
                5,
                Value::Array(self.rules.iter().map(ProjectionRule::value).collect()),
            ),
            (6, Value::Bool(self.default_fail)),
        ]);
        optional(&mut m, 4, self.previous.as_ref().map(oid));
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionProof {
    pub policy_ref: PolicyRef,
    pub algorithm: ProjectionProofAlgorithm,
    pub source_snapshot: ObjectId,
    pub rules: ObjectId,
    pub audience_policy: ObjectId,
    pub projected_manifest: ObjectId,
    pub proof: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ProjectionProofAlgorithm {
    MerkleV0 = 0,
}
impl ProjectionProof {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
}
impl CanonicalObject for ProjectionProof {
    const KIND: ObjectKind = ObjectKind::ProjectionProof;
    fn validate(&self) -> Result<(), ObjectError> {
        if self.proof.is_empty() {
            return Err(ObjectError::Invalid("projection proof must not be empty"));
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, Value::Unsigned(self.algorithm as u64)),
            (4, oid(&self.source_snapshot)),
            (5, oid(&self.rules)),
            (6, oid(&self.audience_policy)),
            (7, oid(&self.projected_manifest)),
            (8, Value::Bytes(self.proof.clone())),
        ]);
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildProvenance {
    pub policy_ref: PolicyRef,
    pub snapshot: ObjectId,
    pub ruleset: ObjectId,
    pub builder_identity: ObjectId,
    pub inputs: Vec<TypedObjectRef>,
    pub outputs: Vec<ObjectId>,
    pub reproducibility: Vec<u8>,
    pub wall_time: WallTime,
    pub signatures: Vec<Signature>,
}
impl BuildProvenance {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, oid(&self.snapshot)),
            (4, oid(&self.ruleset)),
            (5, oid(&self.builder_identity)),
            (
                6,
                Value::Array(self.inputs.iter().map(TypedObjectRef::value).collect()),
            ),
            (7, oid_array(&self.outputs)),
            (8, Value::Bytes(self.reproducibility.clone())),
            (9, self.wall_time.value()),
        ]);
        m
    }
}
impl CanonicalObject for BuildProvenance {
    const KIND: ObjectKind = ObjectKind::BuildProvenance;
    fn validate(&self) -> Result<(), ObjectError> {
        sorted_unique_refs(&self.inputs, "build inputs must be sorted and unique")?;
        sorted_unique_ids(&self.outputs, "build outputs must be sorted and unique")?;
        validate_canonical_map(
            &self.reproducibility,
            "build reproducibility metadata must be a canonical CBOR map",
        )?;
        signatures(&self.signatures, SignaturePurpose::BuildProvenance)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            10,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl SignedObject for BuildProvenance {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub policy_ref: PolicyRef,
    pub artifact_kind: ArtifactKind,
    pub digest_algorithm: ArtifactDigestAlgorithm,
    pub digest: Vec<u8>,
    pub byte_length: u64,
    pub locator: Option<String>,
    pub blob: Option<ObjectId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ArtifactKind {
    Binary = 0,
    SourceArchive = 1,
    Sbom = 2,
    Attestation = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ArtifactDigestAlgorithm {
    Sha256 = 0,
    Blake3_256 = 1,
}
impl Artifact {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
}
impl CanonicalObject for Artifact {
    const KIND: ObjectKind = ObjectKind::Artifact;
    fn validate(&self) -> Result<(), ObjectError> {
        if self.digest.len() != 32 || self.locator.is_none() && self.blob.is_none() {
            return Err(ObjectError::Invalid(
                "artifact requires digest and location or blob",
            ));
        }
        if let Some(v) = &self.locator {
            nfc(v)?;
        }
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, Value::Unsigned(self.artifact_kind as u64)),
            (4, Value::Unsigned(self.digest_algorithm as u64)),
            (5, Value::Bytes(self.digest.clone())),
            (6, Value::Unsigned(self.byte_length)),
        ]);
        optional(
            &mut m,
            7,
            self.locator.as_ref().map(|s| Value::Text(s.clone())),
        );
        optional(&mut m, 8, self.blob.as_ref().map(oid));
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum OperationPayloadKind {
    Inverse = 0,
    Recovery = 1,
    PublicRedaction = 2,
    PrivateAudit = 3,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationPayload {
    pub policy_ref: PolicyRef,
    pub payload_kind: OperationPayloadKind,
    pub references: Vec<TypedObjectRef>,
    pub payload_schema: OperationPayloadSchema,
    pub canonical_payload: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum OperationPayloadSchema {
    CanonicalCborMapV0 = 0,
}
impl OperationPayload {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
}
impl CanonicalObject for OperationPayload {
    const KIND: ObjectKind = ObjectKind::OperationPayload;
    fn validate(&self) -> Result<(), ObjectError> {
        sorted_unique_refs(
            &self.references,
            "payload references must be sorted and unique",
        )?;
        validate_canonical_map(
            &self.canonical_payload,
            "operation payload must be a canonical CBOR map",
        )
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, Value::Unsigned(self.payload_kind as u64)),
            (
                4,
                Value::Array(self.references.iter().map(TypedObjectRef::value).collect()),
            ),
            (5, Value::Unsigned(self.payload_schema as u64)),
            (6, Value::Bytes(self.canonical_payload.clone())),
        ]);
        Ok(Value::Map(m))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineGeneration {
    pub line_id: crate::LineId,
    pub generation: u64,
    pub state: ObjectId,
}
impl LineGeneration {
    fn value(&self) -> Value {
        Value::Map(vec![
            (0, self.line_id.into()),
            (1, Value::Unsigned(self.generation)),
            (2, oid(&self.state)),
        ])
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub policy_ref: PolicyRef,
    pub actor: ActorId,
    pub device: DeviceId,
    pub policies: Vec<ObjectId>,
    pub lines: Vec<LineGeneration>,
    pub projected_manifest: ObjectId,
    pub validity_constraints: Vec<u8>,
    pub signatures: Vec<Signature>,
}
impl View {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, self.actor.into()),
            (4, self.device.into()),
            (5, oid_array(&self.policies)),
            (
                6,
                Value::Array(self.lines.iter().map(LineGeneration::value).collect()),
            ),
            (7, oid(&self.projected_manifest)),
            (8, Value::Bytes(self.validity_constraints.clone())),
        ]);
        m
    }
}
impl CanonicalObject for View {
    const KIND: ObjectKind = ObjectKind::View;
    fn validate(&self) -> Result<(), ObjectError> {
        sorted_unique_ids(&self.policies, "view policies must be sorted and unique")?;
        if self.lines.windows(2).any(|p| p[0] >= p[1]) {
            return Err(ObjectError::Invalid("view lines must be sorted and unique"));
        }
        let mut line_ids = BTreeSet::new();
        if self.lines.iter().any(|line| !line_ids.insert(line.line_id)) {
            return Err(ObjectError::Invalid("view contains duplicate line IDs"));
        }
        validate_canonical_map(
            &self.validity_constraints,
            "view validity constraints must be a canonical CBOR map",
        )?;
        signatures(&self.signatures, SignaturePurpose::View)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            9,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl SignedObject for View {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdMapping {
    pub old: ObjectId,
    pub new: ObjectId,
}
impl IdMapping {
    fn value(&self) -> Value {
        Value::Map(vec![(0, oid(&self.old)), (1, oid(&self.new))])
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration {
    pub policy_ref: PolicyRef,
    pub source_format: ObjectIdFormat,
    pub target_format: ObjectIdFormat,
    pub mappings: Vec<IdMapping>,
    pub tool_identity: ObjectId,
    pub wall_time: WallTime,
    pub signatures: Vec<Signature>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ObjectIdFormat {
    V0 = 0,
    V1 = 1,
}
impl Migration {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
    fn unsigned_map(&self) -> Vec<(u64, Value)> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, Value::Unsigned(self.source_format as u64)),
            (4, Value::Unsigned(self.target_format as u64)),
            (
                5,
                Value::Array(self.mappings.iter().map(IdMapping::value).collect()),
            ),
            (6, oid(&self.tool_identity)),
            (7, self.wall_time.value()),
        ]);
        m
    }
}
impl CanonicalObject for Migration {
    const KIND: ObjectKind = ObjectKind::Migration;
    fn validate(&self) -> Result<(), ObjectError> {
        if self.source_format == self.target_format || self.mappings.is_empty() {
            return Err(ObjectError::Invalid(
                "migration must change formats and contain mappings",
            ));
        }
        let encoded = self
            .mappings
            .iter()
            .map(|v| v.value().encode())
            .collect::<Result<Vec<_>, _>>()?;
        if encoded.windows(2).any(|p| p[0] >= p[1]) {
            return Err(ObjectError::Invalid(
                "migration mappings must be sorted and unique",
            ));
        }
        let mut old = BTreeSet::new();
        let mut new = BTreeSet::new();
        if self
            .mappings
            .iter()
            .any(|m| !old.insert(&m.old) || !new.insert(&m.new))
        {
            return Err(ObjectError::Invalid("migration must be one-to-one"));
        }
        signatures(&self.signatures, SignaturePurpose::Migration)
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = self.unsigned_map();
        m.push((
            8,
            Value::Array(self.signatures.iter().map(Signature::value).collect()),
        ));
        Ok(Value::Map(m))
    }
}
impl SignedObject for Migration {
    fn unsigned_value(&self) -> Result<Value, ObjectError> {
        Ok(Value::Map(self.unsigned_map()))
    }
    fn signatures(&self) -> &[Signature] {
        &self.signatures
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum RulesetKind {
    Review = 0,
    Approval = 1,
    Ci = 2,
    Landing = 3,
    Release = 4,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ruleset {
    pub policy_ref: PolicyRef,
    pub ruleset_kind: RulesetKind,
    pub version: u64,
    pub previous: Option<ObjectId>,
    pub constraints: Vec<u8>,
    pub required_evidence_kinds: Vec<ObjectKind>,
}
impl Ruleset {
    pub fn try_new(v: Self) -> Result<Self, ObjectError> {
        v.validate()?;
        Ok(v)
    }
}
impl CanonicalObject for Ruleset {
    const KIND: ObjectKind = ObjectKind::Ruleset;
    fn validate(&self) -> Result<(), ObjectError> {
        if (self.version == 0) == self.previous.is_some() {
            return Err(ObjectError::Invalid(
                "only ruleset version zero may omit previous",
            ));
        }
        if self
            .required_evidence_kinds
            .windows(2)
            .any(|p| p[0] as u64 >= p[1] as u64)
            || self.required_evidence_kinds.iter().any(|k| {
                !matches!(
                    k,
                    ObjectKind::ReviewEvidence
                        | ObjectKind::ApprovalEvidence
                        | ObjectKind::CiEvidence
                        | ObjectKind::PolicyDecisionEvidence
                )
            })
        {
            return Err(ObjectError::Invalid(
                "ruleset evidence kinds must be sorted, unique evidence kinds",
            ));
        }
        validate_canonical_map(
            &self.constraints,
            "ruleset constraints must be a canonical CBOR map",
        )?;
        Ok(())
    }
    fn canonical_value(&self) -> Result<Value, ObjectError> {
        let mut m = header(Self::KIND, Some(&self.policy_ref));
        m.extend([
            (3, Value::Unsigned(self.ruleset_kind as u64)),
            (4, Value::Unsigned(self.version)),
            (6, Value::Bytes(self.constraints.clone())),
            (
                7,
                Value::Array(
                    self.required_evidence_kinds
                        .iter()
                        .map(|k| Value::Unsigned(*k as u64))
                        .collect(),
                ),
            ),
        ]);
        optional(&mut m, 5, self.previous.as_ref().map(oid));
        Ok(Value::Map(m))
    }
}

/// Complete schema-0 bootstrap material in dependency order.
///
/// This validates structural trust bootstrapping and hash links. Signature
/// cryptography is intentionally left to the crypto milestone.
pub struct BootstrapGraph<'a> {
    pub root: &'a RepositoryRoot,
    pub root_policy: &'a Policy,
    pub identities: &'a [Identity],
    pub key_envelope_set: &'a KeyEnvelopeSet,
    pub genesis_operation: &'a Operation,
    pub initial_line_states: &'a [LineState],
}

impl BootstrapGraph<'_> {
    pub fn validate(&self, algorithm: HashAlgorithm) -> Result<(), ObjectError> {
        self.root.validate()?;
        self.root_policy.validate()?;
        self.key_envelope_set.validate()?;
        self.genesis_operation.validate()?;
        if self.root_policy.policy_ref.is_some()
            || self.root_policy.version_sequence != 0
            || self.root_policy.previous_version.is_some()
        {
            return Err(ObjectError::Invalid(
                "bootstrap root policy must be version zero and parentless",
            ));
        }
        if self.key_envelope_set.policy_ref.is_some() || self.key_envelope_set.epoch != 0 {
            return Err(ObjectError::Invalid(
                "bootstrap key envelope set must be epoch zero and policy-free",
            ));
        }
        if self.identities.is_empty() || self.initial_line_states.is_empty() {
            return Err(ObjectError::Invalid(
                "bootstrap identity and line sets must not be empty",
            ));
        }
        let identity_ids = self
            .identities
            .iter()
            .map(|identity| {
                identity.validate()?;
                if identity.version != 0
                    || identity.policy_ref.is_some()
                    || identity.previous.is_some()
                    || identity.activation_operation.is_some()
                {
                    return Err(ObjectError::Invalid(
                        "bootstrap identity has nonbootstrap links",
                    ));
                }
                identity.id(algorithm)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if identity_ids != self.root.trusted_identities {
            return Err(ObjectError::Invalid(
                "repository root identity links do not match bootstrap identities",
            ));
        }
        if self.root.root_policy != self.root_policy.id(algorithm)?
            || self.root.bootstrap_key_envelope_set != self.key_envelope_set.id(algorithm)?
            || self.root.genesis_operation != self.genesis_operation.id(algorithm)?
        {
            return Err(ObjectError::Invalid(
                "repository root bootstrap object link mismatch",
            ));
        }
        if self.root_policy.key_envelope_set != self.root.bootstrap_key_envelope_set {
            return Err(ObjectError::Invalid(
                "root policy does not name bootstrap key envelope set",
            ));
        }
        if !self.genesis_operation.parents.is_empty() {
            return Err(ObjectError::Invalid(
                "genesis operation must not have parents",
            ));
        }
        let policy_ref = PolicyRef {
            policy_id: self.root_policy.policy_id,
            version: self.root.root_policy.clone(),
        };
        if self.genesis_operation.policy_ref != policy_ref {
            return Err(ObjectError::Invalid(
                "genesis operation does not use the root policy",
            ));
        }
        let line_ids = self
            .initial_line_states
            .iter()
            .map(|line| {
                line.validate()?;
                if line.generation != 0
                    || line.previous_state.is_some()
                    || line.policy_ref != policy_ref
                {
                    return Err(ObjectError::Invalid(
                        "initial line state is not a policy-bound genesis state",
                    ));
                }
                line.validate_transaction(self.genesis_operation)?;
                line.id(algorithm)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if line_ids != self.root.initial_line_states {
            return Err(ObjectError::Invalid(
                "repository root line links do not match initial states",
            ));
        }
        for root_signature in &self.root.signatures {
            let signer_id = root_signature.signer();
            let signer = signer_id.as_bytes();
            let key_id = root_signature.signing_key_id();
            if !self.identities.iter().any(|identity| {
                identity.subject_kind == IdentitySubjectKind::Actor
                    && &identity.subject == signer
                    && identity
                        .signing_keys
                        .iter()
                        .any(|key| &key.key_id == key_id)
            }) {
                return Err(ObjectError::Invalid(
                    "repository root signer is not a bootstrap identity key",
                ));
            }
        }
        Ok(())
    }
}
