use std::{fmt, str::FromStr};

use data_encoding::BASE32HEX_NOPAD;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u64)]
pub enum HashAlgorithm {
    Sha256 = 0x12,
    Blake3_256 = 0x1e,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    format_version: u64,
    algorithm: HashAlgorithm,
    digest: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ObjectIdError {
    #[error("object ID must start with rg0_")]
    Prefix,
    #[error("object ID is not valid unpadded base32hex")]
    Base32,
    #[error("unsupported object ID format {0}")]
    Format(u64),
    #[error("unsupported hash algorithm {0:#x}")]
    Algorithm(u64),
    #[error("invalid digest length {0}")]
    DigestLength(u64),
    #[error("truncated or trailing object ID bytes")]
    Length,
    #[error("non-minimal object ID varint")]
    NonMinimalVarint,
}

impl ObjectId {
    pub const FORMAT_VERSION: u64 = 0;
    #[must_use]
    pub(crate) fn from_payload(
        kind: u64,
        schema_version: u64,
        payload: &[u8],
        algorithm: HashAlgorithm,
    ) -> Self {
        let preimage = Self::preimage(kind, schema_version, payload);
        let digest = match algorithm {
            HashAlgorithm::Sha256 => Sha256::digest(&preimage).into(),
            HashAlgorithm::Blake3_256 => *blake3::hash(&preimage).as_bytes(),
        };
        Self {
            format_version: 0,
            algorithm,
            digest,
        }
    }
    #[must_use]
    pub fn preimage(kind: u64, schema_version: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = b"RGIT-OBJECT\0".to_vec();
        push_varint(&mut out, kind);
        push_varint(&mut out, schema_version);
        out.extend_from_slice(payload);
        out
    }
    #[must_use]
    pub const fn format_version(&self) -> u64 {
        self.format_version
    }
    #[must_use]
    pub const fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(35);
        push_varint(&mut out, self.format_version);
        push_varint(&mut out, self.algorithm as u64);
        push_varint(&mut out, 32);
        out.extend_from_slice(&self.digest);
        out
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ObjectIdError> {
        let mut at = 0;
        let format = read_varint(bytes, &mut at)?;
        if format != 0 {
            return Err(ObjectIdError::Format(format));
        }
        let code = read_varint(bytes, &mut at)?;
        let algorithm = match code {
            0x12 => HashAlgorithm::Sha256,
            0x1e => HashAlgorithm::Blake3_256,
            _ => return Err(ObjectIdError::Algorithm(code)),
        };
        let len = read_varint(bytes, &mut at)?;
        if len != 32 {
            return Err(ObjectIdError::DigestLength(len));
        }
        if bytes.len() != at + 32 {
            return Err(ObjectIdError::Length);
        }
        let digest = bytes[at..].try_into().expect("checked length");
        Ok(Self {
            format_version: format,
            algorithm,
            digest,
        })
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rg0_{}",
            BASE32HEX_NOPAD
                .encode(&self.to_bytes())
                .to_ascii_lowercase()
        )
    }
}
impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ObjectId").field(&self.to_string()).finish()
    }
}
impl FromStr for ObjectId {
    type Err = ObjectIdError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let encoded = text
            .strip_prefix("rg0_")
            .or_else(|| text.strip_prefix("RG0_"))
            .ok_or(ObjectIdError::Prefix)?;
        let bytes = BASE32HEX_NOPAD
            .decode(encoded.to_ascii_uppercase().as_bytes())
            .map_err(|_| ObjectIdError::Base32)?;
        Self::from_bytes(&bytes)
    }
}

pub(crate) fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}
fn read_varint(bytes: &[u8], at: &mut usize) -> Result<u64, ObjectIdError> {
    let start = *at;
    let mut result = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*at).ok_or(ObjectIdError::Length)?;
        *at += 1;
        if shift == 63 && byte > 1 {
            return Err(ObjectIdError::Length);
        }
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            let mut encoded = Vec::new();
            push_varint(&mut encoded, result);
            if encoded.len() != *at - start {
                return Err(ObjectIdError::NonMinimalVarint);
            }
            return Ok(result);
        }
    }
    Err(ObjectIdError::Length)
}
