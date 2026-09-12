//! Recoverable command publication for the format-2 compatibility repository.
//! The lock connection remains in a write transaction across journal commits.
use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    time::Duration,
};

mod working_plan;

pub(crate) struct CommandTransaction {
    root: PathBuf,
    _lock: Connection,
    journal: Connection,
    pending: RefCell<BTreeMap<PathBuf, Vec<u8>>>,
    working: RefCell<BTreeMap<String, WorkingUpdate>>,
}

impl CommandTransaction {
    pub(crate) fn open(root: &Path) -> Result<Self> {
        ensure_directory(root)?;
        let lock = open_database(&root.join("command-lock.sqlite3"))?;
        lock.execute_batch("BEGIN IMMEDIATE")
            .context("repository is busy; another command holds its transaction lock")?;
        let journal = open_database(&root.join("command-journal.sqlite3"))?;
        journal.execute_batch("CREATE TABLE IF NOT EXISTS pending (path TEXT PRIMARY KEY NOT NULL, bytes BLOB NOT NULL)")?;
        journal.execute_batch("CREATE TABLE IF NOT EXISTS working (path TEXT PRIMARY KEY NOT NULL, before_bytes BLOB, after_bytes BLOB)")?;
        let columns: i64 = journal.query_row(
            "SELECT count(*) FROM pragma_table_info('working') WHERE name='before_executable'",
            [],
            |r| r.get(0),
        )?;
        if columns == 0 {
            journal.execute_batch("BEGIN IMMEDIATE; ALTER TABLE working ADD COLUMN before_executable INTEGER; ALTER TABLE working ADD COLUMN after_executable INTEGER; COMMIT;")?;
        }
        let columns: i64 = journal.query_row(
            "SELECT count(*) FROM pragma_table_info('working') WHERE name='before_symlink'",
            [],
            |r| r.get(0),
        )?;
        if columns == 0 {
            journal.execute_batch("BEGIN IMMEDIATE; ALTER TABLE working ADD COLUMN before_symlink INTEGER; ALTER TABLE working ADD COLUMN after_symlink INTEGER; COMMIT;")?;
        }
        let result = Self {
            root: root.to_path_buf(),
            _lock: lock,
            journal,
            pending: RefCell::new(BTreeMap::new()),
            working: RefCell::new(BTreeMap::new()),
        };
        result.recover()?;
        Ok(result)
    }

    fn key(&self, path: &Path) -> Result<String> {
        let relative = path
            .strip_prefix(&self.root)
            .context("metadata path escaped repository")?;
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
            || relative.as_os_str().is_empty()
        {
            bail!("invalid metadata path");
        }
        let key = relative.to_str().context("metadata path is not UTF-8")?;
        // Only format-2 JSON records may be replayed, never the journal or worktree.
        if relative.extension().and_then(|v| v.to_str()) != Some("json") {
            bail!("transaction target must be a JSON record");
        }
        Ok(key.to_string())
    }

    pub(crate) fn read(&self, path: &Path) -> Result<Vec<u8>> {
        self.key(path)?;
        if let Some(bytes) = self.pending.borrow().get(path) {
            return Ok(bytes.clone());
        }
        check_path(&self.root, path)?;
        fs::read(path).with_context(|| format!("failed to read {}", path.display()))
    }

    pub(crate) fn stage(&self, path: &Path, bytes: Vec<u8>) -> Result<()> {
        self.key(path)?;
        check_path(&self.root, path)?;
        self.pending.borrow_mut().insert(path.to_path_buf(), bytes);
        Ok(())
    }

    pub(crate) fn list(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        ensure_directory(dir)?;
        let mut paths = BTreeSet::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|v| v.to_str()) == Some("json") {
                paths.insert(path);
            }
        }
        for path in self.pending.borrow().keys() {
            if path.parent() == Some(dir) {
                paths.insert(path.clone());
            }
        }
        Ok(paths.into_iter().collect())
    }

    pub(crate) fn commit(&self) -> Result<()> {
        if self.pending.borrow().is_empty() && self.working.borrow().is_empty() {
            return Ok(());
        }
        let shape = working_plan::ShapePlan::from_updates(&self.working.borrow())?;
        shape.validate_aliases(&self.workspace_root()?)?;
        working_plan::validate_updates(&self.working.borrow())?;
        for (key, update) in self.working.borrow().iter() {
            if !shape.current(&self.workspace_root()?, key)?.matches(
                &update.before,
                update.before_executable,
                update.before_symlink,
            ) {
                bail!("working file changed during command: {key}");
            }
        }
        let transaction = self.journal.unchecked_transaction()?;
        for (key, update) in self.working.borrow().iter() {
            transaction.execute(
                "INSERT INTO working VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    key,
                    update.before,
                    update.after,
                    update.before_executable,
                    update.after_executable,
                    update.before_symlink,
                    update.after_symlink
                ],
            )?;
        }
        for (path, bytes) in self.pending.borrow().iter() {
            transaction.execute(
                "INSERT INTO pending(path, bytes) VALUES (?1, ?2)",
                rusqlite::params![self.key(path)?, bytes],
            )?;
        }
        transaction
            .commit()
            .context("failed to commit command journal")?;
        self.pending.borrow_mut().clear();
        self.working.borrow_mut().clear();
        self.recover().context(
            "command committed; publication incomplete; run another rgit command to recover",
        )
    }

    fn recover(&self) -> Result<()> {
        self.recover_with_hook(&mut || Ok(()))
    }

    fn recover_with_hook(&self, hook: &mut impl FnMut() -> Result<()>) -> Result<()> {
        let entries = {
            let mut query = self
                .journal
                .prepare("SELECT path, bytes FROM pending ORDER BY path")?;
            let rows = query.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let working = {
            let mut query = self
                .journal
                .prepare("SELECT path, before_bytes, after_bytes, before_executable, after_executable, before_symlink, after_symlink FROM working ORDER BY path")?;
            let rows = query.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    WorkingUpdate {
                        before: row.get(1)?,
                        after: row.get(2)?,
                        before_executable: row.get(3)?,
                        after_executable: row.get(4)?,
                        before_symlink: row.get(5)?,
                        after_symlink: row.get(6)?,
                    },
                ))
            })?;
            rows.collect::<std::result::Result<BTreeMap<_, _>, _>>()?
        };
        let shape = working_plan::ShapePlan::from_updates(&working)?;
        shape.validate_aliases(&self.workspace_root()?)?;
        working_plan::validate_updates(&working)?;
        for (key, update) in &working {
            let current = shape.current(&self.workspace_root()?, key)?;
            if !current.matches(
                &update.before,
                update.before_executable,
                update.before_symlink,
            ) && !current.matches(&update.after, update.after_executable, update.after_symlink)
            {
                bail!("recovery stopped: working file was edited after interrupted command: {key}");
            }
        }
        // Validate the entire recovery set before publishing any member.
        for (key, _) in &entries {
            let path = self.root.join(key);
            if self.key(&path)? != *key {
                bail!("invalid journal path");
            }
            check_path(&self.root, &path)?;
        }
        working_plan::publish(&self.workspace_root()?, &working, &shape, hook)?;
        for (key, bytes) in &entries {
            publish_file(&self.root.join(key), bytes)?;
            hook()?;
        }
        if !entries.is_empty() || !working.is_empty() {
            let transaction = self.journal.unchecked_transaction()?;
            transaction.execute_batch("DELETE FROM pending; DELETE FROM working;")?;
            transaction.commit()?;
        }
        Ok(())
    }
}

fn open_database(path: &Path) -> Result<Connection> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("transaction database must be a regular file");
        }
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(Duration::from_secs(10))?;
    let application: i64 = connection.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    match (application, version) {
        (0, 0) => {
            let tables: i64 =
                connection.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
            if tables != 0 {
                bail!("unrecognized command transaction database");
            }
            connection.execute_batch("PRAGMA application_id=1380402004; PRAGMA user_version=5;")?;
        }
        (1380402004, 1..=4) => {
            connection.execute_batch("PRAGMA user_version=5;")?;
        }
        (1380402004, 5) => {}
        _ => bail!("unsupported command transaction database format"),
    }
    connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
    Ok(connection)
}

fn ensure_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "repository directory must not be a symlink: {}",
            path.display()
        );
    }
    Ok(())
}

fn check_path(root: &Path, path: &Path) -> Result<()> {
    let relative = path.strip_prefix(root)?;
    let mut current = root.to_path_buf();
    let count = relative.components().count();
    for (index, part) in relative.components().enumerate() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || (index + 1 < count && !metadata.is_dir())
                    || (index + 1 == count && !metadata.is_file())
                {
                    bail!("unsafe repository metadata path: {}", current.display());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && index + 1 == count => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Publish complete bytes before references can point to them.
pub(crate) fn publish_file(path: &Path, bytes: &[u8]) -> Result<()> {
    publish_file_with_mode(path, bytes, None, false)
}

fn publish_file_with_mode(
    path: &Path,
    bytes: &[u8],
    executable: Option<bool>,
    working: bool,
) -> Result<()> {
    let parent = path.parent().context("file has no parent")?;
    ensure_directory(parent)?;
    let permissions = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Some(metadata.permissions())
        }
        Ok(metadata) if working && metadata.file_type().is_symlink() => None,
        Ok(_) => bail!("publication target is not a regular file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let temporary = parent.join(format!(".rgit-publish-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        #[cfg(unix)]
        if let Some(executable) = executable {
            use std::os::unix::fs::PermissionsExt;
            let mode = file.metadata()?.permissions().mode();
            let mode = if executable {
                mode | ((mode & 0o444) >> 2)
            } else {
                mode & !0o111
            };
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = executable;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

struct WorkingUpdate {
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    before_executable: Option<bool>,
    after_executable: Option<bool>,
    before_symlink: Option<bool>,
    after_symlink: Option<bool>,
}

impl CommandTransaction {
    fn workspace_root(&self) -> Result<PathBuf> {
        Ok(self
            .root
            .parent()
            .context("control directory has no parent")?
            .to_path_buf())
    }

    pub(crate) fn stage_checkout(
        &self,
        before: &BTreeMap<String, (Vec<u8>, WorkingFlags)>,
        after: &BTreeMap<String, (Vec<u8>, WorkingFlags)>,
        current_modes: Option<&BTreeMap<String, WorkingFlags>>,
        discard: bool,
    ) -> Result<()> {
        let paths: BTreeSet<_> = before.keys().chain(after.keys()).collect();
        let shape = working_plan::ShapePlan::new(paths.iter().map(|path| {
            (
                (*path).clone(),
                before.contains_key(*path),
                after.contains_key(*path),
            )
        }))?;
        shape.validate_aliases(&self.workspace_root()?)?;
        for path in paths {
            let current = shape.current(&self.workspace_root()?, path)?;
            let previous = before.get(path).map(|v| &v.0);
            let target = after.get(path).map(|v| &v.0);
            let before_flags = before.get(path).map(|v| v.1).unwrap_or_default();
            let after_flags = after.get(path).map(|v| v.1).unwrap_or_default();
            let flags = if cfg!(not(unix)) && current.bytes.is_some() {
                let mut flags = current_modes.map_or(before_flags, |modes| {
                    modes.get(path).copied().unwrap_or_default()
                });
                flags.symlink |= current.flags.symlink;
                flags
            } else {
                current.flags
            };
            if previous.is_none() && current.bytes.is_some() {
                bail!("checkout would overwrite an untracked file: {path}");
            }
            if !discard && (current.bytes.as_ref() != previous || flags != before_flags) {
                bail!("tracked file has local changes: {path}; snapshot first or explicitly restore with --discard-changes");
            }
            if current.bytes.as_ref() != target || flags != after_flags {
                self.stage_working(path, current.bytes, target.cloned(), flags, after_flags)?;
            }
        }
        Ok(())
    }

    pub(crate) fn stage_working(
        &self,
        key: &str,
        before: Option<Vec<u8>>,
        after: Option<Vec<u8>>,
        before_flags: WorkingFlags,
        after_flags: WorkingFlags,
    ) -> Result<()> {
        validate_working_key(key)?;
        if after_flags.symlink {
            validate_link(after.as_deref().context("symlink update has no target")?)?;
        }
        self.working.borrow_mut().insert(
            key.to_string(),
            WorkingUpdate {
                before,
                after,
                before_executable: Some(before_flags.executable),
                after_executable: Some(after_flags.executable),
                before_symlink: Some(before_flags.symlink),
                after_symlink: Some(after_flags.symlink),
            },
        );
        Ok(())
    }
}

pub(crate) fn validate_working_key(key: &str) -> Result<()> {
    #[cfg(windows)]
    super::validate_named_key(key)?;
    let relative = Path::new(key);
    if key.is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || key.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || matches!(part.to_ascii_lowercase().as_str(), ".git" | ".rgit")
        })
    {
        bail!("unsafe snapshot path");
    }
    Ok(())
}

pub(crate) fn working_path(root: &Path, key: &str) -> Result<PathBuf> {
    validate_working_key(key)?;
    let relative = Path::new(key);
    let mut path = root.to_path_buf();
    for (index, part) in relative.components().enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && index + 1 < relative.components().count() =>
            {
                bail!("working path traverses a symlink: {key}")
            }
            Ok(metadata) if index + 1 < relative.components().count() && !metadata.is_dir() => {
                bail!("working path has a non-directory parent: {key}")
            }
            Ok(metadata)
                if index + 1 == relative.components().count()
                    && !metadata.is_file()
                    && !metadata.file_type().is_symlink() =>
            {
                bail!("working path is not a regular file: {key}")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

pub(crate) fn read_working(root: &Path, key: &str) -> Result<Option<Vec<u8>>> {
    let path = working_path(root, key)?;
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Ok(Some(read_link_bytes(&path)?));
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn is_executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::symlink_metadata(path)?;
        Ok(!metadata.file_type().is_symlink() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkingFlags {
    pub executable: bool,
    pub symlink: bool,
}

pub(crate) fn working_flags(path: &Path) -> Result<WorkingFlags> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(WorkingFlags {
            executable: is_executable(path)?,
            symlink: metadata.file_type().is_symlink(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WorkingFlags::default()),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn validate_link(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.contains(&0) {
        bail!("symlink target must be nonempty and contain no NUL");
    }
    Ok(())
}

pub(crate) fn read_link_bytes(path: &Path) -> Result<Vec<u8>> {
    let target = fs::read_link(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(target.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        Ok(target
            .to_str()
            .context("symlink target must be UTF-8 on this platform")?
            .as_bytes()
            .to_vec())
    }
}

fn publish_link(path: &Path, bytes: &[u8]) -> Result<()> {
    validate_link(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let parent = path.parent().context("symlink has no parent")?;
        ensure_directory(parent)?;
        let temporary = parent.join(format!(".rgit-publish-{}", uuid::Uuid::new_v4().simple()));
        symlink(std::ffi::OsStr::from_bytes(bytes), &temporary)?;
        let result = fs::rename(&temporary, path);
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        bail!("native symlink publication is unsupported on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repository(PathBuf);
    impl Repository {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("rgit-command-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for Repository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn schema_four_upgrades_and_future_schema_is_refused() {
        let repository = Repository::new();
        let database = repository.0.join("journal.sqlite3");
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch("PRAGMA application_id=1380402004; PRAGMA user_version=4;")
                .unwrap();
        }
        {
            let connection = open_database(&database).unwrap();
            let version: i64 = connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, 5);
            connection.execute_batch("PRAGMA user_version=6;").unwrap();
        }
        assert!(open_database(&database).is_err());
    }

    #[test]
    fn aliased_recovery_rows_are_refused_before_any_publication() {
        let repository = Repository::new();
        let meta = repository.0.join(".rgit");
        fs::create_dir(&meta).unwrap();
        {
            let command = CommandTransaction::open(&meta).unwrap();
            command
                .journal
                .execute(
                    "INSERT INTO working(path,after_bytes) VALUES ('README',?1), ('readme',?2)",
                    rusqlite::params![b"first", b"second"],
                )
                .unwrap();
            command
                .journal
                .execute(
                    "INSERT INTO pending VALUES ('record.json',?1)",
                    [b"saved".as_slice()],
                )
                .unwrap();
        }
        assert!(CommandTransaction::open(&meta).is_err());
        assert_eq!(fs::read_dir(&repository.0).unwrap().count(), 1);
        assert!(!meta.join("record.json").exists());
        let db = Connection::open(meta.join("command-journal.sqlite3")).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM working", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    fn shape_fixture(root: &Path, to_directory: bool) -> BTreeMap<String, WorkingUpdate> {
        let mut updates = BTreeMap::new();
        for (path, before, after) in [
            ("shape", Some(b"file".to_vec()), None),
            ("shape/nested/one", None, Some(b"one".to_vec())),
            ("shape/two", None, Some(b"two".to_vec())),
        ] {
            let (before, after) = if to_directory {
                (before, after)
            } else {
                (after, before)
            };
            if let Some(bytes) = &before {
                fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
                fs::write(root.join(path), bytes).unwrap();
            }
            updates.insert(
                path.to_string(),
                WorkingUpdate {
                    before,
                    after,
                    before_executable: Some(false),
                    after_executable: Some(false),
                    before_symlink: Some(false),
                    after_symlink: Some(false),
                },
            );
        }
        updates
    }

    #[test]
    #[ignore = "shape transition interruption subprocess helper"]
    fn shape_interruption_child() {
        let root = PathBuf::from(std::env::var_os("RGIT_SHAPE_ROOT").unwrap());
        let selected: usize = std::env::var("RGIT_SHAPE_PHASE").unwrap().parse().unwrap();
        let to_directory = std::env::var("RGIT_SHAPE_DIRECTION").unwrap() == "directory";
        let updates = shape_fixture(&root, to_directory);
        let command = CommandTransaction::open(&root.join(".rgit")).unwrap();
        let tx = command.journal.unchecked_transaction().unwrap();
        for (path, update) in updates {
            tx.execute(
                "INSERT INTO working VALUES (?1,?2,?3,0,0,0,0)",
                rusqlite::params![path, update.before, update.after],
            )
            .unwrap();
        }
        tx.execute(
            "INSERT INTO pending VALUES ('workspace.json',?1)",
            [b"new pointer".as_slice()],
        )
        .unwrap();
        tx.commit().unwrap();
        let mut count = 0;
        command
            .recover_with_hook(&mut || {
                count += 1;
                if count == selected {
                    std::process::exit(77);
                }
                Ok(())
            })
            .unwrap();
        panic!("requested publication phase not reached");
    }

    #[test]
    fn shape_transitions_recover_after_every_publication_boundary() {
        for direction in ["directory", "file"] {
            // delete/mkdir/rmdir/file publication plus metadata publication.
            for phase in 1..=6 {
                let repository = Repository::new();
                let meta = repository.0.join(".rgit");
                fs::create_dir(&meta).unwrap();
                fs::write(meta.join("workspace.json"), b"old pointer").unwrap();
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "transaction::tests::shape_interruption_child",
                        "--ignored",
                    ])
                    .env("RGIT_SHAPE_ROOT", &repository.0)
                    .env("RGIT_SHAPE_PHASE", phase.to_string())
                    .env("RGIT_SHAPE_DIRECTION", direction)
                    .status()
                    .unwrap();
                assert_eq!(status.code(), Some(77), "{direction} phase {phase}");
                let command = CommandTransaction::open(&meta).unwrap();
                assert_eq!(
                    command.read(&meta.join("workspace.json")).unwrap(),
                    b"new pointer"
                );
                if direction == "directory" {
                    assert_eq!(
                        fs::read(repository.0.join("shape/nested/one")).unwrap(),
                        b"one"
                    );
                    assert_eq!(fs::read(repository.0.join("shape/two")).unwrap(), b"two");
                } else {
                    assert_eq!(fs::read(repository.0.join("shape")).unwrap(), b"file");
                }
                let pending: i64 = command
                    .journal
                    .query_row("SELECT count(*) FROM working", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(pending, 0);
            }
        }
    }

    #[test]
    fn directory_recovery_preserves_untracked_files_created_after_interruption() {
        let repository = Repository::new();
        let meta = repository.0.join(".rgit");
        fs::create_dir(&meta).unwrap();
        fs::write(meta.join("workspace.json"), b"old pointer").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "transaction::tests::shape_interruption_child",
                "--ignored",
            ])
            .env("RGIT_SHAPE_ROOT", &repository.0)
            .env("RGIT_SHAPE_PHASE", "1")
            .env("RGIT_SHAPE_DIRECTION", "file")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(77));
        fs::write(repository.0.join("shape/untracked"), b"later work").unwrap();
        assert!(CommandTransaction::open(&meta).is_err());
        assert_eq!(
            fs::read(repository.0.join("shape/untracked")).unwrap(),
            b"later work"
        );
        assert_eq!(
            fs::read(meta.join("workspace.json")).unwrap(),
            b"old pointer"
        );
        fs::remove_file(repository.0.join("shape/untracked")).unwrap();
        let command = CommandTransaction::open(&meta).unwrap();
        assert_eq!(fs::read(repository.0.join("shape")).unwrap(), b"file");
        assert_eq!(
            command.read(&meta.join("workspace.json")).unwrap(),
            b"new pointer"
        );
    }

    #[test]
    fn file_to_directory_recovery_preserves_later_untracked_work_and_refuses_collisions() {
        for collision in [false, true] {
            let repository = Repository::new();
            let meta = repository.0.join(".rgit");
            fs::create_dir(&meta).unwrap();
            fs::write(meta.join("workspace.json"), b"old pointer").unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "transaction::tests::shape_interruption_child",
                    "--ignored",
                ])
                .env("RGIT_SHAPE_ROOT", &repository.0)
                .env("RGIT_SHAPE_PHASE", "1")
                .env("RGIT_SHAPE_DIRECTION", "directory")
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(77));
            fs::create_dir_all(repository.0.join("shape/nested")).unwrap();
            let path = repository.0.join(if collision {
                "shape/nested/one"
            } else {
                "shape/untracked"
            });
            fs::write(&path, b"later work").unwrap();
            let recovered = CommandTransaction::open(&meta);
            if collision {
                assert!(recovered.is_err());
                assert_eq!(
                    fs::read(meta.join("workspace.json")).unwrap(),
                    b"old pointer"
                );
            } else {
                let command = recovered.unwrap();
                assert_eq!(
                    command.read(&meta.join("workspace.json")).unwrap(),
                    b"new pointer"
                );
                assert_eq!(
                    fs::read(repository.0.join("shape/nested/one")).unwrap(),
                    b"one"
                );
            }
            assert_eq!(fs::read(path).unwrap(), b"later work");
        }
    }

    #[test]
    fn dropping_failed_command_preserves_all_previous_records() {
        let repository = Repository::new();
        let first = repository.0.join("first.json");
        let second = repository.0.join("second.json");
        fs::write(&first, b"old").unwrap();
        {
            let command = CommandTransaction::open(&repository.0).unwrap();
            command.stage(&first, b"new".to_vec()).unwrap();
            command.stage(&second, b"created".to_vec()).unwrap();
            assert_eq!(command.read(&first).unwrap(), b"new");
            assert_eq!(command.list(&repository.0).unwrap().len(), 2);
            assert_eq!(fs::read(&first).unwrap(), b"old");
        }
        let command = CommandTransaction::open(&repository.0).unwrap();
        assert_eq!(command.read(&first).unwrap(), b"old");
        assert!(!second.exists());
    }

    #[test]
    fn recovery_completes_a_partially_published_committed_command() {
        let repository = Repository::new();
        let first = repository.0.join("first.json");
        let second = repository.0.join("second.json");
        {
            let command = CommandTransaction::open(&repository.0).unwrap();
            let transaction = command.journal.unchecked_transaction().unwrap();
            transaction
                .execute(
                    "INSERT INTO pending VALUES ('first.json', ?1), ('second.json', ?2)",
                    rusqlite::params![b"new first", b"new second"],
                )
                .unwrap();
            transaction.commit().unwrap();
            // Model interruption after the first rename, before the second.
            publish_file(&first, b"new first").unwrap();
        }
        {
            let command = CommandTransaction::open(&repository.0).unwrap();
            assert_eq!(command.read(&first).unwrap(), b"new first");
            assert_eq!(command.read(&second).unwrap(), b"new second");
            assert_eq!(
                command
                    .journal
                    .query_row("SELECT count(*) FROM pending", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        let command = CommandTransaction::open(&repository.0).unwrap();
        assert_eq!(command.read(&second).unwrap(), b"new second");
    }

    #[test]
    fn recovery_rejects_paths_outside_control_directory() {
        let repository = Repository::new();
        {
            let command = CommandTransaction::open(&repository.0).unwrap();
            command
                .journal
                .execute(
                    "INSERT INTO pending VALUES ('../escape.json', ?1)",
                    [b"bad".as_slice()],
                )
                .unwrap();
        }
        assert!(CommandTransaction::open(&repository.0).is_err());
    }

    #[test]
    fn concurrent_commands_observe_committed_predecessor() {
        let repository = Repository::new();
        let path = repository.0.join("state.json");
        let first = CommandTransaction::open(&repository.0).unwrap();
        first.stage(&path, b"committed".to_vec()).unwrap();
        let root = repository.0.clone();
        let worker = std::thread::spawn(move || {
            let command = CommandTransaction::open(&root).unwrap();
            command.read(&root.join("state.json")).unwrap()
        });
        first.commit().unwrap();
        drop(first);
        assert_eq!(worker.join().unwrap(), b"committed");
    }
    #[test]
    #[ignore = "subprocess interruption helper"]
    fn checkout_interruption_child() {
        let root = PathBuf::from(std::env::var_os("RGIT_CHECKOUT_CRASH_ROOT").unwrap());
        let command = CommandTransaction::open(&root.join(".rgit")).unwrap();
        let transaction = command.journal.unchecked_transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO working(path, before_bytes, after_bytes) VALUES ('first.txt', ?1, ?2), ('second.txt', ?1, ?2)",
                rusqlite::params![b"old", b"new"],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO pending VALUES ('workspace.json', ?1)",
                [b"new pointer".as_slice()],
            )
            .unwrap();
        transaction.commit().unwrap();
        publish_file(&root.join("first.txt"), b"new").unwrap();
        std::process::exit(77);
    }

    #[test]
    fn checkout_recovers_after_process_exit_and_preserves_later_edits() {
        for edit_after_exit in [false, true] {
            let repository = Repository::new();
            let meta = repository.0.join(".rgit");
            fs::create_dir(&meta).unwrap();
            fs::write(meta.join("workspace.json"), b"old pointer").unwrap();
            fs::write(repository.0.join("first.txt"), b"old").unwrap();
            fs::write(repository.0.join("second.txt"), b"old").unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "transaction::tests::checkout_interruption_child",
                    "--ignored",
                ])
                .env("RGIT_CHECKOUT_CRASH_ROOT", &repository.0)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(77));
            if edit_after_exit {
                fs::write(repository.0.join("second.txt"), b"later edit").unwrap();
            }
            let recovered = CommandTransaction::open(&meta);
            if edit_after_exit {
                assert!(recovered.is_err());
                assert_eq!(
                    fs::read(repository.0.join("second.txt")).unwrap(),
                    b"later edit"
                );
                assert_eq!(
                    fs::read(meta.join("workspace.json")).unwrap(),
                    b"old pointer"
                );
            } else {
                let recovered = recovered.unwrap();
                assert_eq!(fs::read(repository.0.join("second.txt")).unwrap(), b"new");
                assert_eq!(
                    recovered.read(&meta.join("workspace.json")).unwrap(),
                    b"new pointer"
                );
            }
        }
    }
    #[cfg(unix)]
    #[test]
    #[ignore = "symlink interruption subprocess helper"]
    fn symlink_interruption_child() {
        let root = PathBuf::from(std::env::var_os("RGIT_SYMLINK_CRASH_ROOT").unwrap());
        let command = CommandTransaction::open(&root.join(".rgit")).unwrap();
        let transaction = command.journal.unchecked_transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO working VALUES ('link', ?1, ?2, 0, 0, 0, 1)",
                rusqlite::params![b"old", b"../outside"],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO pending VALUES ('workspace.json', ?1)",
                [b"new pointer".as_slice()],
            )
            .unwrap();
        transaction.commit().unwrap();
        publish_link(&root.join("link"), b"../outside").unwrap();
        std::process::exit(77);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_recovery_preserves_type_and_refuses_later_type_changes() {
        for replace_with_regular in [false, true] {
            let repo = Repository::new();
            let meta = repo.0.join(".rgit");
            fs::create_dir(&meta).unwrap();
            fs::write(meta.join("workspace.json"), b"old pointer").unwrap();
            fs::write(repo.0.join("link"), b"old").unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "transaction::tests::symlink_interruption_child",
                    "--ignored",
                ])
                .env("RGIT_SYMLINK_CRASH_ROOT", &repo.0)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(77));
            if replace_with_regular {
                fs::remove_file(repo.0.join("link")).unwrap();
                fs::write(repo.0.join("link"), b"../outside").unwrap();
            }
            let recovered = CommandTransaction::open(&meta);
            if replace_with_regular {
                assert!(recovered.is_err());
                assert_eq!(
                    fs::read(meta.join("workspace.json")).unwrap(),
                    b"old pointer"
                );
                assert!(!fs::symlink_metadata(repo.0.join("link"))
                    .unwrap()
                    .file_type()
                    .is_symlink());
            } else {
                recovered.unwrap();
                assert_eq!(
                    fs::read_link(repo.0.join("link")).unwrap(),
                    PathBuf::from("../outside")
                );
                assert_eq!(
                    fs::read(meta.join("workspace.json")).unwrap(),
                    b"new pointer"
                );
            }
        }
    }
}
