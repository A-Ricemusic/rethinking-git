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
    resolved_file(repo, &actor, &conflict, &decision)?;
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
            if let Some(file) = resolved_file(repo, actor, conflict, decision)? {
                plan.merged_files.push(file);
            }
        } else {
            unresolved.push(pending);
        }
    }
    plan.conflicts = unresolved;
    plan.merged_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(())
}
