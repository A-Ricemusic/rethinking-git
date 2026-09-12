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
        for (key, update) in self.working.borrow().iter() {
            let current = read_working(&self.workspace_root()?, key)?;
            if current != update.before
                || !matches_mode(&self.workspace_root()?.join(key), update.before_executable)?
            {
                bail!("working file changed during command: {key}");
            }
        }
        let transaction = self.journal.unchecked_transaction()?;
        for (key, update) in self.working.borrow().iter() {
            transaction.execute(
                "INSERT INTO working VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    key,
                    update.before,
                    update.after,
                    update.before_executable,
                    update.after_executable
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
                .prepare("SELECT path, before_bytes, after_bytes, before_executable, after_executable FROM working ORDER BY path")?;
            let rows = query.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    WorkingUpdate {
                        before: row.get(1)?,
                        after: row.get(2)?,
                        before_executable: row.get(3)?,
                        after_executable: row.get(4)?,
                    },
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (key, update) in &working {
            let current = read_working(&self.workspace_root()?, key)?;
            if (current != update.before
                || !matches_mode(&self.workspace_root()?.join(key), update.before_executable)?)
                && (current != update.after
                    || !matches_mode(&self.workspace_root()?.join(key), update.after_executable)?)
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
        for (key, update) in &working {
            publish_working(&self.workspace_root()?, key, update)?;
        }
        for (key, bytes) in &entries {
            publish_file(&self.root.join(key), bytes)?;
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
            connection.execute_batch("PRAGMA application_id=1380402004; PRAGMA user_version=3;")?;
        }
        (1380402004, 1 | 2) => {
            connection.execute_batch("PRAGMA user_version=3;")?;
        }
        (1380402004, 3) => {}
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
    publish_file_with_mode(path, bytes, None)
}

fn publish_file_with_mode(path: &Path, bytes: &[u8], executable: Option<bool>) -> Result<()> {
    let parent = path.parent().context("file has no parent")?;
    ensure_directory(parent)?;
    let permissions = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Some(metadata.permissions())
        }
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
}

impl CommandTransaction {
    fn workspace_root(&self) -> Result<PathBuf> {
        Ok(self
            .root
            .parent()
            .context("control directory has no parent")?
            .to_path_buf())
    }

    pub(crate) fn stage_working(
        &self,
        key: &str,
        before: Option<Vec<u8>>,
        after: Option<Vec<u8>>,
        before_executable: bool,
        after_executable: bool,
    ) -> Result<()> {
        working_path(&self.workspace_root()?, key)?;
        self.working.borrow_mut().insert(
            key.to_string(),
            WorkingUpdate {
                before,
                after,
                before_executable: Some(before_executable),
                after_executable: Some(after_executable),
            },
        );
        Ok(())
    }
}

pub(crate) fn working_path(root: &Path, key: &str) -> Result<PathBuf> {
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
                || matches!(
                    part.to_ascii_lowercase().as_str(),
                    ".git" | ".rgit" | "target" | "node_modules"
                )
        })
    {
        bail!("unsafe snapshot path");
    }
    let mut path = root.to_path_buf();
    for (index, part) in relative.components().enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("working path traverses a symlink: {key}")
            }
            Ok(metadata) if index + 1 < relative.components().count() && !metadata.is_dir() => {
                bail!("working path has a non-directory parent: {key}")
            }
            Ok(metadata) if index + 1 == relative.components().count() && !metadata.is_file() => {
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
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn publish_working(root: &Path, key: &str, update: &WorkingUpdate) -> Result<()> {
    let path = working_path(root, key)?;
    let current = read_working(root, key)?;
    if current == update.after && matches_mode(&path, update.after_executable)? {
        return Ok(());
    }
    if (current != update.before || !matches_mode(&path, update.before_executable)?)
        && current != update.after
    {
        bail!("working file changed during recovery: {key}");
    }
    if let Some(bytes) = &update.after {
        let parent = path.parent().context("working file has no parent")?;
        let mut directory = root.to_path_buf();
        for component in parent.strip_prefix(root)?.components() {
            directory.push(component);
            if !directory.exists() {
                fs::create_dir(&directory)?;
                #[cfg(unix)]
                fs::File::open(directory.parent().unwrap())?.sync_all()?;
            }
            ensure_directory(&directory)?;
        }
        publish_file_with_mode(&path, bytes, update.after_executable)?;
    } else {
        fs::remove_file(&path)?;
        #[cfg(unix)]
        fs::File::open(path.parent().unwrap())?.sync_all()?;
    }
    Ok(())
}

pub(crate) fn is_executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

fn matches_mode(path: &Path, expected: Option<bool>) -> Result<bool> {
    #[cfg(unix)]
    {
        if !path.exists() {
            return Ok(true);
        }
        Ok(expected
            .is_none_or(|expected| is_executable(path).is_ok_and(|actual| actual == expected)))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, expected);
        Ok(true)
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
}
