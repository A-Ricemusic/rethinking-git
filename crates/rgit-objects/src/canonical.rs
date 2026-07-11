use std::collections::BTreeSet;

use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// The restricted set of values allowed in RGit canonical CBOR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Unsigned(u64),
    Signed(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Self>),
    Map(Vec<(u64, Self)>),
    Bool(bool),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalLimits {
    /// Maximum size of the complete canonical representation.
    pub max_bytes: usize,
    /// Maximum size of one CBOR byte string.
    pub max_byte_string_bytes: usize,
    /// Maximum UTF-8 byte length of one CBOR text string.
    pub max_string_bytes: usize,
    pub max_depth: usize,
    pub max_collection_items: usize,
}

impl CanonicalLimits {
    /// Limits for graph metadata and small inline blob content.
    #[must_use]
    pub const fn metadata() -> Self {
        Self {
            max_bytes: 1024 * 1024,
            max_byte_string_bytes: 256 * 1024,
            max_string_bytes: 64 * 1024,
            max_depth: 64,
            max_collection_items: 65_536,
        }
    }

    /// Limits for chunk payloads and blob descriptors.
    ///
    /// The extra encoded-size allowance covers a 4 MiB chunk's schema and
    /// policy-reference envelope without weakening metadata decoding limits.
    #[must_use]
    pub const fn bulk() -> Self {
        Self {
            max_bytes: 4 * 1024 * 1024 + 64 * 1024,
            max_byte_string_bytes: 4 * 1024 * 1024,
            max_string_bytes: 64 * 1024,
            max_depth: 64,
            max_collection_items: 1_000_000,
        }
    }
}

impl Default for CanonicalLimits {
    fn default() -> Self {
        Self::metadata()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CanonicalError {
    #[error("canonical object exceeds configured size limit")]
    SizeLimit,
    #[error("canonical object exceeds nesting limit")]
    DepthLimit,
    #[error("canonical collection exceeds item limit")]
    CollectionLimit,
    #[error("truncated CBOR input")]
    Truncated,
    #[error("trailing bytes after CBOR value")]
    TrailingBytes,
    #[error("forbidden CBOR major type or additional information")]
    ForbiddenType,
    #[error("integer is not encoded in its shortest form")]
    NonMinimalInteger,
    #[error("map key is not an unsigned integer")]
    InvalidMapKey,
    #[error("map keys are duplicated or not in canonical order")]
    MapOrder,
    #[error("text is not valid UTF-8")]
    Utf8,
    #[error("text is not Unicode NFC")]
    NonNormalizedText,
    #[error("decoded value is not byte-for-byte canonical")]
    NonCanonical,
    #[error("value cannot be canonically encoded: duplicated map key or non-NFC text")]
    InvalidValue,
}

impl Value {
    /// Encode using the bounded metadata profile.
    pub fn encode(&self) -> Result<Vec<u8>, CanonicalError> {
        self.encode_with_limits(CanonicalLimits::metadata())
    }

    /// Encode while enforcing the same limits accepted by the decoder.
    pub fn encode_with_limits(&self, limits: CanonicalLimits) -> Result<Vec<u8>, CanonicalError> {
        let mut out = Vec::new();
        self.encode_into(&mut out, limits, 0)?;
        Ok(out)
    }

    fn encode_into(
        &self,
        out: &mut Vec<u8>,
        limits: CanonicalLimits,
        depth: usize,
    ) -> Result<(), CanonicalError> {
        if depth > limits.max_depth {
            return Err(CanonicalError::DepthLimit);
        }
        match self {
            Self::Unsigned(value) => push_head(out, 0, *value, limits)?,
            Self::Signed(value) if *value >= 0 => push_head(out, 0, *value as u64, limits)?,
            Self::Signed(value) => {
                push_head(out, 1, (-1_i128 - i128::from(*value)) as u64, limits)?;
            }
            Self::Bytes(bytes) => {
                if bytes.len() > limits.max_byte_string_bytes {
                    return Err(CanonicalError::SizeLimit);
                }
                push_head(out, 2, bytes.len() as u64, limits)?;
                extend(out, bytes, limits)?;
            }
            Self::Text(text) => {
                if !text.nfc().eq(text.chars()) {
                    return Err(CanonicalError::InvalidValue);
                }
                if text.len() > limits.max_string_bytes {
                    return Err(CanonicalError::SizeLimit);
                }
                push_head(out, 3, text.len() as u64, limits)?;
                extend(out, text.as_bytes(), limits)?;
            }
            Self::Array(values) => {
                check_collection(values.len(), limits)?;
                push_head(out, 4, values.len() as u64, limits)?;
                for value in values {
                    value.encode_into(out, limits, depth + 1)?;
                }
            }
            Self::Map(entries) => {
                check_collection(entries.len(), limits)?;
                push_head(out, 5, entries.len() as u64, limits)?;
                let mut ordered: Vec<_> = entries.iter().collect();
                ordered.sort_by_key(|(key, _)| *key);
                if ordered.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                    return Err(CanonicalError::InvalidValue);
                }
                for (key, value) in ordered {
                    push_head(out, 0, *key, limits)?;
                    value.encode_into(out, limits, depth + 1)?;
                }
            }
            Self::Bool(false) => extend(out, &[0xf4], limits)?,
            Self::Bool(true) => extend(out, &[0xf5], limits)?,
            Self::Null => extend(out, &[0xf6], limits)?,
        }
        Ok(())
    }
}

fn check_collection(len: usize, limits: CanonicalLimits) -> Result<(), CanonicalError> {
    if len > limits.max_collection_items {
        Err(CanonicalError::CollectionLimit)
    } else {
        Ok(())
    }
}

fn extend(out: &mut Vec<u8>, bytes: &[u8], limits: CanonicalLimits) -> Result<(), CanonicalError> {
    let new_len = out
        .len()
        .checked_add(bytes.len())
        .ok_or(CanonicalError::SizeLimit)?;
    if new_len > limits.max_bytes {
        return Err(CanonicalError::SizeLimit);
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn push_head(
    out: &mut Vec<u8>,
    major: u8,
    value: u64,
    limits: CanonicalLimits,
) -> Result<(), CanonicalError> {
    let prefix = major << 5;
    match value {
        0..=23 => extend(out, &[prefix | value as u8], limits)?,
        24..=0xff => extend(out, &[prefix | 24, value as u8], limits)?,
        0x100..=0xffff => {
            extend(out, &[prefix | 25], limits)?;
            extend(out, &(value as u16).to_be_bytes(), limits)?;
        }
        0x1_0000..=0xffff_ffff => {
            extend(out, &[prefix | 26], limits)?;
            extend(out, &(value as u32).to_be_bytes(), limits)?;
        }
        _ => {
            extend(out, &[prefix | 27], limits)?;
            extend(out, &value.to_be_bytes(), limits)?;
        }
    }
    Ok(())
}

pub fn decode_canonical(input: &[u8], limits: CanonicalLimits) -> Result<Value, CanonicalError> {
    if input.len() > limits.max_bytes {
        return Err(CanonicalError::SizeLimit);
    }
    let mut decoder = Decoder {
        input,
        offset: 0,
        limits,
    };
    let value = decoder.value(0)?;
    if decoder.offset != input.len() {
        return Err(CanonicalError::TrailingBytes);
    }
    if value.encode_with_limits(limits)? != input {
        return Err(CanonicalError::NonCanonical);
    }
    Ok(value)
}

struct Decoder<'a> {
    input: &'a [u8],
    offset: usize,
    limits: CanonicalLimits,
}

impl Decoder<'_> {
    fn value(&mut self, depth: usize) -> Result<Value, CanonicalError> {
        if depth > self.limits.max_depth {
            return Err(CanonicalError::DepthLimit);
        }
        let initial = self.byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1f;
        match major {
            0 => Ok(Value::Unsigned(self.argument(additional)?)),
            1 => {
                let n = self.argument(additional)?;
                if n > i64::MAX as u64 {
                    return Err(CanonicalError::ForbiddenType);
                }
                Ok(Value::Signed(-1 - n as i64))
            }
            2 => {
                let len = self.byte_string_length(additional)?;
                Ok(Value::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let len = self.string_length(additional)?;
                let text = std::str::from_utf8(self.take(len)?)
                    .map_err(|_| CanonicalError::Utf8)?
                    .to_owned();
                if !text.nfc().eq(text.chars()) {
                    return Err(CanonicalError::NonNormalizedText);
                }
                Ok(Value::Text(text))
            }
            4 => {
                let len = self.length(additional)?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(values))
            }
            5 => {
                let len = self.length(additional)?;
                let mut entries = Vec::with_capacity(len);
                let mut keys = BTreeSet::new();
                let mut previous = None;
                for _ in 0..len {
                    let key_initial = self.byte()?;
                    if key_initial >> 5 != 0 {
                        return Err(CanonicalError::InvalidMapKey);
                    }
                    let key = self.argument(key_initial & 0x1f)?;
                    if previous.is_some_and(|old| old >= key) || !keys.insert(key) {
                        return Err(CanonicalError::MapOrder);
                    }
                    previous = Some(key);
                    entries.push((key, self.value(depth + 1)?));
                }
                Ok(Value::Map(entries))
            }
            7 => match additional {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                _ => Err(CanonicalError::ForbiddenType),
            },
            _ => Err(CanonicalError::ForbiddenType),
        }
    }

    fn byte(&mut self) -> Result<u8, CanonicalError> {
        let byte = *self
            .input
            .get(self.offset)
            .ok_or(CanonicalError::Truncated)?;
        self.offset += 1;
        Ok(byte)
    }
    fn take(&mut self, len: usize) -> Result<&[u8], CanonicalError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(CanonicalError::SizeLimit)?;
        let bytes = self
            .input
            .get(self.offset..end)
            .ok_or(CanonicalError::Truncated)?;
        self.offset = end;
        Ok(bytes)
    }
    fn length(&mut self, additional: u8) -> Result<usize, CanonicalError> {
        let value = self.argument(additional)?;
        let len = usize::try_from(value).map_err(|_| CanonicalError::CollectionLimit)?;
        if len > self.limits.max_collection_items {
            return Err(CanonicalError::CollectionLimit);
        }
        Ok(len)
    }
    fn string_length(&mut self, additional: u8) -> Result<usize, CanonicalError> {
        let value = self.argument(additional)?;
        let len = usize::try_from(value).map_err(|_| CanonicalError::SizeLimit)?;
        if len > self.limits.max_string_bytes {
            return Err(CanonicalError::SizeLimit);
        }
        Ok(len)
    }
    fn byte_string_length(&mut self, additional: u8) -> Result<usize, CanonicalError> {
        let value = self.argument(additional)?;
        let len = usize::try_from(value).map_err(|_| CanonicalError::SizeLimit)?;
        if len > self.limits.max_byte_string_bytes {
            return Err(CanonicalError::SizeLimit);
        }
        Ok(len)
    }
    fn argument(&mut self, additional: u8) -> Result<u64, CanonicalError> {
        match additional {
            0..=23 => Ok(u64::from(additional)),
            24 => {
                let n = u64::from(self.byte()?);
                if n < 24 {
                    Err(CanonicalError::NonMinimalInteger)
                } else {
                    Ok(n)
                }
            }
            25 => {
                let n = u64::from(u16::from_be_bytes(
                    self.take(2)?.try_into().expect("length"),
                ));
                if n <= 0xff {
                    Err(CanonicalError::NonMinimalInteger)
                } else {
                    Ok(n)
                }
            }
            26 => {
                let n = u64::from(u32::from_be_bytes(
                    self.take(4)?.try_into().expect("length"),
                ));
                if n <= 0xffff {
                    Err(CanonicalError::NonMinimalInteger)
                } else {
                    Ok(n)
                }
            }
            27 => {
                let n = u64::from_be_bytes(self.take(8)?.try_into().expect("length"));
                if n <= 0xffff_ffff {
                    Err(CanonicalError::NonMinimalInteger)
                } else {
                    Ok(n)
                }
            }
            _ => Err(CanonicalError::ForbiddenType),
        }
    }
}
