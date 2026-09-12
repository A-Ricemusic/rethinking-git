use super::*;

fn matches_sources(conflict: &Conflict, line: &Line, change: &Change, incoming: &Snapshot) -> bool {
    conflict.line == line.name
        && conflict.change_id == change.id
        && conflict.base_snapshot == change.base_snapshot
        && conflict.line_snapshot == line.head_snapshot
        && conflict.incoming_snapshot == incoming.id
}

pub(super) fn resolve_conflict(
    repo: &Repo,
    id: &str,
    decision: Resolution,
    actor_name: &str,
) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let mut conflict = read_conflict(repo, id).map_err(|_| CliFailure::OperationUnavailable)?;
    if !can_access_conflict(&actor, &conflict) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    let mut change = read_change(repo, &conflict.change_id)?;
    let line = read_line(repo, &conflict.line)?;
    let incoming = read_snapshot(repo, &conflict.incoming_snapshot)?;
    if !can_access(&actor, &change.policy) || !can_access(&actor, &line.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    change.base_snapshot = ancestry::merge_base(
        repo,
        line.head_snapshot.as_deref(),
        &incoming,
        change.base_snapshot.as_deref(),
    )?
    .map(|snapshot| snapshot.id);
    if !matches_sources(&conflict, &line, &change, &incoming)
        || change.current_snapshot.as_deref() != Some(incoming.id.as_str())
    {
        bail!("conflict sources changed; integrate again to obtain current conflicts");
    }
    // Recheck every source grant before reading any working content.
    resolved_files(repo, &actor, &conflict, &Resolution::Incoming)?;
    if conflict.kind == ConflictKind::FileDirectory && decision == Resolution::Custom {
        bail!("file/directory conflicts require --take base, line, incoming, or delete; edit and snapshot a side for custom content");
    }
    conflict.replacement = if decision == Resolution::Custom {
        let policy = policy_for_path(&conflict.path, &read_path_policies(repo)?);
        if !can_access(&actor, &policy) {
            return Err(CliFailure::OperationUnavailable.into());
        }
        let path = transaction::working_path(&repo.root, &conflict.path)?;
        let bytes = transaction::read_working(&repo.root, &conflict.path)?.context(
            "custom resolution file is missing; use --take delete to resolve a deletion",
        )?;
        let mut flags = transaction::working_flags(&path)?;
        // Windows materializes logical symlinks/executable modes as regular files.
        if !cfg!(unix) {
            let workspace = read_workspace(repo)?;
            let modes = read_optional_snapshot(repo, workspace.mode_snapshot.as_deref())?;
            let modes = modes.as_ref().unwrap_or(&incoming);
            if let Some(file) = modes.files.iter().find(|f| f.path == conflict.path) {
                flags = file.flags();
            }
        }
        if flags.symlink {
            transaction::validate_link(&bytes)?;
        }
        let hash = hash_bytes(&bytes);
        let blob = repo.path(&["blobs", &hash]);
        if blob.try_exists()? {
            if verify::read_blob(repo, &hash)? != bytes {
                bail!("custom resolution blob failed verification");
            }
        } else {
            transaction::publish_file(&blob, &bytes)?;
        }
        Some(FileEntry {
            path: conflict.path.clone(),
            hash,
            bytes: bytes.len() as u64,
            executable: flags.executable,
            symlink: flags.symlink,
            policy,
        })
    } else {
        None
    };
    // Persist the conservative effective policy alongside the custom content.
    if decision == Resolution::Custom {
        conflict.replacement = resolved_file(repo, &actor, &conflict, &decision)?;
    } else {
        resolved_files(repo, &actor, &conflict, &decision)?;
    }
    conflict.status = ConflictStatus::Resolved;
    conflict.resolution = Some(decision);
    write_json(repo, &conflict_path(repo, id)?, &conflict)?;
    record_operation(
        repo,
        OperationKind::ResolveConflict {
            conflict_id: id.to_string(),
        },
        admin_policy(),
        format!("resolved conflict `{id}`"),
        None,
    )?;
    println!("resolved {id}; run line integrate to publish the merge");
    Ok(())
}

fn resolved_files(
    repo: &Repo,
    actor: &Actor,
    conflict: &Conflict,
    decision: &Resolution,
) -> Result<Vec<FileEntry>> {
    if conflict.kind == ConflictKind::FileDirectory {
        tree_conflicts::resolved_files(repo, actor, conflict, decision)
    } else {
        Ok(resolved_file(repo, actor, conflict, decision)?
            .into_iter()
            .collect())
    }
}

fn resolved_file(
    repo: &Repo,
    actor: &Actor,
    conflict: &Conflict,
    decision: &Resolution,
) -> Result<Option<FileEntry>> {
    let base = read_optional_snapshot(repo, conflict.base_snapshot.as_deref())?;
    let line = read_optional_snapshot(repo, conflict.line_snapshot.as_deref())?;
    let incoming = Some(read_snapshot(repo, &conflict.incoming_snapshot)?);
    let snapshots = [&base, &line, &incoming];
    let sides: Vec<Option<FileEntry>> = snapshots
        .iter()
        .map(|snapshot| {
            snapshot
                .as_ref()
                .and_then(|s| s.files.iter().find(|f| f.path == conflict.path))
                .cloned()
        })
        .collect();
    if snapshots
        .iter()
        .any(|s| s.as_ref().is_some_and(|s| !can_access(actor, &s.policy)))
        || sides
            .iter()
            .flatten()
            .any(|file| !can_access(actor, &file.policy))
    {
        return Err(CliFailure::OperationUnavailable.into());
    }
    let chosen = match decision {
        Resolution::Base => sides[0].clone(),
        Resolution::Line => sides[1].clone(),
        Resolution::Incoming => sides[2].clone(),
        Resolution::Delete => None,
        Resolution::Custom => {
            let file = conflict
                .replacement
                .clone()
                .context("custom resolution content missing")?;
            verify::verify_file(repo, &file)?;
            if file.path != conflict.path {
                bail!("custom resolution path mismatch");
            }
            if !can_access(actor, &file.policy) {
                return Err(CliFailure::OperationUnavailable.into());
            }
            Some(file)
        }
    };
    Ok(chosen.map(|mut file| {
        // Choosing content must not silently undo a concurrent access restriction.
        if sides
            .iter()
            .flatten()
            .any(|side| side.policy != file.policy)
        {
            file.policy = admin_policy();
        }
        file
    }))
}

pub(super) fn apply_resolutions(
    repo: &Repo,
    actor: &Actor,
    line: &Line,
    change: &Change,
    incoming: &Snapshot,
    plan: &mut MergePlan,
) -> Result<()> {
    if plan.conflicts.is_empty() {
        return Ok(());
    }
    let stored = read_dir_json::<Conflict>(repo, &repo.path(&["conflicts"]))?;
    let mut unresolved = Vec::new();
    for pending in std::mem::take(&mut plan.conflicts) {
        let matches: Vec<&Conflict> = stored
            .iter()
            .filter(|conflict| {
                conflict.status == ConflictStatus::Resolved
                    && conflict.path == pending.path
                    && conflict.kind == pending.kind
                    && matches_sources(conflict, line, change, incoming)
            })
            .collect();
        if matches.len() > 1 {
            bail!("multiple resolutions match the same conflict sources");
        }
        if let Some(conflict) = matches.first() {
            if !can_access_conflict(actor, conflict) {
                return Err(CliFailure::OperationUnavailable.into());
            }
            let decision = conflict
                .resolution
                .as_ref()
                .context("resolved conflict has no decision")?;
            plan.merged_files
                .extend(resolved_files(repo, actor, conflict, decision)?);
        } else {
            unresolved.push(pending);
        }
    }
    plan.conflicts = unresolved;
    plan.merged_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(())
}
