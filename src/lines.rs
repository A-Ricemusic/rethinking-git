use super::*;

pub(super) fn create(repo: &Repo, name: &str, from: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let source = read_line(repo, from)?;
    if !can_access(&actor, &source.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    let path = line_path(repo, name)?;
    if path.try_exists()? {
        bail!("line already exists");
    }
    let mut policy = source.policy;
    if let Some(id) = &source.head_snapshot {
        let snapshot = read_snapshot(repo, id)?;
        let owner = read_change(repo, &snapshot.change_id)?;
        if !can_access(&actor, &snapshot.policy) || !can_access(&actor, &owner.policy) {
            return Err(CliFailure::OperationUnavailable.into());
        }
        verify_target(repo, &snapshot)?;
        if policy != snapshot.policy || policy != owner.policy {
            policy = admin_policy();
        }
    }
    let line = Line {
        name: name.to_string(),
        head_snapshot: source.head_snapshot,
        policy,
        created_at: now()?,
    };
    write_json(repo, &path, &line)?;
    record_operation(
        repo,
        OperationKind::CreateLine {
            line: name.to_string(),
            source_line: from.to_string(),
            snapshot_id: line.head_snapshot.clone(),
        },
        admin_policy(),
        format!("created line `{name}` from `{from}`"),
        None,
    )?;
    println!("created line {name}; working files are unchanged");
    Ok(())
}

pub(super) fn reset(
    repo: &Repo,
    name: &str,
    to: &str,
    expected: &str,
    actor_name: &str,
) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let mut line = read_line(repo, name)?;
    let snapshot = read_snapshot(repo, to)?;
    let owner = read_change(repo, &snapshot.change_id)?;
    if !can_access(&actor, &line.policy)
        || !can_access(&actor, &snapshot.policy)
        || !can_access(&actor, &owner.policy)
    {
        return Err(CliFailure::OperationUnavailable.into());
    }
    validate_object_id(expected, "snap_")?;
    if line.head_snapshot.as_deref() != Some(expected) {
        bail!("line head changed; inspect the current head before retrying reset");
    }
    if expected == to {
        println!("line already points to the requested snapshot");
        return Ok(());
    }
    verify_target(repo, &snapshot)?;
    if line.policy != snapshot.policy || line.policy != owner.policy {
        line.policy = admin_policy();
    }
    line.head_snapshot = Some(to.to_string());
    write_json(repo, &line_path(repo, name)?, &line)?;
    record_operation(
        repo,
        OperationKind::ResetLine {
            line: name.to_string(),
            previous_snapshot: expected.to_string(),
            snapshot_id: to.to_string(),
        },
        admin_policy(),
        format!("reset line `{name}` from `{expected}` to `{to}`"),
        None,
    )?;
    println!("reset line {name}; working files are unchanged");
    println!("previous head: {expected}");
    Ok(())
}

fn verify_target(repo: &Repo, snapshot: &Snapshot) -> Result<()> {
    if manifest_hash(&snapshot.files)? != snapshot.manifest_hash {
        bail!("line target manifest failed verification");
    }
    for file in &snapshot.files {
        verify::verify_file(repo, file)?;
    }
    Ok(())
}
