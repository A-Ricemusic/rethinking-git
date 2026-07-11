use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use rgit_objects::{ObjectId, PortablePath};
use thiserror::Error;

use crate::{
    TreeEntry,
    tree::{PathKey, first_portable_collision},
};

/// A caller-generated, path-hiding identity for an entry the caller may not see.
///
/// The identifier must be stable for the same logical path within the two
/// compared views (for example, an HMAC under a view-specific key). It is never
/// returned by [`diff`], and its debug representation intentionally hides it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpaqueEntryId([u8; 32]);

impl OpaqueEntryId {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for OpaqueEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueEntryId(..)")
    }
}

/// A hidden entry fingerprint. It contains no path or displayable label.
/// Callers should fingerprint the complete canonical entry so target and policy
/// changes are both observable.
#[derive(Clone, PartialEq, Eq)]
pub struct OpaqueEntry {
    id: OpaqueEntryId,
    fingerprint: ObjectId,
}

impl fmt::Debug for OpaqueEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueEntry(..)")
    }
}

impl OpaqueEntry {
    #[must_use]
    pub const fn new(id: OpaqueEntryId, fingerprint: ObjectId) -> Self {
        Self { id, fingerprint }
    }
}

/// An already-authorized tree view supplied to the pure diff engine.
#[derive(Clone, PartialEq, Eq)]
pub struct DiffInput {
    visible: BTreeMap<PathKey, TreeEntry>,
    opaque: BTreeMap<OpaqueEntryId, ObjectId>,
}

impl fmt::Debug for DiffInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiffInput")
            .field("visible", &self.visible)
            .field("opaque_entries", &self.opaque.len())
            .finish()
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DiffError {
    #[error("visible diff input contains the same path more than once")]
    DuplicateVisiblePath,
    #[error(
        "visible diff input contains paths that cannot coexist portably: {first:?} and {second:?}"
    )]
    PortablePathCollision {
        first: PortablePath,
        second: PortablePath,
    },
    #[error("opaque diff input contains the same identity more than once")]
    DuplicateOpaqueIdentity,
    #[error("the opaque change count does not fit in u64")]
    OpaqueCountOverflow,
}

impl DiffInput {
    pub fn new(
        visible: impl IntoIterator<Item = TreeEntry>,
        opaque: impl IntoIterator<Item = OpaqueEntry>,
    ) -> Result<Self, DiffError> {
        let mut visible_by_path = BTreeMap::new();
        for entry in visible {
            if visible_by_path.insert(entry.path_key(), entry).is_some() {
                return Err(DiffError::DuplicateVisiblePath);
            }
        }
        if let Some((first, second)) = first_portable_collision(&visible_by_path) {
            return Err(DiffError::PortablePathCollision { first, second });
        }

        let mut opaque_by_id = BTreeMap::new();
        for entry in opaque {
            if opaque_by_id.insert(entry.id, entry.fingerprint).is_some() {
                return Err(DiffError::DuplicateOpaqueIdentity);
            }
        }

        Ok(Self {
            visible: visible_by_path,
            opaque: opaque_by_id,
        })
    }
}

/// A metadata-safe manifest diff.
///
/// Only authorized paths appear in the path lists. Hidden changes are exposed
/// solely as an aggregate count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub added: Vec<PortablePath>,
    pub modified: Vec<PortablePath>,
    pub deleted: Vec<PortablePath>,
    pub opaque_changed: u64,
}

pub fn diff(previous: &DiffInput, current: &DiffInput) -> Result<FileDiff, DiffError> {
    let mut result = FileDiff::default();

    let visible_paths = previous
        .visible
        .keys()
        .chain(current.visible.keys())
        .collect::<BTreeSet<_>>();
    for key in visible_paths {
        match (previous.visible.get(key), current.visible.get(key)) {
            (None, Some(after)) => result.added.push(after.path().clone()),
            (Some(before), None) => result.deleted.push(before.path().clone()),
            (Some(before), Some(after)) if before != after => {
                result.modified.push(after.path().clone());
            }
            _ => {}
        }
    }

    let opaque_changed = previous
        .opaque
        .keys()
        .chain(current.opaque.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|id| previous.opaque.get(id) != current.opaque.get(id))
        .count();
    result.opaque_changed =
        u64::try_from(opaque_changed).map_err(|_| DiffError::OpaqueCountOverflow)?;

    Ok(result)
}
