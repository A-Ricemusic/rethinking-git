//! Pure graph algorithms for RGit.
//!
//! This crate deliberately has no filesystem, object-store, authorization, or
//! command-line dependencies. Callers must authorize and materialize canonical
//! inputs before invoking these algorithms.

mod diff;
mod merge;
mod reachability;
mod tree;

pub use diff::{DiffError, DiffInput, FileDiff, OpaqueEntry, OpaqueEntryId, diff};
pub use merge::{
    ConflictKind, ConflictSide, MergeConflict, MergeError, MergePlan, merge_three_way,
};
pub use reachability::{GenerationError, GraphError, GraphIndex, GraphNode, next_generation};
pub use tree::{TreeEntry, TreeEntryError};
