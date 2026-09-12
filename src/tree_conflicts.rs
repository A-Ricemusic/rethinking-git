//! A file/directory conflict is resolved as one complete subtree, so a decision
//! cannot combine a leaf from one side with descendants from another side.
use super::*;

pub(super) fn contains(root: &str, path: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn group(plan: &mut MergePlan, sources: [&BTreeMap<String, FileEntry>; 3]) {
    let paths: BTreeSet<_> = plan
        .merged_files
        .iter()
        .map(|file| file.path.as_str())
        .chain(plan.conflicts.iter().map(|conflict| conflict.path.as_str()))
        .collect();
    let mut roots = BTreeSet::new();
    for path in &paths {
        for (index, _) in path.match_indices('/') {
            let ancestor = &path[..index];
            if paths.contains(ancestor) {
                roots.insert(ancestor.to_string());
                break;
            }
        }
    }
    for root in roots {
        let mut policies = Vec::new();
        for file in sources
            .iter()
            .flat_map(|source| source.values())
            .filter(|file| contains(&root, &file.path))
        {
            if !policies.contains(&file.policy) {
                policies.push(file.policy.clone());
            }
        }
        let domains: BTreeSet<_> = policies
            .iter()
            .flat_map(|policy| policy.domains.iter().cloned())
            .collect();
        plan.merged_files
            .retain(|file| !contains(&root, &file.path));
        plan.conflicts
            .retain(|conflict| !contains(&root, &conflict.path));
        plan.conflicts.push(PendingConflict {
            path: root,
            kind: ConflictKind::FileDirectory,
            policy: AccessPolicy {
                domains: domains.into_iter().collect(),
                redaction: Redaction::Placeholder,
            },
            file_policies: policies,
        });
    }
    plan.conflicts.sort_by(|a, b| a.path.cmp(&b.path));
}

pub(super) fn resolved_files(
    repo: &Repo,
    actor: &Actor,
    conflict: &Conflict,
    decision: &Resolution,
) -> Result<Vec<FileEntry>> {
    if *decision == Resolution::Custom {
        bail!("file/directory conflicts require --take base, line, incoming, or delete; edit and snapshot a side for custom content");
    }
    let snapshots = [
        read_optional_snapshot(repo, conflict.base_snapshot.as_deref())?,
        read_optional_snapshot(repo, conflict.line_snapshot.as_deref())?,
        Some(read_snapshot(repo, &conflict.incoming_snapshot)?),
    ];
    let sides: Vec<Vec<FileEntry>> = snapshots
        .iter()
        .map(|snapshot| {
            snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .files
                        .iter()
                        .filter(|file| contains(&conflict.path, &file.path))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    if snapshots
        .iter()
        .flatten()
        .any(|snapshot| !can_access(actor, &snapshot.policy))
        || sides
            .iter()
            .flatten()
            .any(|file| !can_access(actor, &file.policy))
    {
        return Err(CliFailure::OperationUnavailable.into());
    }
    let mut chosen = match decision {
        Resolution::Base => sides[0].clone(),
        Resolution::Line => sides[1].clone(),
        Resolution::Incoming => sides[2].clone(),
        Resolution::Delete => Vec::new(),
        Resolution::Custom => unreachable!(),
    };
    for file in &mut chosen {
        // Conservatively preserve restrictions across a structural change, even
        // when the restriction belonged to a descendant that is being replaced.
        if sides
            .iter()
            .flatten()
            .any(|side| side.policy != file.policy)
        {
            file.policy = admin_policy();
        }
    }
    verify::verify_manifest(repo, &chosen)?;
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_small_valid_tree_combinations_produce_disjoint_files_and_conflict_groups() {
        let paths = ["a", "a/b", "a/b/c", "d", "d/e", "f"];
        let trees: Vec<Vec<FileEntry>> = (0..1 << paths.len())
            .filter_map(|mask| {
                let selected: Vec<_> = paths
                    .iter()
                    .enumerate()
                    .filter_map(|(i, path)| (mask & (1 << i) != 0).then_some(*path))
                    .collect();
                if selected
                    .iter()
                    .any(|a| selected.iter().any(|b| a != b && contains(a, b)))
                {
                    return None;
                }
                Some(
                    selected
                        .into_iter()
                        .map(|path| FileEntry {
                            path: path.to_string(),
                            hash: "same-content".to_string(),
                            bytes: 1,
                            executable: false,
                            symlink: false,
                            policy: public_policy(),
                        })
                        .collect(),
                )
            })
            .collect();
        for base in &trees {
            for line in &trees {
                for incoming in &trees {
                    let plan = plan_merge(base.clone(), line.clone(), incoming.clone());
                    let outputs: Vec<_> = plan
                        .merged_files
                        .iter()
                        .map(|file| &file.path)
                        .chain(plan.conflicts.iter().map(|conflict| &conflict.path))
                        .collect();
                    for (i, a) in outputs.iter().enumerate() {
                        for b in &outputs[i + 1..] {
                            assert!(
                                !contains(a, b) && !contains(b, a),
                                "{base:?} {line:?} {incoming:?}"
                            );
                        }
                    }
                    if base == line || base == incoming || line == incoming {
                        assert!(plan.conflicts.is_empty());
                        assert_eq!(
                            &plan.merged_files,
                            if base == line { incoming } else { line }
                        );
                    }
                }
            }
        }
    }
}
