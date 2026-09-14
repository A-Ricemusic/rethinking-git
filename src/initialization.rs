use super::*;

const DIRECTORIES: &[&str] = &[
    "actors",
    "blobs",
    "changes",
    "conflicts",
    "lines",
    "operations",
    "snapshots",
];
const REQUEST: &str = "initialization.json";

fn directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("repository control path must be a directory, not a symlink or file");
    }
    Ok(())
}

fn existing_config(meta: &Path, name: &str) -> Result<Option<RepoConfig>> {
    let path = meta.join(name);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            bail!("unsafe repository configuration entry")
        }
        Ok(_) => {}
    }
    let config: RepoConfig = serde_json::from_slice(&fs::read(&path)?)
        .context("repository configuration is missing or invalid")?;
    if config.format_version != FORMAT_VERSION {
        bail!(
            "unsupported repository format {}; expected {}; migration is required",
            config.format_version,
            FORMAT_VERSION
        );
    }
    validate_object_id(&config.repo_id, "repo_")?;
    Ok(Some(config))
}

/// Refuse incompatible control/configuration entries before opening writable SQLite.
pub(super) fn preflight(meta: &Path) -> Result<()> {
    directory(meta)?;
    if existing_config(meta, "repo.json")?.is_none() {
        existing_config(meta, REQUEST)?;
        if !meta.join("command-journal.sqlite3").try_exists()? {
            bail!("repository configuration is missing; use init --resume for interrupted initialization");
        }
    }
    Ok(())
}

fn initialization_layout(meta: &Path) -> Result<()> {
    directory(meta)?;
    for entry in fs::read_dir(meta)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().context("unrecognized initialization entry")?;
        if DIRECTORIES.contains(&name) {
            directory(&entry.path())?;
        } else if [
            REQUEST,
            "workspace.json",
            "path-policies.json",
            "command-lock.sqlite3",
            "command-lock.sqlite3-journal",
            "command-journal.sqlite3",
            "command-journal.sqlite3-journal",
        ]
        .contains(&name)
            || name
                .strip_prefix(".rgit-publish-")
                .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                bail!("unsafe initialization entry");
            }
        } else {
            bail!("unrecognized control data; initialization will not replace it");
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Directories,
    Request,
    BeforeCommit,
    AfterCommit,
}

pub(super) fn initialize(root: PathBuf, resume: bool) -> Result<Repo> {
    initialize_with(root, resume, |_| Ok(()))
}

fn initialize_with(
    root: PathBuf,
    resume: bool,
    mut phase: impl FnMut(Phase) -> Result<()>,
) -> Result<Repo> {
    let root = fs::canonicalize(root).context("initialization root must exist")?;
    let meta = root.join(META_DIR);
    if fs::symlink_metadata(&meta).is_ok() {
        directory(&meta)?;
        let existing = existing_config(&meta, "repo.json")?;
        existing_config(&meta, REQUEST)?;
        if !resume {
            bail!("repository already exists at {}; use init --resume only to complete initialization", meta.display());
        }
        if existing.is_some() {
            let repo = Repo::discover_from(root)?;
            finish_request(&repo)?;
            println!("repository already initialized; saved history is unchanged");
            return Ok(repo);
        }
        initialization_layout(&meta)?;
    } else {
        if resume {
            bail!("no initialization exists to resume");
        }
        fs::create_dir(&meta).context("failed to create .rgit directory")?;
        #[cfg(unix)]
        fs::File::open(&root)?.sync_all()?;
    }
    for name in DIRECTORIES {
        let path = meta.join(name);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => directory(&path)?,
            Err(e) => return Err(e.into()),
        }
    }
    #[cfg(unix)]
    fs::File::open(&meta)?.sync_all()?;
    phase(Phase::Directories)?;
    let transaction = transaction::CommandTransaction::open(&meta)?;
    let repo = Repo {
        workspace_id: None,
        root,
        meta,
        transaction,
    };
    if existing_config(&repo.meta, "repo.json")?.is_some() {
        finish_request(&repo)?;
        println!("recovered repository initialization");
        return Ok(repo);
    }
    // A missing config in a populated repository is corruption, not permission
    // to recreate defaults or replace its identity/history.
    for name in DIRECTORIES {
        if fs::read_dir(repo.path(&[name]))?.next().is_some() {
            bail!("saved control data exists without configuration; initialization will not replace it");
        }
    }
    for name in ["workspace.json", "path-policies.json"] {
        if repo.path(&[name]).try_exists()? {
            bail!("saved control data exists without configuration; initialization will not replace it");
        }
    }
    let config = if let Some(config) = existing_config(&repo.meta, REQUEST)? {
        config
    } else {
        let config = RepoConfig {
            git: Default::default(),
            author: None,
            format_version: FORMAT_VERSION,
            repo_id: format!("repo_{}", new_id_suffix()),
            created_at: now()?,
        };
        transaction::publish_file(&repo.path(&[REQUEST]), &serde_json::to_vec_pretty(&config)?)?;
        config
    };
    phase(Phase::Request)?;
    let workspace = Workspace {
        mode_snapshot: None,
        current_change: None,
    };
    let public_actor = Actor {
        name: PUBLIC_DOMAIN.to_string(),
        domains: vec![PUBLIC_DOMAIN.to_string()],
    };
    let admin_actor = Actor {
        name: ADMIN_DOMAIN.to_string(),
        domains: vec![PUBLIC_DOMAIN.to_string(), ADMIN_DOMAIN.to_string()],
    };
    let main_line = Line {
        name: DEFAULT_LINE.to_string(),
        head_snapshot: None,
        policy: public_policy(),
        created_at: now()?,
    };

    write_json(&repo, &repo.path(&["repo.json"]), &config)?;
    write_json(&repo, &repo.path(&["workspace.json"]), &workspace)?;
    write_json(
        &repo,
        &repo.path(&["path-policies.json"]),
        &Vec::<PathPolicy>::new(),
    )?;
    write_json(&repo, &actor_path(&repo, PUBLIC_DOMAIN)?, &public_actor)?;
    write_json(&repo, &actor_path(&repo, ADMIN_DOMAIN)?, &admin_actor)?;
    write_json(&repo, &line_path(&repo, DEFAULT_LINE)?, &main_line)?;
    record_operation(
        &repo,
        OperationKind::InitRepo,
        public_policy(),
        "initialized repository".to_string(),
        None,
    )?;

    phase(Phase::BeforeCommit)?;
    repo.transaction.commit()?;
    phase(Phase::AfterCommit)?;
    finish_request(&repo)?;
    println!("initialized rgit repository");
    println!("default line: {DEFAULT_LINE}");
    println!("default actors: public, admin");
    Ok(repo)
}

fn finish_request(repo: &Repo) -> Result<()> {
    verify::check(repo, ADMIN_DOMAIN)?;
    if let Some(request) = existing_config(&repo.meta, REQUEST)? {
        let config: RepoConfig = read_json(repo, &repo.path(&["repo.json"]))?;
        if request.repo_id != config.repo_id {
            bail!("initialization identity differs from saved repository");
        }
        fs::remove_file(repo.path(&[REQUEST]))?;
        #[cfg(unix)]
        fs::File::open(&repo.meta)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    #[ignore = "initialization interruption subprocess helper"]
    fn interruption_child() {
        let root = PathBuf::from(std::env::var_os("RGIT_INIT_CRASH_ROOT").unwrap());
        let selected = std::env::var("RGIT_INIT_CRASH_PHASE").unwrap();
        initialize_with(root, false, |phase| {
            if format!("{phase:?}") == selected {
                std::process::exit(77);
            }
            Ok(())
        })
        .unwrap();
        panic!("interruption phase was not reached");
    }

    #[test]
    fn interrupted_initialization_resumes_without_changing_identity_or_working_files() {
        for selected in ["Directories", "Request", "BeforeCommit", "AfterCommit"] {
            let root = Root(std::env::temp_dir().join(format!("rgit-init-{}", Uuid::new_v4())));
            fs::create_dir(&root.0).unwrap();
            fs::write(root.0.join("existing.txt"), b"user work").unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "initialization::tests::interruption_child",
                    "--ignored",
                ])
                .env("RGIT_INIT_CRASH_ROOT", &root.0)
                .env("RGIT_INIT_CRASH_PHASE", selected)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(77));
            let request = existing_config(&root.0.join(META_DIR), REQUEST).unwrap();
            let repo = initialize(root.0.clone(), true).unwrap();
            verify::check(&repo, ADMIN_DOMAIN).unwrap();
            let config: RepoConfig = read_json(&repo, &repo.path(&["repo.json"])).unwrap();
            if let Some(request) = request {
                assert_eq!(request.repo_id, config.repo_id);
            }
            assert!(!repo.path(&[REQUEST]).exists());
            assert_eq!(fs::read(root.0.join("existing.txt")).unwrap(), b"user work");
            assert_eq!(fs::read_dir(repo.path(&["operations"])).unwrap().count(), 1);
            drop(repo);
            let repo = initialize(root.0.clone(), true).unwrap();
            assert_eq!(fs::read_dir(repo.path(&["operations"])).unwrap().count(), 1);
        }
    }
}
