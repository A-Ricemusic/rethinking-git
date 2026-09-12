use super::*;

fn current_snapshot(repo: &Repo, actor: &Actor) -> Result<Option<Snapshot>> {
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

pub(super) fn switch(repo: &Repo, id: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let change = read_change(repo, id)?;
    if !can_access(&actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
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
    let before = current_snapshot(repo, &actor)?;
    let after = if let Some(id) = from {
        Some(read_snapshot(repo, id)?)
    } else {
        before.clone()
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
        transaction::working_path(&repo.root, &file.path)?;
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
    let materialized =
        read_optional_snapshot(repo, read_workspace(repo)?.mode_snapshot.as_deref())?;
    let paths: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    checkout_paths::validate(paths.iter().map(|path| path.as_str()))?;
    checkout_paths::validate_existing(&repo.root, paths.iter().map(|path| path.as_str()))?;
    for path in paths {
        let current = transaction::read_working(&repo.root, path)?;
        let previous = before.get(path).map(|v| &v.0);
        let target = after.get(path).map(|v| &v.0);
        let current_flags = transaction::working_flags(&repo.root.join(path))?;
        let before_flags = before.get(path).map(|v| v.1).unwrap_or_default();
        let after_flags = after.get(path).map(|v| v.1).unwrap_or_default();
        #[cfg(not(unix))]
        let current_flags = if current.is_some() {
            let saved = materialized
                .as_ref()
                .and_then(|s| s.files.iter().find(|f| f.path == *path));
            let mut flags = if materialized.is_some() {
                saved.map(FileEntry::flags).unwrap_or_default()
            } else {
                before_flags
            };
            flags.symlink |= current_flags.symlink;
            flags
        } else {
            current_flags
        };
        if previous.is_none() && current.is_some() {
            bail!("checkout would overwrite an untracked file: {path}");
        }
        if !discard && (current.as_ref() != previous || current_flags != before_flags) {
            bail!("tracked file has local changes: {path}; snapshot first or explicitly restore with --discard-changes");
        }
        if current.as_ref() != target || current_flags != after_flags {
            repo.transaction.stage_working(
                path,
                current,
                target.cloned(),
                current_flags,
                after_flags,
            )?;
        }
    }
    Ok(())
}
