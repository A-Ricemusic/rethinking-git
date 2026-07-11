use serde::Serialize;

use crate::{ObjectId, Value};

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub [u8; 16]);
        impl $name {
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }
        impl From<$name> for Value {
            fn from(id: $name) -> Self {
                Self::Bytes(id.0.to_vec())
            }
        }
    };
}

stable_id!(ChangeId);
stable_id!(LineId);
stable_id!(PolicyId);
stable_id!(ActorId);
stable_id!(DeviceId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRef {
    pub policy_id: PolicyId,
    pub version: ObjectId,
}

impl PolicyRef {
    pub(crate) fn value(&self) -> Value {
        Value::Map(vec![(0, self.policy_id.into()), (1, oid(&self.version))])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedObjectRef {
    pub kind: ObjectKind,
    pub id: ObjectId,
}
impl TypedObjectRef {
    pub(crate) fn value(&self) -> Value {
        Value::Map(vec![
            (0, Value::Unsigned(self.kind as u64)),
            (1, oid(&self.id)),
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallTime {
    pub utc_seconds: i64,
    pub offset_seconds: i64,
}
impl WallTime {
    pub(crate) fn value(&self) -> Value {
        Value::Map(vec![
            (0, Value::Signed(self.utc_seconds)),
            (1, Value::Signed(self.offset_seconds)),
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u64)]
pub enum SignatureAlgorithm {
    Ed25519 = 0,
}
impl TryFrom<u64> for SignatureAlgorithm {
    type Error = u64;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Ed25519),
            other => Err(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u64)]
pub enum SignaturePurpose {
    LineState = 0,
    Operation = 1,
    Marker = 2,
    Release = 3,
    Policy = 4,
}
impl SignaturePurpose {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LineState => "line-state",
            Self::Operation => "operation",
            Self::Marker => "marker",
            Self::Release => "release",
            Self::Policy => "policy",
        }
    }
}
impl TryFrom<&str> for SignaturePurpose {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "line-state" => Ok(Self::LineState),
            "operation" => Ok(Self::Operation),
            "marker" => Ok(Self::Marker),
            "release" => Ok(Self::Release),
            "policy" => Ok(Self::Policy),
            _ => Err(()),
        }
    }
}
impl TryFrom<u64> for SignaturePurpose {
    type Error = u64;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::LineState),
            1 => Ok(Self::Operation),
            2 => Ok(Self::Marker),
            3 => Ok(Self::Release),
            4 => Ok(Self::Policy),
            other => Err(other),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("signing key ID must not be the reserved all-zero placeholder")]
    PlaceholderKeyId,
    #[error("signature must not be the reserved all-zero placeholder")]
    PlaceholderSignature,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signature {
    algorithm: SignatureAlgorithm,
    signer: ActorId,
    signing_key_id: [u8; 32],
    bytes: [u8; 64],
    purpose: SignaturePurpose,
}
impl Signature {
    pub fn new(
        algorithm: SignatureAlgorithm,
        signer: ActorId,
        signing_key_id: [u8; 32],
        bytes: [u8; 64],
        purpose: SignaturePurpose,
    ) -> Result<Self, SignatureError> {
        if signing_key_id == [0; 32] {
            return Err(SignatureError::PlaceholderKeyId);
        }
        if bytes == [0; 64] {
            return Err(SignatureError::PlaceholderSignature);
        }
        Ok(Self {
            algorithm,
            signer,
            signing_key_id,
            bytes,
            purpose,
        })
    }
    #[must_use]
    pub const fn algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }
    #[must_use]
    pub const fn signer(&self) -> ActorId {
        self.signer
    }
    #[must_use]
    pub const fn signing_key_id(&self) -> &[u8; 32] {
        &self.signing_key_id
    }
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 64] {
        &self.bytes
    }
    #[must_use]
    pub const fn purpose(&self) -> SignaturePurpose {
        self.purpose
    }
    pub(crate) fn value(&self) -> Value {
        Value::Map(vec![
            (0, Value::Unsigned(self.algorithm as u64)),
            (1, self.signer.into()),
            (2, Value::Bytes(self.signing_key_id.to_vec())),
            (3, Value::Bytes(self.bytes.to_vec())),
            (4, Value::Unsigned(self.purpose as u64)),
        ])
    }
}

/// Frozen signature-profile-0 preimage.
///
/// This function only constructs the bytes that the crypto layer signs or
/// verifies; it intentionally performs no private-key operation.
#[must_use]
pub fn signing_preimage(
    algorithm: SignatureAlgorithm,
    purpose: SignaturePurpose,
    signer: ActorId,
    signing_key_id: &[u8; 32],
    kind: ObjectKind,
    schema_version: u64,
    unsigned_cbor: &[u8],
) -> Vec<u8> {
    let mut out = b"RGIT-SIGNATURE\0".to_vec();
    crate::id::push_varint(&mut out, 0);
    crate::id::push_varint(&mut out, algorithm as u64);
    crate::id::push_varint(&mut out, purpose as u64);
    out.extend_from_slice(signer.as_bytes());
    crate::id::push_varint(&mut out, signing_key_id.len() as u64);
    out.extend_from_slice(signing_key_id);
    crate::id::push_varint(&mut out, kind as u64);
    crate::id::push_varint(&mut out, schema_version);
    crate::id::push_varint(&mut out, unsigned_cbor.len() as u64);
    out.extend_from_slice(unsigned_cbor);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[repr(u64)]
pub enum ObjectKind {
    Chunk = 1,
    Blob = 2,
    SecretRef = 3,
    Manifest = 4,
    Subproject = 5,
    Snapshot = 6,
    ChangeRevision = 7,
    LineState = 8,
    Conflict = 9,
    Operation = 10,
    Marker = 11,
    Release = 12,
    Policy = 13,
}
impl ObjectKind {
    #[must_use]
    pub const fn schema_version(self) -> u64 {
        0
    }
}
impl TryFrom<u64> for ObjectKind {
    type Error = u64;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => Self::Chunk,
            2 => Self::Blob,
            3 => Self::SecretRef,
            4 => Self::Manifest,
            5 => Self::Subproject,
            6 => Self::Snapshot,
            7 => Self::ChangeRevision,
            8 => Self::LineState,
            9 => Self::Conflict,
            10 => Self::Operation,
            11 => Self::Marker,
            12 => Self::Release,
            13 => Self::Policy,
            other => return Err(other),
        })
    }
}

pub(crate) fn oid(id: &ObjectId) -> Value {
    Value::Bytes(id.to_bytes())
}
pub(crate) fn oid_array(ids: &[ObjectId]) -> Value {
    Value::Array(ids.iter().map(oid).collect())
}
pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}
pub(crate) fn optional(map: &mut Vec<(u64, Value)>, key: u64, value: Option<Value>) {
    if let Some(value) = value {
        map.push((key, value));
    }
}
