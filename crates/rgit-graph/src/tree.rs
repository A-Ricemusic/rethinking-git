use std::collections::BTreeMap;

use rgit_objects::{ManifestTarget, PolicyRef, PortablePath};
use thiserror::Error;

/// Stable full-path ordering key without requiring canonical object types to
/// expose collection-specific ordering semantics.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PathKey(Vec<Vec<u8>>);

/// Returns the first deterministic pair of paths that cannot coexist in one
/// portable materialized tree.
///
/// Besides exact file/descendant overlaps, this rejects two different
/// spellings of the same portable sibling at any shared directory level. It
/// intentionally does not compare equal leaf names under different parents.
pub(crate) fn first_portable_collision(
    entries: &BTreeMap<PathKey, TreeEntry>,
) -> Option<(PortablePath, PortablePath)> {
    let mut root = PortableTreeNode::default();
    for entry in entries.values() {
        let current_path = entry.path();
        let mut node = &mut root;
        for segment in current_path.segments() {
            if let Some(ancestor) = &node.terminal {
                return Some((ancestor.clone(), current_path.clone()));
            }

            let folded = segment.portable_case_fold();
            let child = node
                .children
                .entry(folded)
                .or_insert_with(|| PortableTreeChild {
                    spelling: segment.as_str().to_owned(),
                    representative: current_path.clone(),
                    node: PortableTreeNode::default(),
                });
            if child.spelling != segment.as_str() {
                return Some((child.representative.clone(), current_path.clone()));
            }
            node = &mut child.node;
        }

        // Exact duplicate paths were rejected while constructing `entries`.
        // Reaching an existing subtree means this file would replace a
        // directory required by an earlier entry.
        if !node.children.is_empty() {
            let descendant = node
                .children
                .values()
                .map(|child| &child.representative)
                .min_by_key(|path| path_key_bytes(path))
                .expect("a nonempty child map has a representative");
            return Some((current_path.clone(), descendant.clone()));
        }
        node.terminal = Some(current_path.clone());
    }
    None
}

#[derive(Default)]
struct PortableTreeNode {
    terminal: Option<PortablePath>,
    children: BTreeMap<String, PortableTreeChild>,
}

struct PortableTreeChild {
    spelling: String,
    representative: PortablePath,
    node: PortableTreeNode,
}

fn path_key_bytes(path: &PortablePath) -> Vec<&[u8]> {
    path.segments()
        .iter()
        .map(|segment| segment.as_str().as_bytes())
        .collect()
}

/// A canonical manifest target at a full repository-relative path.
///
/// Unlike [`rgit_objects::ManifestEntry`], which represents one directory
/// level, this type is flattened for tree algorithms. The target and policy
/// reference are retained verbatim when an entry is selected by diff or merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    path: PortablePath,
    target: ManifestTarget,
    policy_ref: PolicyRef,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TreeEntryError {
    #[error("a tree entry path must contain at least one segment")]
    EmptyPath,
}

impl TreeEntry {
    pub fn new(
        path: PortablePath,
        target: ManifestTarget,
        policy_ref: PolicyRef,
    ) -> Result<Self, TreeEntryError> {
        if path.segments().is_empty() {
            return Err(TreeEntryError::EmptyPath);
        }
        Ok(Self {
            path,
            target,
            policy_ref,
        })
    }

    #[must_use]
    pub const fn path(&self) -> &PortablePath {
        &self.path
    }

    #[must_use]
    pub const fn target(&self) -> &ManifestTarget {
        &self.target
    }

    #[must_use]
    pub const fn policy_ref(&self) -> &PolicyRef {
        &self.policy_ref
    }

    pub(crate) fn path_key(&self) -> PathKey {
        PathKey(
            self.path
                .segments()
                .iter()
                .map(|segment| segment.as_str().as_bytes().to_vec())
                .collect(),
        )
    }
}
