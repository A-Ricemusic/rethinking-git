use std::collections::{BTreeMap, BTreeSet};

use rgit_objects::{ManifestTarget, PolicyRef, PortablePath};
use thiserror::Error;

use crate::{
    TreeEntry,
    tree::{PathKey, first_portable_collision},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    AddAdd,
    BothModified,
    DeleteModify,
}

/// One immutable side of a merge conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictSide {
    pub target: ManifestTarget,
    pub policy_ref: PolicyRef,
}

impl From<&TreeEntry> for ConflictSide {
    fn from(entry: &TreeEntry) -> Self {
        Self {
            target: entry.target().clone(),
            policy_ref: entry.policy_ref().clone(),
        }
    }
}

/// A path-level conflict with every source reference preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeConflict {
    pub path: PortablePath,
    pub kind: ConflictKind,
    pub base: Option<ConflictSide>,
    pub line: Option<ConflictSide>,
    pub incoming: Option<ConflictSide>,
    /// Distinct policy references in line, incoming, base order. A policy layer
    /// must derive an output label covering all of these references.
    pub source_policy_refs: Vec<PolicyRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergePlan {
    pub merged_entries: Vec<TreeEntry>,
    pub conflicts: Vec<MergeConflict>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    #[error("{input} merge input contains the same path more than once")]
    DuplicatePath { input: &'static str },
    #[error(
        "{input} merge tree contains paths that cannot coexist portably: {first:?} and {second:?}"
    )]
    PortablePathCollision {
        input: &'static str,
        first: PortablePath,
        second: PortablePath,
    },
}

pub fn merge_three_way(
    base: impl IntoIterator<Item = TreeEntry>,
    line: impl IntoIterator<Item = TreeEntry>,
    incoming: impl IntoIterator<Item = TreeEntry>,
) -> Result<MergePlan, MergeError> {
    let base = entries_by_path("base", base)?;
    let line = entries_by_path("line", line)?;
    let incoming = entries_by_path("incoming", incoming)?;
    let all_paths = base
        .keys()
        .chain(line.keys())
        .chain(incoming.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut plan = MergePlan::default();
    for key in all_paths {
        let base_entry = base.get(&key);
        let line_entry = line.get(&key);
        let incoming_entry = incoming.get(&key);

        if line_entry == incoming_entry {
            if let Some(entry) = line_entry {
                plan.merged_entries.push((*entry).clone());
            }
            continue;
        }

        let line_changed = base_entry != line_entry;
        let incoming_changed = base_entry != incoming_entry;
        match (line_changed, incoming_changed) {
            (false, true) => {
                if let Some(entry) = incoming_entry {
                    plan.merged_entries.push((*entry).clone());
                }
            }
            (true, false) => {
                if let Some(entry) = line_entry {
                    plan.merged_entries.push((*entry).clone());
                }
            }
            (false, false) => {
                if let Some(entry) = base_entry {
                    plan.merged_entries.push((*entry).clone());
                }
            }
            (true, true) => {
                let Some(source) = line_entry.or(incoming_entry).or(base_entry) else {
                    // The key came from the union of these maps, but retain a
                    // total implementation if that construction ever changes.
                    continue;
                };
                plan.conflicts.push(conflict(
                    source.path().clone(),
                    base_entry,
                    line_entry,
                    incoming_entry,
                ));
            }
        }
    }

    let merged = entries_by_path("merged result", plan.merged_entries.iter().cloned())?;
    if let Some((first, second)) = first_portable_collision(&merged) {
        return Err(MergeError::PortablePathCollision {
            input: "merged result",
            first,
            second,
        });
    }

    Ok(plan)
}

fn entries_by_path(
    input: &'static str,
    entries: impl IntoIterator<Item = TreeEntry>,
) -> Result<BTreeMap<PathKey, TreeEntry>, MergeError> {
    let mut by_path = BTreeMap::new();
    for entry in entries {
        if by_path.insert(entry.path_key(), entry).is_some() {
            return Err(MergeError::DuplicatePath { input });
        }
    }
    if let Some((first, second)) = first_portable_collision(&by_path) {
        return Err(MergeError::PortablePathCollision {
            input,
            first,
            second,
        });
    }
    Ok(by_path)
}

fn conflict(
    path: PortablePath,
    base: Option<&TreeEntry>,
    line: Option<&TreeEntry>,
    incoming: Option<&TreeEntry>,
) -> MergeConflict {
    let kind = match (base, line, incoming) {
        (None, Some(_), Some(_)) => ConflictKind::AddAdd,
        (Some(_), None, Some(_)) | (Some(_), Some(_), None) => ConflictKind::DeleteModify,
        _ => ConflictKind::BothModified,
    };
    let mut source_policy_refs = Vec::new();
    for entry in [line, incoming, base].into_iter().flatten() {
        if !source_policy_refs.contains(entry.policy_ref()) {
            source_policy_refs.push(entry.policy_ref().clone());
        }
    }
    MergeConflict {
        path,
        kind,
        base: base.map(ConflictSide::from),
        line: line.map(ConflictSide::from),
        incoming: incoming.map(ConflictSide::from),
        source_policy_refs,
    }
}
