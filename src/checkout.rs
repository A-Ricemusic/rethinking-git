use super::*;

fn change_snapshot(repo: &Repo, actor: &Actor) -> Result<Option<Snapshot>> {
    let workspace = read_workspace(repo)?;
    let Some(id) = workspace.current_change else {
        return Ok(None);
    };
    let change = read_change(repo, &id)?;
    if !can_access(actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    read_optional_snapshot(repo, change.workspace_base_snapshot_id())
}

// Ownership of materialized paths is independent of the change's ancestry.
// mode_snapshot already records every successful capture/checkout on all platforms.
fn current_snapshot(repo: &Repo, actor: &Actor) -> Result<Option<Snapshot>> {
    let logical = change_snapshot(repo, actor)?;
    match read_workspace(repo)?.mode_snapshot {
        Some(id) => Ok(Some(read_snapshot(repo, &id)?)),
        None => Ok(logical), // legacy repositories without a materialization record
    }
}

fn guard_restored_changes(repo: &Repo, actor: &Actor) -> Result<()> {
    let logical = change_snapshot(repo, actor)?;
    let materialized = current_snapshot(repo, actor)?;
    if logical.as_ref().map(|s| &s.id) == materialized.as_ref().map(|s| &s.id) {
        return Ok(());
    }
    let logical = verified_files(repo, actor, &logical)?;
    let materialized = verified_files(repo, actor, &materialized)?;
    if logical != materialized {
        bail!("workspace contains restored changes; snapshot them or use workspace restore --discard-changes before switching");
    }
    Ok(())
}

pub(super) fn start(repo: &Repo, name: &str, line_name: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let line = read_line(repo, line_name)?;
    if !can_access(&actor, &line.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    guard_restored_changes(repo, &actor)?;
    let before = current_snapshot(repo, &actor)?;
    let after = read_optional_snapshot(repo, line.head_snapshot.as_deref())?;
    stage_checkout(repo, &actor, &before, &after, false)?;
    create_change(repo, name, line_name, line.policy)?;
    let mut workspace = read_workspace(repo)?;
    workspace.mode_snapshot = after.map(|s| s.id);
    write_json(repo, &repo.path(&["workspace.json"]), &workspace)?;
    record_operation(
        repo,
        OperationKind::SwitchWorkspace {
            change_id: workspace.current_change.context("new change is missing")?,
        },
        admin_policy(),
        format!("started change at line `{line_name}`"),
        None,
    )?;
    println!("checked out {line_name}; new change is ready");
    Ok(())
}

pub(super) fn switch(repo: &Repo, id: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let change = read_change(repo, id)?;
    if !can_access(&actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    guard_restored_changes(repo, &actor)?;
    let before = current_snapshot(repo, &actor)?;
    let after = read_optional_snapshot(repo, change.workspace_base_snapshot_id())?;
    stage_checkout(repo, &actor, &before, &after, false)?;
    write_json(
        repo,
        &repo.path(&["workspace.json"]),
        &Workspace {
            mode_snapshot: after.as_ref().map(|s| s.id.clone()),
            current_change: Some(id.to_string()),
        },
    )?;
    record_operation(
        repo,
        OperationKind::SwitchWorkspace {
            change_id: id.to_string(),
        },
        admin_policy(),
        format!("switched workspace to `{id}`"),
        None,
    )?;
    println!("switched workspace to {id}");
    Ok(())
}

pub(super) fn restore(
    repo: &Repo,
    from: Option<&str>,
    discard: bool,
    actor_name: &str,
) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    if !discard {
        guard_restored_changes(repo, &actor)?;
    }
    let before = current_snapshot(repo, &actor)?;
    let after = if let Some(id) = from {
        Some(read_snapshot(repo, id)?)
    } else {
        change_snapshot(repo, &actor)?
    };
    if after.is_none() {
        bail!("workspace has no snapshot to restore");
    }
    stage_checkout(repo, &actor, &before, &after, discard)?;
    let mut workspace = read_workspace(repo)?;
    workspace.mode_snapshot = after.as_ref().map(|s| s.id.clone());
    write_json(repo, &repo.path(&["workspace.json"]), &workspace)?;
    record_operation(
        repo,
        OperationKind::RestoreWorkspace {
            snapshot_id: after.as_ref().map(|s| s.id.clone()),
        },
        admin_policy(),
        "restored working files".to_string(),
        None,
    )?;
    println!("restored working files; snapshot to record these contents");
    Ok(())
}

fn verified_files(
    repo: &Repo,
    actor: &Actor,
    snapshot: &Option<Snapshot>,
) -> Result<BTreeMap<String, (Vec<u8>, transaction::WorkingFlags)>> {
    let Some(snapshot) = snapshot else {
        return Ok(BTreeMap::new());
    };
    if !can_access(actor, &snapshot.policy)
        || snapshot
            .files
            .iter()
            .any(|file| !can_access(actor, &file.policy))
    {
        return Err(CliFailure::OperationUnavailable.into());
    }
    if manifest_hash(&snapshot.files)? != snapshot.manifest_hash {
        bail!("snapshot manifest failed verification");
    }
    let mut files = BTreeMap::new();
    let blobs = repo.path(&["blobs"]);
    let directory = fs::symlink_metadata(&blobs)?;
    if !directory.is_dir() || directory.file_type().is_symlink() {
        bail!("blob directory is unsafe");
    }
    for file in &snapshot.files {
        transaction::validate_working_key(&file.path)?;
        if file.hash.len() != 64
            || !file
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("invalid blob identifier");
        }
        let path = blobs.join(&file.hash);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("stored blob is unsafe");
        }
        let bytes = fs::read(&path)?;
        if bytes.len() as u64 != file.bytes || hash_bytes(&bytes) != file.hash {
            bail!("stored blob failed verification");
        }
        if file.symlink {
            transaction::validate_link(&bytes)?;
            if file.executable {
                bail!("symlink cannot be executable");
            }
        }
        if files
            .insert(file.path.clone(), (bytes, file.flags()))
            .is_some()
        {
            bail!("snapshot contains duplicate paths");
        }
    }
    for path in files.keys() {
        let mut ancestor = Path::new(path).parent();
        while let Some(parent) = ancestor {
            if files.contains_key(parent.to_str().context("invalid snapshot path")?) {
                bail!("snapshot contains a file/directory path collision");
            }
            ancestor = parent.parent();
        }
    }
    Ok(files)
}

fn stage_checkout(
    repo: &Repo,
    actor: &Actor,
    before: &Option<Snapshot>,
    after: &Option<Snapshot>,
    discard: bool,
) -> Result<()> {
    let before = verified_files(repo, actor, before)?;
    let after = verified_files(repo, actor, after)?;
    #[cfg(not(unix))]
    let current_modes =
        read_optional_snapshot(repo, read_workspace(repo)?.mode_snapshot.as_deref())?.map(
            |snapshot| {
                snapshot
                    .files
                    .iter()
                    .map(|file| (file.path.clone(), file.flags()))
                    .collect()
            },
        );
    #[cfg(unix)]
    let current_modes = None;
    repo.transaction
        .stage_checkout(&before, &after, current_modes.as_ref(), discard)
}
