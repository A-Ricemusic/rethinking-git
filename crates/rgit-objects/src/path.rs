use std::fmt;

use thiserror::Error;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

use crate::Value;

/// Maximum UTF-8 encoding length of one schema-0 portable path component.
///
/// UTF-8 bytes are used instead of host-native units so validation is identical
/// on every platform. This bound is also conservative for Windows: a valid
/// Unicode string never has more UTF-16 code units than UTF-8 bytes.
pub const PORTABLE_COMPONENT_MAX_BYTES: usize = 255;

/// Maximum UTF-8 encoding length of a schema-0 materialized relative path.
///
/// The length includes one `/` byte between components, but no leading slash or
/// trailing NUL. The 1,023-byte limit fits the smallest Tier-1 `PATH_MAX`
/// profile while remaining far below extended-length Win32's UTF-16 limit.
pub const PORTABLE_PATH_MAX_BYTES: usize = 1_023;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathSegment(String);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("path segment is empty")]
    Empty,
    #[error("path segment is not Unicode NFC")]
    NonNormalized,
    #[error("path segment is reserved or contains a forbidden character")]
    Unsafe,
    #[error("path segment contains a character forbidden by the portable profile")]
    NonPortableCharacter,
    #[error("path segment is not portable to Windows filesystems")]
    WindowsReserved,
    #[error("path segment exceeds the portable profile byte limit")]
    SegmentTooLong,
    #[error("relative path exceeds the portable profile byte limit")]
    PathTooLong,
}

impl PathSegment {
    pub fn new(value: impl Into<String>) -> Result<Self, PathError> {
        Self::validate(value.into(), false)
    }
    pub fn new_portable(value: impl Into<String>) -> Result<Self, PathError> {
        Self::validate(value.into(), true)
    }
    /// Returns the repository-portable sibling comparison key.
    ///
    /// This is Unicode Default Case Folding (full, non-Turkic) using the
    /// Unicode 9.0.0 data pinned by `unicode-casefold` 0.2.0. The result is
    /// normalized back to NFC so canonically equivalent fold expansions have
    /// one deterministic key.
    #[must_use]
    pub fn portable_case_fold(&self) -> String {
        portable_case_fold(&self.0)
    }
    /// Revalidates this segment against the portable filesystem profile.
    pub fn ensure_portable(&self) -> Result<(), PathError> {
        Self::validate(self.0.clone(), true).map(|_| ())
    }
    fn validate(value: String, portable: bool) -> Result<Self, PathError> {
        if value.is_empty() {
            return Err(PathError::Empty);
        }
        if value.len() > PORTABLE_COMPONENT_MAX_BYTES {
            return Err(PathError::SegmentTooLong);
        }
        if !value.nfc().eq(value.chars()) {
            return Err(PathError::NonNormalized);
        }
        if matches!(value.as_str(), "." | "..") || value.contains(['\0', '/', '\\']) {
            return Err(PathError::Unsafe);
        }
        if portable {
            if value.chars().any(is_nonportable_character) {
                return Err(PathError::NonPortableCharacter);
            }
            if !value.trim_end_matches([' ', '.']).eq(&value)
                || is_windows_reserved(&portable_case_fold(&value))
            {
                return Err(PathError::WindowsReserved);
            }
        }
        Ok(Self(value))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        Value::Text(self.0.clone())
            .encode()
            .expect("validated path segment always has a canonical encoding")
    }
}
impl fmt::Debug for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl From<&PathSegment> for Value {
    fn from(path: &PathSegment) -> Self {
        Self::Text(path.0.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortablePath(Vec<PathSegment>);
impl PortablePath {
    pub fn new(segments: Vec<PathSegment>) -> Result<Self, PathError> {
        let mut encoded_len = 0_usize;
        for segment in &segments {
            segment.ensure_portable()?;
            encoded_len = encoded_len
                .checked_add(usize::from(encoded_len != 0))
                .and_then(|length| length.checked_add(segment.as_str().len()))
                .ok_or(PathError::PathTooLong)?;
            if encoded_len > PORTABLE_PATH_MAX_BYTES {
                return Err(PathError::PathTooLong);
            }
        }
        Ok(Self(segments))
    }
    #[must_use]
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }
    pub(crate) fn value(&self) -> Value {
        Value::Array(self.0.iter().map(Value::from).collect())
    }
}

fn is_windows_reserved(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or(value);
    matches!(
        stem,
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "com¹"
            | "com²"
            | "com³"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
            | "lpt¹"
            | "lpt²"
            | "lpt³"
    )
}

fn portable_case_fold(value: &str) -> String {
    value.case_fold().collect::<String>().nfc().collect()
}

fn is_nonportable_character(character: char) -> bool {
    matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        || matches!(character as u32, 0x00..=0x1f | 0x7f)
}
