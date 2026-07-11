//! Canonical, immutable logical objects used by RGit.
//!
//! The binary representation is a protocol boundary. [`CanonicalObject::encode`]
//! is the only representation that may be hashed. JSON output is diagnostic.

mod canonical;
mod decode;
mod id;
mod object;
mod path;
mod types;

pub use canonical::{CanonicalError, CanonicalLimits, Value, decode_canonical};
pub use decode::{AnyObject, DecodeObjectError, DecodedObject, ReferenceEdge, ReferenceRole};
pub use id::{HashAlgorithm, ObjectId};
pub use object::*;
pub use path::{PathSegment, PortablePath};
pub use types::*;

/// Schema version shared by all initial object schemas.
pub const SCHEMA_VERSION_0: u64 = 0;
