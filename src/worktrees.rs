//! Linked directories share one command lock, recovery journal, and history store.
use super::*;

#[derive(Subcommand)]
pub(super) enum WorktreeCommand {
    /// Create a linked directory and a fresh change; resume interrupted creation explicitly.
    Add {
        path: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long, default_value = DEFAULT_LINE)]
        from: String,
        #[arg(long)]
        resume: bool,
        #[arg(long = "as", default_value = ADMIN_DOMAIN)]
        actor: String,
    },
    /// List the primary and linked directories (admin view).
    List,
    /// Detach a linked directory, preserving every working file and its saved history.
    Detach { path: PathBuf },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    id: String,
    root: PathBuf,
    name: String,
    line: String,
    snapshot: Option<String>,
    #[serde(default)]
    detached: bool,
}
#[derive(Serialize, Deserialize)]
struct Marker {
    common: PathBuf,
    repo_id: String,
    entry: Entry,
}

fn safe_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("unsafe worktree control directory");
    }
    Ok(())
}
fn create_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    safe_directory(path)?;
    sync_directory(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn load<T: DeserializeOwned>(meta: &Path, path: &Path) -> Result<T> {
    safe_directory(meta)?;
    transaction::check_path(meta, path)?;
    serde_json::from_slice(&fs::read(path)?).context("invalid worktree control record")
}
fn registry(meta: &Path) -> Result<Vec<Entry>> {
    let path = meta.join("worktrees.json");
    transaction::check_path(meta, &path)?;
    if !path.try_exists()? {
        return Ok(Vec::new());
    }
    let entries: Vec<Entry> = load(meta, &path)?;
    let mut ids = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for entry in &entries {
        validate_object_id(&entry.id, "wt_")?;
        if !entry.root.is_absolute()
            || !ids.insert(&entry.id)
            || !roots.insert(&entry.root)
            || entry.root.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            bail!("invalid or duplicate worktree registration");
        }
    }
    Ok(entries)
}
fn marker(root: &Path) -> Result<Marker> {
    let meta = root.join(META_DIR);
    load(&meta, &meta.join("linked.json"))
}

/// Resolve only a registered directory whose marker agrees with this repository.
pub(super) fn resolve_root(meta: &Path, id: &str) -> Result<PathBuf> {
    validate_object_id(id, "wt_")?;
    let entry = registry(meta)?
        .into_iter()
        .find(|e| e.id == id)
        .context("worktree is not registered")?;
    if entry.detached {
        bail!("worktree is detached");
    }
    let root = fs::canonicalize(&entry.root).context("registered worktree is unavailable")?;
    if root != entry.root {
        bail!("registered worktree path changed");
    }
    let marker = marker(&root)?;
    let config: RepoConfig = load(meta, &meta.join("repo.json"))?;
    if marker.common != meta || marker.repo_id != config.repo_id || marker.entry != entry {
        bail!("worktree marker does not match its registration");
    }
    Ok(root)
}

pub(super) fn discover(root: &Path) -> Result<Repo> {
    let root = fs::canonicalize(root)?;
    let marker = marker(&root)?;
    if marker.entry.root != root || fs::canonicalize(&marker.common)? != marker.common {
        bail!("worktree was moved; registered paths must remain unchanged");
    }
    initialization::preflight(&marker.common)?;
    let transaction = transaction::CommandTransaction::open_for(&marker.common, &marker.entry.id)?;
    Ok(Repo {
        root,
        meta: marker.common,
        workspace_id: Some(marker.entry.id),
        transaction,
    })
}

pub(super) fn ensure_change_available(repo: &Repo, change: &str) -> Result<()> {
    let primary: Workspace = read_json(repo, &repo.meta.join("workspace.json"))?;
    if repo.workspace_id.is_some() && primary.current_change.as_deref() == Some(change) {
        bail!("change is already active in the primary worktree");
    }
    for entry in registry(&repo.meta)? {
        if entry.detached || repo.workspace_id.as_deref() == Some(&entry.id) {
            continue;
        }
        let workspace: Workspace = read_json(
            repo,
            &repo
                .meta
                .join("worktrees")
                .join(&entry.id)
                .join("workspace.json"),
        )?;
        if workspace.current_change.as_deref() == Some(change) {
            bail!("change is already active in another worktree");
        }
    }
    Ok(())
}

fn admin(repo: &Repo) -> Result<()> {
    let actor = read_actor(repo, ADMIN_DOMAIN)?;
    if !can_access(&actor, &admin_policy()) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    Ok(())
}
fn report(repo: &Repo, entry: Option<&Entry>) -> Result<()> {
    let (id, root, detached) = match entry {
        Some(entry) => (Some(entry.id.as_str()), entry.root.clone(), entry.detached),
        None => (
            None,
            repo.meta
                .parent()
                .context("missing primary root")?
                .to_path_buf(),
            false,
        ),
    };
    let path = match id {
        Some(id) => repo.meta.join("worktrees").join(id).join("workspace.json"),
        None => repo.meta.join("workspace.json"),
    };
    let workspace: Workspace = read_json(repo, &path)?;
    let available = !detached
        && match id {
            Some(id) => resolve_root(&repo.meta, id).is_ok(),
            None => root.is_dir(),
        };
    output::record(
        "worktree",
        serde_json::json!({"id":id,"path":root.to_str(),"path_display":root.display().to_string(),"change_id":workspace.current_change,"available":available,"detached":detached}),
    );
    println!(
        "{} {} change:{}{}",
        id.unwrap_or("primary"),
        root.display(),
        workspace.current_change.as_deref().unwrap_or("none"),
        if detached {
            " (detached)"
        } else if available {
            ""
        } else {
            " (unavailable)"
        }
    );
    Ok(())
}

pub(super) fn run(command: &WorktreeCommand) -> Result<()> {
    match command {
        WorktreeCommand::Add {
            path,
            name,
            from,
            resume,
            actor,
        } => add(path, name, from, *resume, actor),
        WorktreeCommand::List => {
            let repo = Repo::discover()?;
            admin(&repo)?;
            report(&repo, None)?;
            for entry in registry(&repo.meta)? {
                report(&repo, Some(&entry))?;
            }
            Ok(())
        }
        WorktreeCommand::Detach { path } => detach(path),
    }
}

fn add(path: &Path, name: &str, from: &str, resume: bool, actor_name: &str) -> Result<()> {
    let repo = Repo::discover()?;
    admin(&repo)?;
    let actor = read_actor(&repo, actor_name)?;
    let line = read_line(&repo, from)?;
    if !can_access(&actor, &line.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    repo.meta
        .to_str()
        .context("linked worktrees require UTF-8 repository paths")?;
    let config: RepoConfig = read_json(&repo, &repo.meta.join("repo.json"))?;
    let mut entries = registry(&repo.meta)?;
    let request = if resume {
        let root = fs::canonicalize(path)?;
        let marker = marker(&root)?;
        if marker.common != repo.meta
            || marker.repo_id != config.repo_id
            || marker.entry.root != root
            || marker.entry.name != name
            || marker.entry.line != from
            || marker.entry.detached
        {
            bail!("resume arguments do not match the recorded worktree request");
        }
        marker.entry
    } else {
        let parent = fs::canonicalize(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let root = parent.join(
            path.file_name()
                .context("worktree path needs a directory name")?,
        );
        root.to_str()
            .context("linked worktrees require UTF-8 destination paths")?;
        let primary = repo.meta.parent().context("missing primary root")?;
        for existing in std::iter::once(primary).chain(
            entries
                .iter()
                .filter(|e| !e.detached)
                .map(|e| e.root.as_path()),
        ) {
            if root.starts_with(existing) || existing.starts_with(&root) {
                bail!("worktree directories must not overlap");
            }
        }
        if entries.iter().any(|entry| entry.root == root) {
            bail!("path has an existing worktree registration; choose a new path");
        }
        if fs::symlink_metadata(&root).is_ok() {
            bail!("worktree destination already exists; use --resume only for an interrupted creation");
        }
        let request = Entry {
            id: format!("wt_{}", new_id_suffix()),
            root,
            name: name.to_owned(),
            line: from.to_owned(),
            snapshot: line.head_snapshot,
            detached: false,
        };
        fs::create_dir(&request.root)?;
        fs::create_dir(request.root.join(META_DIR))?;
        transaction::publish_file(
            &request.root.join(META_DIR).join("linked.json"),
            &serde_json::to_vec_pretty(&Marker {
                common: repo.meta.clone(),
                repo_id: config.repo_id,
                entry: request.clone(),
            })?,
        )?;
        sync_directory(&request.root)?;
        sync_directory(request.root.parent().context("worktree has no parent")?)?;
        request
    };
    validate_object_id(&request.id, "wt_")?;
    let primary = repo.meta.parent().context("missing primary root")?;
    for existing in std::iter::once(primary).chain(
        entries
            .iter()
            .filter(|entry| !entry.detached && entry.id != request.id)
            .map(|entry| entry.root.as_path()),
    ) {
        if request.root.starts_with(existing) || existing.starts_with(&request.root) {
            bail!("worktree directories must not overlap");
        }
    }
    if let Some(existing) = entries.iter().find(|e| e.id == request.id) {
        if existing.detached || existing != &request {
            bail!("worktree registration is detached or inconsistent");
        }
    } else {
        // Publishing registration precedes checkout; resume reuses this exact intent.
        let directory = repo.meta.join("worktrees").join(&request.id);
        create_directory(&repo.meta.join("worktrees"))?;
        create_directory(&directory)?;
        transaction::check_path(&repo.meta, &directory.join("workspace.json"))?;
        if directory.join("workspace.json").try_exists()? {
            bail!("unregistered workspace metadata exists; refusing to replace it");
        }
        write_json(
            &repo,
            &directory.join("workspace.json"),
            &Workspace {
                current_change: None,
                mode_snapshot: None,
            },
        )?;
        entries.push(request.clone());
        write_json(&repo, &repo.meta.join("worktrees.json"), &entries)?;
        repo.transaction.commit()?;
    }
    drop(repo);
    let linked = discover(&request.root)?;
    if read_workspace(&linked)?.current_change.is_none() {
        checkout::start_at(
            &linked,
            &request.name,
            &request.line,
            actor_name,
            Some(request.snapshot.clone()),
        )?;
        linked.transaction.commit()?;
    }
    report(&linked, Some(&request))
}

fn detach(path: &Path) -> Result<()> {
    let repo = Repo::discover()?;
    admin(&repo)?;
    let root = fs::canonicalize(path)?;
    let mut entries = registry(&repo.meta)?;
    let entry = entries
        .iter_mut()
        .find(|e| e.root == root)
        .context("not a registered linked worktree")?;
    let marker_path = root.join(META_DIR).join("linked.json");
    if !entry.detached {
        resolve_root(&repo.meta, &entry.id)?;
    }
    if entry.detached && !marker_path.try_exists()? {
        return report(&repo, Some(entry));
    }
    let marker = marker(&root)?;
    if marker.common != repo.meta || marker.entry.id != entry.id {
        bail!("worktree marker changed");
    }
    entry.detached = true;
    let detached = entry.clone();
    write_json(&repo, &repo.meta.join("worktrees.json"), &entries)?;
    repo.transaction.commit()?;
    fs::remove_file(marker_path)?;
    // Only the empty control directory is removed; user files are never recursively deleted.
    fs::remove_dir(root.join(META_DIR))
        .context("detached; control directory contains extra files and was retained")?;
    sync_directory(&root)?;
    report(&repo, Some(&detached))
}

/// Validate every saved workspace, including unavailable and detached directories.
pub(super) fn saved_workspaces(repo: &Repo) -> Result<Vec<(Workspace, bool)>> {
    let mut result = vec![(read_json(repo, &repo.meta.join("workspace.json"))?, true)];
    for entry in registry(&repo.meta)? {
        result.push((
            read_json(
                repo,
                &repo
                    .meta
                    .join("worktrees")
                    .join(entry.id)
                    .join("workspace.json"),
            )?,
            !entry.detached,
        ));
    }
    Ok(result)
}
