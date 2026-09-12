use super::*;

/// The caller's command lock excludes concurrent metadata publication throughout.
pub(super) fn backup(repo: &Repo, destination: &Path, actor_name: &str) -> Result<()> {
    verify::check(repo, actor_name)?;
    let absolute = std::env::current_dir()?.join(destination);
    let parent = fs::canonicalize(
        absolute
            .parent()
            .context("backup destination has no parent")?,
    )?;
    let name = absolute
        .file_name()
        .context("backup destination needs a new directory name")?;
    let destination = parent.join(name);
    if destination.starts_with(&repo.root) {
        bail!("backup destination must be outside the source working tree");
    }
    fs::create_dir(&destination).context("backup destination must not already exist")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o700))?;
    }
    let staging = destination.join(format!(".rgit-backup-{}", Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        fs::create_dir(&staging)?;
        for directory in [
            "actors",
            "blobs",
            "changes",
            "conflicts",
            "lines",
            "operations",
            "snapshots",
        ] {
            let source = repo.path(&[directory]);
            let metadata = fs::symlink_metadata(&source)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                bail!("backup source directory is unsafe");
            }
            let target = staging.join(directory);
            fs::create_dir(&target)?;
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".rgit-publish-")
                {
                    continue;
                }
                copy_record(&entry.path(), &target.join(entry.file_name()))?;
            }
            sync_directory(&target)?;
        }
        for name in ["repo.json", "workspace.json", "path-policies.json"] {
            copy_record(&repo.path(&[name]), &staging.join(name))?;
        }
        {
            let transaction = transaction::CommandTransaction::open(&staging)?;
            let copy = Repo {
                root: destination.clone(),
                meta: staging.clone(),
                transaction,
            };
            verify::check(&copy, actor_name).context("backup verification failed")?;
        }
        sync_directory(&staging)?;
        fs::rename(&staging, destination.join(META_DIR))?;
        sync_directory(&destination)?;
        sync_directory(&parent)?;
        Ok(())
    })();
    if result.is_err() {
        // Only remove our unpublished staging tree. Never delete a published backup.
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir(&destination);
    }
    result?;
    println!("saved-history backup created at {}", destination.display());
    println!("to materialize its current snapshot: run workspace restore --discard-changes --as {actor_name} inside the backup");
    Ok(())
}

fn copy_record(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("backup source must be a regular file");
    }
    // No mutable SQLite/WAL files are copied. The new copy gets its own empty journal.
    transaction::publish_file(target, &fs::read(source)?)?;
    let file = fs::OpenOptions::new().read(true).write(true).open(target)?;
    file.set_permissions(metadata.permissions())?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
