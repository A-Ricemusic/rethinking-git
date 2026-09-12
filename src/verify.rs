use super::*;

fn records<T: DeserializeOwned>(repo: &Repo, directory: &str) -> Result<Vec<(String, T)>> {
    repo.transaction
        .list(&repo.path(&[directory]))?
        .into_iter()
        .map(|path| {
            let key = path
                .file_stem()
                .and_then(|v| v.to_str())
                .context("invalid record filename")?
                .to_string();
            Ok((key, read_json(repo, &path)?))
        })
        .collect()
}

fn identity(key: &str, id: &str, prefix: &str) -> Result<()> {
    validate_object_id(id, prefix)?;
    if key != id {
        bail!("record filename and identifier differ");
    }
    Ok(())
}

fn require(present: bool, description: &str) -> Result<()> {
    if !present {
        bail!("missing or inconsistent {description}");
    }
    Ok(())
}

fn snapshot_reference(id: Option<&str>, snapshots: &BTreeMap<String, Snapshot>) -> Result<()> {
    if let Some(id) = id {
        require(snapshots.contains_key(id), "snapshot reference")?;
    }
    Ok(())
}

pub(super) fn verify(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    if !actor.domains.iter().any(|domain| domain == ADMIN_DOMAIN) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    let config: RepoConfig = read_json(repo, &repo.path(&["repo.json"]))?;
    validate_object_id(&config.repo_id, "repo_")?;
    let mut changes = BTreeMap::new();
    for (key, change) in records::<Change>(repo, "changes")? {
        identity(&key, &change.id, "chg_")?;
        changes.insert(key, change);
    }
    let mut lines = BTreeMap::new();
    for (key, line) in records::<Line>(repo, "lines")? {
        validate_named_key(&line.name)?;
        require(key == file_name(&line.name), "line identity")?;
        lines.insert(line.name.clone(), line);
    }
    for (key, actor) in records::<Actor>(repo, "actors")? {
        validate_named_key(&actor.name)?;
        require(key == file_name(&actor.name), "actor identity")?;
    }
    let mut snapshots = BTreeMap::new();
    let mut referenced_blobs = BTreeSet::new();
    for (key, snapshot) in records::<Snapshot>(repo, "snapshots")? {
        identity(&key, &snapshot.id, "snap_")?;
        require(
            changes.contains_key(&snapshot.change_id),
            "snapshot owning change",
        )?;
        require(
            manifest_hash(&snapshot.files)? == snapshot.manifest_hash,
            "manifest hash",
        )?;
        let mut paths = BTreeSet::new();
        for file in &snapshot.files {
            transaction::validate_working_key(&file.path)?;
            require(paths.insert(file.path.clone()), "unique snapshot path")?;
            require(
                file.hash.len() == 64
                    && file
                        .hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "blob identifier",
            )?;
            let bytes = read_blob(repo, &file.hash)?;
            require(
                bytes.len() as u64 == file.bytes && hash_bytes(&bytes) == file.hash,
                "blob digest or length",
            )?;
            referenced_blobs.insert(file.hash.clone());
        }
        for path in &paths {
            let mut ancestor = Path::new(path).parent();
            while let Some(parent) = ancestor {
                require(
                    !paths.contains(parent.to_str().context("invalid snapshot path")?),
                    "noncolliding snapshot paths",
                )?;
                ancestor = parent.parent();
            }
        }
        snapshots.insert(key, snapshot);
    }
    for change in changes.values() {
        require(
            lines.contains_key(&change.target_line),
            "change target line",
        )?;
        snapshot_reference(change.base_snapshot.as_deref(), &snapshots)?;
        snapshot_reference(change.current_snapshot.as_deref(), &snapshots)?;
        if let Some(id) = &change.current_snapshot {
            require(
                snapshots[id].change_id == change.id,
                "current snapshot ownership",
            )?;
        }
    }
    for line in lines.values() {
        snapshot_reference(line.head_snapshot.as_deref(), &snapshots)?;
    }
    let workspace = read_workspace(repo)?;
    if let Some(id) = workspace.current_change {
        require(changes.contains_key(&id), "workspace change")?;
    }
    ancestry::validate_graph(&snapshots)?;
    for snapshot in snapshots.values() {
        if let Some(metadata) = &snapshot.git {
            let parsed = metadata.parse()?;
            require(
                snapshot.message == String::from_utf8_lossy(&parsed.message),
                "Git provenance message",
            )?;
            require(
                parsed.timestamp.checked_mul(1000) == Some(snapshot.created_at),
                "Git provenance timestamp",
            )?;
            let (tree, _) = git_objects::tree(repo, &snapshot.files, &metadata.object_format)?;
            require(tree == parsed.tree, "Git provenance tree")?;
            let parents = ancestry::parents(snapshot)
                .map(|id| {
                    snapshots[id]
                        .git
                        .as_ref()
                        .map(|m| m.object_id.clone())
                        .context("Git parent provenance missing")
                })
                .collect::<Result<Vec<_>>>()?;
            require(parents == parsed.parents, "Git provenance parents")?;
        }
    }
    let conflicts: BTreeMap<String, Conflict> = records::<Conflict>(repo, "conflicts")?
        .into_iter()
        .collect();
    for (key, conflict) in &conflicts {
        identity(key, &conflict.id, "conf_")?;
        require(
            changes.contains_key(&conflict.change_id) && lines.contains_key(&conflict.line),
            "conflict source",
        )?;
        snapshot_reference(conflict.base_snapshot.as_deref(), &snapshots)?;
        snapshot_reference(conflict.line_snapshot.as_deref(), &snapshots)?;
        snapshot_reference(Some(&conflict.incoming_snapshot), &snapshots)?;
        require(
            (conflict.status == ConflictStatus::Resolved) == conflict.resolution.is_some(),
            "conflict resolution state",
        )?;
    }
    let operations = records::<Operation>(repo, "operations")?;
    for (key, operation) in &operations {
        identity(key, &operation.id, "op_")?;
        match &operation.kind {
            OperationKind::InitRepo | OperationKind::SetPathPolicy { .. } => {}
            OperationKind::SetActor { actor } => {
                read_actor(repo, actor)?;
            }
            OperationKind::CreateChange { change_id }
            | OperationKind::SwitchWorkspace { change_id } => {
                require(changes.contains_key(change_id), "operation change")?
            }
            OperationKind::CreateSnapshot {
                change_id,
                snapshot_id,
            }
            | OperationKind::ImportGit {
                change_id,
                snapshot_id,
                ..
            }
            | OperationKind::IntegrateLine {
                change_id,
                snapshot_id,
                ..
            } => {
                require(changes.contains_key(change_id), "operation change")?;
                snapshot_reference(Some(snapshot_id), &snapshots)?;
                require(
                    snapshots[snapshot_id].change_id == *change_id,
                    "operation snapshot owner",
                )?;
                if let OperationKind::IntegrateLine { line, .. }
                | OperationKind::ImportGit { line, .. } = &operation.kind
                {
                    require(lines.contains_key(line), "operation line")?;
                }
            }
            OperationKind::CreateConflict {
                conflict_id,
                change_id,
            } => {
                require(
                    conflicts.contains_key(conflict_id) && changes.contains_key(change_id),
                    "operation conflict",
                )?;
            }
            OperationKind::ResolveConflict { conflict_id } => require(
                conflicts.contains_key(conflict_id),
                "resolution operation conflict",
            )?,
            OperationKind::RestoreWorkspace { snapshot_id } => {
                snapshot_reference(snapshot_id.as_deref(), &snapshots)?
            }
        }
    }
    let mut blob_count = 0;
    for entry in fs::read_dir(repo.path(&["blobs"]))? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("invalid blob filename"))?;
        if name.starts_with(".rgit-publish-") {
            continue;
        }
        require(
            name.len() == 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "blob filename",
        )?;
        require(hash_bytes(&read_blob(repo, &name)?) == name, "blob digest")?;
        blob_count += 1;
    }
    read_path_policies(repo)?;
    println!("repository verified: {} changes, {} lines, {} snapshots, {} conflicts, {} operations, {} blobs ({} unreferenced)",
        changes.len(), lines.len(), snapshots.len(), conflicts.len(), operations.len(), blob_count, blob_count - referenced_blobs.len());
    Ok(())
}

fn read_blob(repo: &Repo, hash: &str) -> Result<Vec<u8>> {
    let directory = repo.path(&["blobs"]);
    let metadata = fs::symlink_metadata(&directory)?;
    require(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "regular blob directory",
    )?;
    let path = directory.join(hash);
    let metadata = fs::symlink_metadata(&path)?;
    require(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "regular blob file",
    )?;
    fs::read(path).context("failed to read blob")
}
