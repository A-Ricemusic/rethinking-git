//! File/directory publication uses a validated whole-set plan: delete tracked
//! leaves first, remove only empty directories, then publish replacement leaves.
use super::*;

pub(super) struct ShapePlan(BTreeMap<String, (bool, bool)>);

pub(super) struct Current {
    pub(super) bytes: Option<Vec<u8>>,
    pub(super) flags: WorkingFlags,
}

impl Current {
    pub(super) fn matches(
        &self,
        bytes: &Option<Vec<u8>>,
        executable: Option<bool>,
        symlink: Option<bool>,
    ) -> bool {
        if &self.bytes != bytes {
            return false;
        }
        if self.bytes.is_none() {
            return true;
        }
        #[cfg(unix)]
        {
            executable.is_none_or(|v| v == self.flags.executable)
                && symlink.is_none_or(|v| v == self.flags.symlink)
        }
        #[cfg(not(unix))]
        {
            let _ = executable;
            // A logical link may be a regular target-text fallback on Windows,
            // but a regular-file expectation must not accept a native link.
            symlink != Some(false) || !self.flags.symlink
        }
    }
    fn absent() -> Self {
        Self {
            bytes: None,
            flags: WorkingFlags::default(),
        }
    }
}

impl ShapePlan {
    pub(super) fn new(paths: impl IntoIterator<Item = (String, bool, bool)>) -> Result<Self> {
        let plan = Self(paths.into_iter().map(|(p, b, a)| (p, (b, a))).collect());
        crate::checkout_paths::validate(plan.0.keys().map(String::as_str))?;
        for after in [false, true] {
            let files: BTreeSet<_> = plan
                .0
                .iter()
                .filter_map(|(key, (b, a))| {
                    if if after { *a } else { *b } {
                        Some(key.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            for key in &files {
                let mut prefix = String::new();
                let parts: Vec<_> = key.split('/').collect();
                for part in &parts[..parts.len() - 1] {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    if files.contains(prefix.as_str()) {
                        bail!("working plan contains a file/directory collision");
                    }
                }
            }
        }
        Ok(plan)
    }

    pub(super) fn from_updates(updates: &BTreeMap<String, WorkingUpdate>) -> Result<Self> {
        Self::new(
            updates
                .iter()
                .map(|(key, u)| (key.clone(), u.before.is_some(), u.after.is_some())),
        )
    }

    pub(super) fn validate_aliases(&self, root: &Path) -> Result<()> {
        let blockers = self
            .0
            .iter()
            .filter_map(|(key, (b, a))| if *b || *a { Some(key.as_str()) } else { None })
            .collect();
        crate::checkout_paths::validate_existing(root, self.0.keys().map(String::as_str), &blockers)
    }

    fn directory_can_be_replaced(&self, root: &Path, path: &Path) -> Result<()> {
        let mut pending = vec![path.to_path_buf()];
        while let Some(dir) = pending.pop() {
            ensure_directory(&dir)?;
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                let key = relative_key(root, &path)?;
                validate_working_key(&key)?;
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    pending.push(path);
                } else if self.0.get(&key) != Some(&(true, false)) {
                    bail!("checkout would replace a directory containing untracked files");
                }
            }
        }
        Ok(())
    }

    pub(super) fn current(&self, root: &Path, key: &str) -> Result<Current> {
        validate_working_key(key)?;
        let (before, after) = self.0.get(key).context("working path missing from plan")?;
        let mut path = root.to_path_buf();
        let mut prefix = String::new();
        let parts: Vec<_> = key.split('/').collect();
        for (index, part) in parts.iter().enumerate() {
            path.push(part);
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Current::absent()),
                Err(e) => return Err(e.into()),
            };
            let directory = metadata.is_dir() && !metadata.file_type().is_symlink();
            if index + 1 < parts.len() {
                if directory {
                    continue;
                }
                if let Some((parent_before, parent_after)) = self.0.get(&prefix) {
                    // A tracked leaf legitimately blocks this path in one side of
                    // a shape transition. Its bytes/type are checked separately.
                    if (*parent_before && !before) || (*parent_after && !after) {
                        return Ok(Current::absent());
                    }
                }
                bail!("working path has an unsafe parent");
            }
            if directory {
                if *after {
                    self.directory_can_be_replaced(root, &path)?;
                }
                return Ok(Current::absent());
            }
            return Ok(Current {
                bytes: read_working(root, key)?,
                flags: working_flags(&path)?,
            });
        }
        bail!("empty working path")
    }
}

fn relative_key(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)?
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .context("unrepresentable working path")
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

pub(super) fn validate_updates(updates: &BTreeMap<String, WorkingUpdate>) -> Result<()> {
    for update in updates.values() {
        for (bytes, executable, symlink) in [
            (
                &update.before,
                update.before_executable,
                update.before_symlink,
            ),
            (&update.after, update.after_executable, update.after_symlink),
        ] {
            if symlink == Some(true) {
                validate_link(bytes.as_deref().context("symlink journal target missing")?)?;
                if executable == Some(true) {
                    bail!("symlink journal mode is invalid");
                }
            }
        }
    }
    Ok(())
}

fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path.parent().context("path has no parent")?)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn remove_empty_tree(
    root: &Path,
    path: &Path,
    hook: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    // Iterative postorder avoids following links or overflowing on deep trees.
    let mut stack = vec![(path.to_path_buf(), false)];
    while let Some((path, leaving)) = stack.pop() {
        validate_working_key(&relative_key(root, &path)?)?;
        ensure_directory(&path)?;
        if leaving {
            fs::remove_dir(&path)
                .context("directory changed during checkout; refusing to remove its contents")?;
            sync_parent(&path)?;
            hook()?;
        } else {
            stack.push((path.clone(), true));
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    bail!("directory changed during checkout; preserving untracked entry");
                }
                stack.push((entry.path(), false));
            }
        }
    }
    Ok(())
}

pub(super) fn publish(
    root: &Path,
    updates: &BTreeMap<String, WorkingUpdate>,
    plan: &ShapePlan,
    hook: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    let mut keys: Vec<_> = updates.keys().collect();
    keys.sort_by_key(|key| (std::cmp::Reverse(key.split('/').count()), *key));
    for key in keys
        .iter()
        .copied()
        .filter(|key| updates[*key].after.is_none())
    {
        let update = &updates[key];
        let current = plan.current(root, key)?;
        if current.matches(&update.after, update.after_executable, update.after_symlink) {
            continue;
        }
        if !current.matches(
            &update.before,
            update.before_executable,
            update.before_symlink,
        ) {
            bail!("working file changed during recovery: {key}");
        }
        let path = working_path(root, key)?;
        fs::remove_file(&path)?;
        sync_parent(&path)?;
        hook()?;
    }
    for (key, update) in updates.iter().filter(|(_, u)| u.after.is_some()) {
        let current = plan.current(root, key)?;
        if current.matches(&update.after, update.after_executable, update.after_symlink) {
            continue;
        }
        if !current.matches(
            &update.before,
            update.before_executable,
            update.before_symlink,
        ) {
            bail!("working file changed during recovery: {key}");
        }
        let path = root.join(key);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            remove_empty_tree(root, &path, hook)?;
        }
        // All blocking tracked leaves have already been deleted. No parent link
        // can be followed by ordinary publication after this strict recheck.
        let path = working_path(root, key)?;
        let parent = path.parent().context("working file has no parent")?;
        let mut directory = root.to_path_buf();
        for component in parent.strip_prefix(root)?.components() {
            directory.push(component);
            match fs::create_dir(&directory) {
                Ok(()) => {
                    sync_parent(&directory)?;
                    hook()?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    ensure_directory(&directory)?
                }
                Err(e) => return Err(e.into()),
            }
        }
        let bytes = update.after.as_deref().context("working target missing")?;
        if cfg!(unix) && update.after_symlink == Some(true) {
            publish_link(&path, bytes)?;
        } else {
            publish_file_with_mode(&path, bytes, update.after_executable, true)?;
        }
        hook()?;
    }
    Ok(())
}

#[cfg(all(test, not(unix)))]
mod tests {
    use super::*;
    #[test]
    fn native_links_cannot_satisfy_regular_file_expectations_but_link_fallbacks_remain_valid() {
        let mut current = Current {
            bytes: Some(b"target".to_vec()),
            flags: WorkingFlags {
                executable: false,
                symlink: true,
            },
        };
        assert!(!current.matches(&Some(b"target".to_vec()), Some(false), Some(false)));
        current.flags.symlink = false;
        assert!(current.matches(&Some(b"target".to_vec()), Some(false), Some(true)));
    }
}
