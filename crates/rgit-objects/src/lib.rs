//! Canonical, immutable logical objects used by RGit.
//!
//! The binary representation is a protocol boundary. [`CanonicalObject::encode`]
//! is the only representation that may be hashed. JSON output is diagnostic.

mod canonical;
mod decode;
mod extended;
mod fastcdc;
mod id;
mod object;
mod path;
mod types;

pub use canonical::{
    BULK_MAX_BYTE_STRING_BYTES, BULK_MAX_COLLECTION_ITEMS, BULK_MAX_DEPTH, BULK_MAX_ENCODED_BYTES,
    BULK_MAX_TEXT_STRING_BYTES, CanonicalError, CanonicalLimits, METADATA_MAX_BYTE_STRING_BYTES,
    METADATA_MAX_COLLECTION_ITEMS, METADATA_MAX_DEPTH, METADATA_MAX_ENCODED_BYTES,
    METADATA_MAX_TEXT_STRING_BYTES, Value, decode_canonical,
};
pub use decode::{
    AnyObject, BoundReferenceTransition, DecodeObjectError, DecodedObject, ReferenceEdge,
    ReferenceRole,
};
pub use extended::*;
pub use fastcdc::*;
pub use id::{HashAlgorithm, ObjectId};
pub use object::*;
pub use path::{
    PORTABLE_COMPONENT_MAX_BYTES, PORTABLE_PATH_MAX_BYTES, PathError, PathSegment, PortablePath,
};
pub use types::*;

/// Schema version shared by all initial object schemas.
pub const SCHEMA_VERSION_0: u64 = 0;
/// Schema version of the key-bound Operation format.
pub const OPERATION_SCHEMA_VERSION_1: u64 = 1;
